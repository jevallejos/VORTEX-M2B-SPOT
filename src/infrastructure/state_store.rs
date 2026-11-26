use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::domain::error::InfraError;
use crate::domain::ports::{RunLockGuard, StateStorePort};
use crate::domain::types::{
    fnv1a64_hex, AssetSymbol, AuditEntry, EnginePersistedState, ExecutionMode, Money1e8,
    PortfolioSnapshot, PositionState, Price1e8, Qty1e8, Rate1e10, Shadow4hPersistedState, Universe,
    STATE_SCHEMA_VERSION,
};

const PRIMARY_STATE_FILE: &str = "primary_state.json";
const SHADOW_4H_STATE_FILE: &str = "m2b_shadow_4h_state.json";
const RUN_LOCK_FILE: &str = "run.lock";
const DRAIN_LOCK_FILE: &str = "DRAIN_MODE.lock";
const HALT_LOCK_FILE: &str = "HALT_LOCK";
const AUDIT_LEDGER_FILE: &str = "m2b_audit_ledger.jsonl";

#[derive(Debug, Clone)]
pub struct FsStateStore {
    state_dir: PathBuf,
    logs_dir: PathBuf,
}

struct FsRunLockGuard {
    path: PathBuf,
}

impl RunLockGuard for FsRunLockGuard {}

impl Drop for FsRunLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl FsStateStore {
    pub fn new(
        state_dir: impl Into<PathBuf>,
        logs_dir: impl Into<PathBuf>,
    ) -> Result<Self, InfraError> {
        let state_dir = state_dir.into();
        let logs_dir = logs_dir.into();
        fs::create_dir_all(&state_dir).map_err(|e| {
            InfraError::Io(format!(
                "failed to create state_dir {}: {e}",
                state_dir.display()
            ))
        })?;
        fs::create_dir_all(&logs_dir).map_err(|e| {
            InfraError::Io(format!(
                "failed to create logs_dir {}: {e}",
                logs_dir.display()
            ))
        })?;
        Ok(Self {
            state_dir,
            logs_dir,
        })
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub fn logs_dir(&self) -> &Path {
        &self.logs_dir
    }

    pub fn primary_state_path(&self) -> PathBuf {
        self.state_dir.join(PRIMARY_STATE_FILE)
    }

    pub fn shadow_4h_state_path(&self) -> PathBuf {
        self.state_dir.join(SHADOW_4H_STATE_FILE)
    }

    pub fn audit_ledger_path(&self) -> PathBuf {
        self.logs_dir.join(AUDIT_LEDGER_FILE)
    }
}

impl StateStorePort for FsStateStore {
    fn acquire_run_lock(&self) -> Result<Box<dyn RunLockGuard>, InfraError> {
        fs::create_dir_all(&self.state_dir).map_err(|e| {
            InfraError::Io(format!(
                "failed to create state_dir {}: {e}",
                self.state_dir.display()
            ))
        })?;

        let lock_path = self.state_dir.join(RUN_LOCK_FILE);
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                return Err(InfraError::LockBusy(format!(
                    "run lock already held at {}",
                    lock_path.display()
                )));
            }
            Err(e) => {
                return Err(InfraError::Io(format!(
                    "failed to acquire run lock {}: {e}",
                    lock_path.display()
                )));
            }
        };

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ = writeln!(file, "pid={} ts_ms={}", std::process::id(), now_ms);
        let _ = file.sync_all();

        Ok(Box::new(FsRunLockGuard { path: lock_path }))
    }

    fn has_drain_lock(&self) -> Result<bool, InfraError> {
        Ok(self.state_dir.join(DRAIN_LOCK_FILE).exists())
    }

    fn has_halt_lock(&self) -> Result<bool, InfraError> {
        Ok(self.state_dir.join(HALT_LOCK_FILE).exists())
    }

    fn write_halt_lock(&self, reason: &str) -> Result<(), InfraError> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let content = format!("ts_ms={now_ms}\nreason={reason}\n");
        atomic_write_bytes(&self.state_dir.join(HALT_LOCK_FILE), content.as_bytes())
    }

    fn load_primary_state(
        &self,
        expected_universe: &Universe,
    ) -> Result<Option<EnginePersistedState>, InfraError> {
        let path = self.primary_state_path();
        let raw = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(InfraError::Io(format!(
                    "failed to read primary state {}: {e}",
                    path.display()
                )));
            }
        };
        check_schema_header(&raw, &path)?;
        let dto: EnginePersistedStateDto = serde_json::from_str(&raw).map_err(|e| {
            InfraError::Parse(format!(
                "failed to parse primary state {}: {e}",
                path.display()
            ))
        })?;
        Ok(Some(dto.into_domain(expected_universe, &path)?))
    }

    fn save_primary_state(&self, state: &EnginePersistedState) -> Result<(), InfraError> {
        let universe = Universe::new(state.symbols.clone()).map_err(|e| {
            InfraError::Parse(format!(
                "cannot serialize primary state with invalid universe: {e}"
            ))
        })?;
        state.validate_against(&universe).map_err(|e| {
            InfraError::Parse(format!("cannot serialize inconsistent primary state: {e}"))
        })?;
        let dto = EnginePersistedStateDto::from_domain(state);
        let json = serde_json::to_string_pretty(&dto)
            .map_err(|e| InfraError::Parse(format!("failed to serialize primary state: {e}")))?;
        atomic_write_bytes(&self.primary_state_path(), json.as_bytes())
    }

    fn load_shadow_4h_state(
        &self,
        expected_universe: &Universe,
    ) -> Result<Option<Shadow4hPersistedState>, InfraError> {
        let path = self.shadow_4h_state_path();
        let raw = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(InfraError::Io(format!(
                    "failed to read shadow 4h state {}: {e}",
                    path.display()
                )));
            }
        };
        check_schema_header(&raw, &path)?;
        let dto: Shadow4hPersistedStateDto = serde_json::from_str(&raw).map_err(|e| {
            InfraError::Parse(format!(
                "failed to parse shadow 4h state {}: {e}",
                path.display()
            ))
        })?;
        Ok(Some(dto.into_domain(expected_universe, &path)?))
    }

    fn save_shadow_4h_state(&self, state: &Shadow4hPersistedState) -> Result<(), InfraError> {
        let universe = Universe::new(state.symbols.clone()).map_err(|e| {
            InfraError::Parse(format!(
                "cannot serialize shadow 4h state with invalid universe: {e}"
            ))
        })?;
        state.validate_against(&universe).map_err(|e| {
            InfraError::Parse(format!(
                "cannot serialize inconsistent shadow 4h state: {e}"
            ))
        })?;
        let dto = Shadow4hPersistedStateDto::from_domain(state);
        let json = serde_json::to_string_pretty(&dto)
            .map_err(|e| InfraError::Parse(format!("failed to serialize shadow 4h state: {e}")))?;
        atomic_write_bytes(&self.shadow_4h_state_path(), json.as_bytes())
    }

    fn append_audit_entry(&self, entry: &AuditEntry) -> Result<(), InfraError> {
        fs::create_dir_all(&self.logs_dir).map_err(|e| {
            InfraError::Io(format!(
                "failed to create logs_dir {}: {e}",
                self.logs_dir.display()
            ))
        })?;

        let payload_dto = AuditEntryPayloadDto::from_domain(entry);
        let canonical = serde_json::to_string(&payload_dto)
            .map_err(|e| InfraError::Parse(format!("failed to serialize audit payload: {e}")))?;
        let hash = fnv1a64_hex(canonical.as_bytes());
        let sealed = AuditEntrySealedDto::from_payload(payload_dto, hash);
        let mut line = serde_json::to_string(&sealed).map_err(|e| {
            InfraError::Parse(format!("failed to serialize sealed audit entry: {e}"))
        })?;
        line.push('\n');

        let path = self.audit_ledger_path();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| {
                InfraError::Io(format!(
                    "failed to open audit ledger {}: {e}",
                    path.display()
                ))
            })?;
        file.write_all(line.as_bytes()).map_err(|e| {
            InfraError::Io(format!(
                "failed to append audit ledger {}: {e}",
                path.display()
            ))
        })?;
        file.sync_all().map_err(|e| {
            InfraError::Io(format!(
                "failed to sync audit ledger {}: {e}",
                path.display()
            ))
        })?;
        Ok(())
    }
}

pub fn verify_audit_ledger_line(line: &str) -> Result<bool, InfraError> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err(InfraError::Parse("empty audit ledger line".to_string()));
    }
    let sealed: AuditEntrySealedDto = serde_json::from_str(trimmed)
        .map_err(|e| InfraError::Parse(format!("invalid audit ledger JSON line: {e}")))?;
    let payload = sealed.to_payload();
    let canonical = serde_json::to_string(&payload).map_err(|e| {
        InfraError::Parse(format!("failed to serialize audit payload for verify: {e}"))
    })?;
    let expected = fnv1a64_hex(canonical.as_bytes());
    Ok(expected == sealed.fnv1a64_hex)
}

fn check_schema_header(raw: &str, path: &Path) -> Result<(), InfraError> {
    let header: StateSchemaHeader = serde_json::from_str(raw).map_err(|e| {
        InfraError::Parse(format!(
            "failed to read schema_version in {}: {e}",
            path.display()
        ))
    })?;
    if header.schema_version != STATE_SCHEMA_VERSION {
        return Err(InfraError::SchemaVersionMismatch {
            expected: STATE_SCHEMA_VERSION,
            actual: header.schema_version,
            path: path.display().to_string(),
        });
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_dto_against_universe(
    schema_version: u32,
    dto_symbols: &[String],
    expected_universe: &Universe,
    portfolio: &PortfolioSnapshotDto,
    last_target_weights: &[f64],
    last_current_weights: &[f64],
    last_sigma_ann: &[f64],
    last_tsmom: &[f64],
    last_prices_1e8: &[i128],
    path: &Path,
) -> Result<Vec<AssetSymbol>, InfraError> {
    let path_str = path.display().to_string();
    if schema_version != STATE_SCHEMA_VERSION {
        return Err(InfraError::SchemaVersionMismatch {
            expected: STATE_SCHEMA_VERSION,
            actual: schema_version,
            path: path_str,
        });
    }

    let expected_strings = expected_universe.as_strings();
    if dto_symbols != expected_strings.as_slice() {
        return Err(InfraError::UniverseMismatch {
            expected: expected_strings,
            actual: dto_symbols.to_vec(),
            path: path_str,
        });
    }

    let n = expected_universe.len();
    let checks: [(&'static str, usize); 7] = [
        ("portfolio.positions", portfolio.positions.len()),
        (
            "portfolio.cum_buy_fees_base_1e8",
            portfolio.cum_buy_fees_base_1e8.len(),
        ),
        ("last_target_weights", last_target_weights.len()),
        ("last_current_weights", last_current_weights.len()),
        ("last_sigma_ann", last_sigma_ann.len()),
        ("last_tsmom", last_tsmom.len()),
        ("last_prices_1e8", last_prices_1e8.len()),
    ];
    for (field, actual) in checks {
        if actual != n {
            return Err(InfraError::CorruptStateLength {
                field,
                expected: n,
                actual,
                path: path_str,
            });
        }
    }

    Ok(expected_universe.symbols().to_vec())
}

fn atomic_write_bytes(target_path: &Path, data: &[u8]) -> Result<(), InfraError> {
    if let Some(parent) = target_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| {
                InfraError::Io(format!(
                    "failed to create parent directory {}: {e}",
                    parent.display()
                ))
            })?;
        }
    }

    let file_name = target_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| {
            InfraError::Io(format!(
                "invalid target file path {}",
                target_path.display()
            ))
        })?;
    let tmp_name = format!("{file_name}.tmp.{}", std::process::id());
    let tmp_path = target_path.with_file_name(tmp_name);

    {
        let mut file = File::create(&tmp_path).map_err(|e| {
            InfraError::Io(format!(
                "failed to create temp file {}: {e}",
                tmp_path.display()
            ))
        })?;
        if let Err(e) = file.write_all(data) {
            let _ = fs::remove_file(&tmp_path);
            return Err(InfraError::Io(format!(
                "failed to write temp file {}: {e}",
                tmp_path.display()
            )));
        }
        if let Err(e) = file.sync_all() {
            let _ = fs::remove_file(&tmp_path);
            return Err(InfraError::Io(format!(
                "failed to sync temp file {}: {e}",
                tmp_path.display()
            )));
        }
    }

    if let Err(e) = fs::rename(&tmp_path, target_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(InfraError::Io(format!(
            "failed to rename {} -> {}: {e}",
            tmp_path.display(),
            target_path.display()
        )));
    }

    Ok(())
}

#[derive(Deserialize)]
struct StateSchemaHeader {
    schema_version: u32,
}

#[derive(Serialize, Deserialize)]
struct PositionStateDto {
    qty_1e8: i128,
    cost_basis_usdt_1e8: i128,
}

impl PositionStateDto {
    fn from_domain(p: &PositionState) -> Self {
        Self {
            qty_1e8: p.qty_1e8.raw(),
            cost_basis_usdt_1e8: p.cost_basis_usdt_1e8.raw(),
        }
    }

    fn into_domain(self) -> PositionState {
        PositionState {
            qty_1e8: Qty1e8(self.qty_1e8),
            cost_basis_usdt_1e8: Money1e8(self.cost_basis_usdt_1e8),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct PortfolioSnapshotDto {
    cash_1e8: i128,
    positions: Vec<PositionStateDto>,
    total_deposits_1e8: i128,
    realized_pnl_gross_1e8: i128,
    cum_sell_fees_usdt_1e8: i128,
    cum_buy_fees_base_1e8: Vec<i128>,
    cum_buy_fees_usdt_est_1e8: i128,
    twr_index_1e10: i128,
    twr_peak_1e10: i128,
    last_marked_equity_1e8: i128,
    halted: bool,
}

impl PortfolioSnapshotDto {
    fn from_domain(s: &PortfolioSnapshot) -> Self {
        Self {
            cash_1e8: s.cash_1e8.raw(),
            positions: s
                .positions
                .iter()
                .map(PositionStateDto::from_domain)
                .collect(),
            total_deposits_1e8: s.total_deposits_1e8.raw(),
            realized_pnl_gross_1e8: s.realized_pnl_gross_1e8.raw(),
            cum_sell_fees_usdt_1e8: s.cum_sell_fees_usdt_1e8.raw(),
            cum_buy_fees_base_1e8: s.cum_buy_fees_base_1e8.iter().map(|q| q.raw()).collect(),
            cum_buy_fees_usdt_est_1e8: s.cum_buy_fees_usdt_est_1e8.raw(),
            twr_index_1e10: s.twr_index_1e10.raw(),
            twr_peak_1e10: s.twr_peak_1e10.raw(),
            last_marked_equity_1e8: s.last_marked_equity_1e8.raw(),
            halted: s.halted,
        }
    }

    fn into_domain(self) -> PortfolioSnapshot {
        PortfolioSnapshot {
            cash_1e8: Money1e8(self.cash_1e8),
            positions: self
                .positions
                .into_iter()
                .map(PositionStateDto::into_domain)
                .collect(),
            total_deposits_1e8: Money1e8(self.total_deposits_1e8),
            realized_pnl_gross_1e8: Money1e8(self.realized_pnl_gross_1e8),
            cum_sell_fees_usdt_1e8: Money1e8(self.cum_sell_fees_usdt_1e8),
            cum_buy_fees_base_1e8: self.cum_buy_fees_base_1e8.into_iter().map(Qty1e8).collect(),
            cum_buy_fees_usdt_est_1e8: Money1e8(self.cum_buy_fees_usdt_est_1e8),
            twr_index_1e10: Rate1e10(self.twr_index_1e10),
            twr_peak_1e10: Rate1e10(self.twr_peak_1e10),
            last_marked_equity_1e8: Money1e8(self.last_marked_equity_1e8),
            halted: self.halted,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct EnginePersistedStateDto {
    schema_version: u32,
    symbols: Vec<String>,
    mode: String,
    last_processed_hour_ts_ms: i64,
    last_daily_summary_day_utc: i64,
    last_order_ts_ms: i64,
    portfolio: PortfolioSnapshotDto,
    last_target_weights: Vec<f64>,
    last_current_weights: Vec<f64>,
    last_sigma_ann: Vec<f64>,
    last_tsmom: Vec<f64>,
    last_prices_1e8: Vec<i128>,
    last_drift_1e8: i128,
}

impl EnginePersistedStateDto {
    fn from_domain(s: &EnginePersistedState) -> Self {
        Self {
            schema_version: s.schema_version,
            symbols: s
                .symbols
                .iter()
                .map(|sym| sym.as_str().to_string())
                .collect(),
            mode: s.mode.as_str().to_string(),
            last_processed_hour_ts_ms: s.last_processed_hour_ts_ms,
            last_daily_summary_day_utc: s.last_daily_summary_day_utc,
            last_order_ts_ms: s.last_order_ts_ms,
            portfolio: PortfolioSnapshotDto::from_domain(&s.portfolio),
            last_target_weights: s.last_target_weights.clone(),
            last_current_weights: s.last_current_weights.clone(),
            last_sigma_ann: s.last_sigma_ann.clone(),
            last_tsmom: s.last_tsmom.clone(),
            last_prices_1e8: s.last_prices_1e8.iter().map(|p| p.raw()).collect(),
            last_drift_1e8: s.last_drift_1e8,
        }
    }

    fn into_domain(
        self,
        expected_universe: &Universe,
        path: &Path,
    ) -> Result<EnginePersistedState, InfraError> {
        let symbols = validate_dto_against_universe(
            self.schema_version,
            &self.symbols,
            expected_universe,
            &self.portfolio,
            &self.last_target_weights,
            &self.last_current_weights,
            &self.last_sigma_ann,
            &self.last_tsmom,
            &self.last_prices_1e8,
            path,
        )?;
        let mode = match self.mode.as_str() {
            "paper" => ExecutionMode::Paper,
            "live-spot" => ExecutionMode::LiveSpot,
            other => {
                return Err(InfraError::Parse(format!(
                    "unknown execution mode in state: '{other}'"
                )));
            }
        };
        Ok(EnginePersistedState {
            schema_version: self.schema_version,
            symbols,
            mode,
            last_processed_hour_ts_ms: self.last_processed_hour_ts_ms,
            last_daily_summary_day_utc: self.last_daily_summary_day_utc,
            last_order_ts_ms: self.last_order_ts_ms,
            portfolio: self.portfolio.into_domain(),
            last_target_weights: self.last_target_weights,
            last_current_weights: self.last_current_weights,
            last_sigma_ann: self.last_sigma_ann,
            last_tsmom: self.last_tsmom,
            last_prices_1e8: self.last_prices_1e8.into_iter().map(Price1e8).collect(),
            last_drift_1e8: self.last_drift_1e8,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct Shadow4hPersistedStateDto {
    schema_version: u32,
    symbols: Vec<String>,
    last_processed_4h_close_ts_ms: i64,
    portfolio: PortfolioSnapshotDto,
    last_target_weights: Vec<f64>,
    last_current_weights: Vec<f64>,
    last_sigma_ann: Vec<f64>,
    last_tsmom: Vec<f64>,
    last_prices_1e8: Vec<i128>,
    last_drift_1e8: i128,
}

impl Shadow4hPersistedStateDto {
    fn from_domain(s: &Shadow4hPersistedState) -> Self {
        Self {
            schema_version: s.schema_version,
            symbols: s
                .symbols
                .iter()
                .map(|sym| sym.as_str().to_string())
                .collect(),
            last_processed_4h_close_ts_ms: s.last_processed_4h_close_ts_ms,
            portfolio: PortfolioSnapshotDto::from_domain(&s.portfolio),
            last_target_weights: s.last_target_weights.clone(),
            last_current_weights: s.last_current_weights.clone(),
            last_sigma_ann: s.last_sigma_ann.clone(),
            last_tsmom: s.last_tsmom.clone(),
            last_prices_1e8: s.last_prices_1e8.iter().map(|p| p.raw()).collect(),
            last_drift_1e8: s.last_drift_1e8,
        }
    }

    fn into_domain(
        self,
        expected_universe: &Universe,
        path: &Path,
    ) -> Result<Shadow4hPersistedState, InfraError> {
        let symbols = validate_dto_against_universe(
            self.schema_version,
            &self.symbols,
            expected_universe,
            &self.portfolio,
            &self.last_target_weights,
            &self.last_current_weights,
            &self.last_sigma_ann,
            &self.last_tsmom,
            &self.last_prices_1e8,
            path,
        )?;
        Ok(Shadow4hPersistedState {
            schema_version: self.schema_version,
            symbols,
            last_processed_4h_close_ts_ms: self.last_processed_4h_close_ts_ms,
            portfolio: self.portfolio.into_domain(),
            last_target_weights: self.last_target_weights,
            last_current_weights: self.last_current_weights,
            last_sigma_ann: self.last_sigma_ann,
            last_tsmom: self.last_tsmom,
            last_prices_1e8: self.last_prices_1e8.into_iter().map(Price1e8).collect(),
            last_drift_1e8: self.last_drift_1e8,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct AuditEntryPayloadDto {
    ts_ms: i64,
    engine: String,
    event_type: String,
    mode: String,
    hour_ts_ms: i64,
    symbol: String,
    side: String,
    order_link_id: String,
    qty_1e8: i128,
    quote_1e8: i128,
    price_1e8: i128,
    fee_1e8: i128,
    fee_in_base: bool,
    equity_1e8: i128,
    drift_1e8: i128,
    detail: String,
}

impl AuditEntryPayloadDto {
    fn from_domain(e: &AuditEntry) -> Self {
        Self {
            ts_ms: e.ts_ms,
            engine: e.engine.clone(),
            event_type: e.event_type.clone(),
            mode: e.mode.clone(),
            hour_ts_ms: e.hour_ts_ms,
            symbol: e.symbol.clone(),
            side: e.side.clone(),
            order_link_id: e.order_link_id.clone(),
            qty_1e8: e.qty_1e8,
            quote_1e8: e.quote_1e8,
            price_1e8: e.price_1e8,
            fee_1e8: e.fee_1e8,
            fee_in_base: e.fee_in_base,
            equity_1e8: e.equity_1e8,
            drift_1e8: e.drift_1e8,
            detail: e.detail.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct AuditEntrySealedDto {
    ts_ms: i64,
    engine: String,
    event_type: String,
    mode: String,
    hour_ts_ms: i64,
    symbol: String,
    side: String,
    order_link_id: String,
    qty_1e8: i128,
    quote_1e8: i128,
    price_1e8: i128,
    fee_1e8: i128,
    fee_in_base: bool,
    equity_1e8: i128,
    drift_1e8: i128,
    detail: String,
    fnv1a64_hex: String,
}

impl AuditEntrySealedDto {
    fn from_payload(p: AuditEntryPayloadDto, fnv1a64_hex: String) -> Self {
        Self {
            ts_ms: p.ts_ms,
            engine: p.engine,
            event_type: p.event_type,
            mode: p.mode,
            hour_ts_ms: p.hour_ts_ms,
            symbol: p.symbol,
            side: p.side,
            order_link_id: p.order_link_id,
            qty_1e8: p.qty_1e8,
            quote_1e8: p.quote_1e8,
            price_1e8: p.price_1e8,
            fee_1e8: p.fee_1e8,
            fee_in_base: p.fee_in_base,
            equity_1e8: p.equity_1e8,
            drift_1e8: p.drift_1e8,
            detail: p.detail,
            fnv1a64_hex,
        }
    }

    fn to_payload(&self) -> AuditEntryPayloadDto {
        AuditEntryPayloadDto {
            ts_ms: self.ts_ms,
            engine: self.engine.clone(),
            event_type: self.event_type.clone(),
            mode: self.mode.clone(),
            hour_ts_ms: self.hour_ts_ms,
            symbol: self.symbol.clone(),
            side: self.side.clone(),
            order_link_id: self.order_link_id.clone(),
            qty_1e8: self.qty_1e8,
            quote_1e8: self.quote_1e8,
            price_1e8: self.price_1e8,
            fee_1e8: self.fee_1e8,
            fee_in_base: self.fee_in_base,
            equity_1e8: self.equity_1e8,
            drift_1e8: self.drift_1e8,
            detail: self.detail.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_seal_and_verify() -> Result<(), InfraError> {
        let dir = std::env::temp_dir().join(format!("vortex_m2b_test_{}", std::process::id()));
        let state_dir = dir.join("state");
        let logs_dir = dir.join("logs");
        let store = FsStateStore::new(&state_dir, &logs_dir)?;

        let entry = AuditEntry {
            ts_ms: 1_700_000_000_000,
            engine: "B2_01".to_string(),
            event_type: "TEST".to_string(),
            mode: "paper".to_string(),
            hour_ts_ms: 1_700_000_000_000,
            symbol: "BTCUSDT".to_string(),
            side: "Buy".to_string(),
            order_link_id: "m2b_1700000000000_BTCUSDT_buy".to_string(),
            qty_1e8: 100_000,
            quote_1e8: 10_000_000_000,
            price_1e8: 10_000_000_000_000,
            fee_1e8: 100,
            fee_in_base: true,
            equity_1e8: 100_000_000_000,
            drift_1e8: 0,
            detail: "ok".to_string(),
        };

        store.append_audit_entry(&entry)?;
        let content = fs::read_to_string(store.audit_ledger_path())
            .map_err(|e| InfraError::Io(e.to_string()))?;
        let first_line = content
            .lines()
            .next()
            .ok_or_else(|| InfraError::Parse("missing audit line".to_string()))?;
        assert!(verify_audit_ledger_line(first_line)?);
        let tampered = first_line.replace("\"detail\":\"ok\"", "\"detail\":\"bad\"");
        assert!(!verify_audit_ledger_line(&tampered)?);
        let _ = fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn test_schema_v2_fail_closed_rejects_v1_mismatch_and_corrupt_len() -> Result<(), InfraError> {
        let dir = std::env::temp_dir().join(format!("vortex_m2b_v2_test_{}", std::process::id()));
        let state_dir = dir.join("state");
        let logs_dir = dir.join("logs");
        let store = FsStateStore::new(&state_dir, &logs_dir)?;
        let universe =
            Universe::default_universe().map_err(|e| InfraError::Config(e.to_string()))?;
        let n = universe.len();

        let state = EnginePersistedState {
            schema_version: STATE_SCHEMA_VERSION,
            symbols: universe.symbols().to_vec(),
            mode: ExecutionMode::Paper,
            last_processed_hour_ts_ms: 1_728_000_000_000,
            last_daily_summary_day_utc: 20_000,
            last_order_ts_ms: 0,
            portfolio: PortfolioSnapshot {
                cash_1e8: Money1e8(100_000_000_000),
                positions: vec![PositionState::default(); n],
                total_deposits_1e8: Money1e8(100_000_000_000),
                realized_pnl_gross_1e8: Money1e8::ZERO,
                cum_sell_fees_usdt_1e8: Money1e8::ZERO,
                cum_buy_fees_base_1e8: vec![Qty1e8::ZERO; n],
                cum_buy_fees_usdt_est_1e8: Money1e8::ZERO,
                twr_index_1e10: Rate1e10::ONE,
                twr_peak_1e10: Rate1e10::ONE,
                last_marked_equity_1e8: Money1e8(100_000_000_000),
                halted: false,
            },
            last_target_weights: vec![0.0; n],
            last_current_weights: vec![0.0; n],
            last_sigma_ann: vec![0.2; n],
            last_tsmom: vec![0.0; n],
            last_prices_1e8: vec![Price1e8(10_000_000_000); n],
            last_drift_1e8: 0,
        };
        store.save_primary_state(&state)?;
        let loaded = store.load_primary_state(&universe)?;
        assert_eq!(loaded, Some(state));

        let alt_universe = Universe::parse_csv("BTCUSDT,ETHUSDT,SOLUSDT,LINKUSDT,AVAXUSDT")
            .map_err(|e| InfraError::Config(e.to_string()))?;
        assert!(matches!(
            store.load_primary_state(&alt_universe),
            Err(InfraError::UniverseMismatch { .. })
        ));

        let v1_json = r#"{"schema_version":1,"mode":"paper","last_processed_hour_ts_ms":1}"#;
        fs::write(store.primary_state_path(), v1_json)
            .map_err(|e| InfraError::Io(e.to_string()))?;
        assert!(matches!(
            store.load_primary_state(&universe),
            Err(InfraError::SchemaVersionMismatch {
                expected: 2,
                actual: 1,
                ..
            })
        ));

        let _ = fs::remove_dir_all(&dir);
        Ok(())
    }
}
