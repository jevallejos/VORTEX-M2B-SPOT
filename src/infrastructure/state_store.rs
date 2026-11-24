use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::domain::error::InfraError;
use crate::domain::ports::{RunLockGuard, StateStorePort};
use crate::domain::types::{
    fnv1a64_hex, AuditEntry, EnginePersistedState, ExecutionMode, Money1e8, PortfolioSnapshot,
    PositionState, Price1e8, Qty1e8, Rate1e10, Shadow4hPersistedState, UNIVERSE_LEN,
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

    fn load_primary_state(&self) -> Result<Option<EnginePersistedState>, InfraError> {
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
        let dto: EnginePersistedStateDto = serde_json::from_str(&raw).map_err(|e| {
            InfraError::Parse(format!(
                "failed to parse primary state {}: {e}",
                path.display()
            ))
        })?;
        Ok(Some(dto.into_domain()?))
    }

    fn save_primary_state(&self, state: &EnginePersistedState) -> Result<(), InfraError> {
        let dto = EnginePersistedStateDto::from_domain(state);
        let json = serde_json::to_string_pretty(&dto)
            .map_err(|e| InfraError::Parse(format!("failed to serialize primary state: {e}")))?;
        atomic_write_bytes(&self.primary_state_path(), json.as_bytes())
    }

    fn load_shadow_4h_state(&self) -> Result<Option<Shadow4hPersistedState>, InfraError> {
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
        let dto: Shadow4hPersistedStateDto = serde_json::from_str(&raw).map_err(|e| {
            InfraError::Parse(format!(
                "failed to parse shadow 4h state {}: {e}",
                path.display()
            ))
        })?;
        Ok(Some(dto.into_domain()))
    }

    fn save_shadow_4h_state(&self, state: &Shadow4hPersistedState) -> Result<(), InfraError> {
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
    positions: [PositionStateDto; UNIVERSE_LEN],
    total_deposits_1e8: i128,
    realized_pnl_gross_1e8: i128,
    cum_sell_fees_usdt_1e8: i128,
    cum_buy_fees_base_1e8: [i128; UNIVERSE_LEN],
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
            positions: [
                PositionStateDto::from_domain(&s.positions[0]),
                PositionStateDto::from_domain(&s.positions[1]),
                PositionStateDto::from_domain(&s.positions[2]),
                PositionStateDto::from_domain(&s.positions[3]),
                PositionStateDto::from_domain(&s.positions[4]),
            ],
            total_deposits_1e8: s.total_deposits_1e8.raw(),
            realized_pnl_gross_1e8: s.realized_pnl_gross_1e8.raw(),
            cum_sell_fees_usdt_1e8: s.cum_sell_fees_usdt_1e8.raw(),
            cum_buy_fees_base_1e8: [
                s.cum_buy_fees_base_1e8[0].raw(),
                s.cum_buy_fees_base_1e8[1].raw(),
                s.cum_buy_fees_base_1e8[2].raw(),
                s.cum_buy_fees_base_1e8[3].raw(),
                s.cum_buy_fees_base_1e8[4].raw(),
            ],
            cum_buy_fees_usdt_est_1e8: s.cum_buy_fees_usdt_est_1e8.raw(),
            twr_index_1e10: s.twr_index_1e10.raw(),
            twr_peak_1e10: s.twr_peak_1e10.raw(),
            last_marked_equity_1e8: s.last_marked_equity_1e8.raw(),
            halted: s.halted,
        }
    }

    fn into_domain(self) -> PortfolioSnapshot {
        let [p0, p1, p2, p3, p4] = self.positions;
        PortfolioSnapshot {
            cash_1e8: Money1e8(self.cash_1e8),
            positions: [
                p0.into_domain(),
                p1.into_domain(),
                p2.into_domain(),
                p3.into_domain(),
                p4.into_domain(),
            ],
            total_deposits_1e8: Money1e8(self.total_deposits_1e8),
            realized_pnl_gross_1e8: Money1e8(self.realized_pnl_gross_1e8),
            cum_sell_fees_usdt_1e8: Money1e8(self.cum_sell_fees_usdt_1e8),
            cum_buy_fees_base_1e8: [
                Qty1e8(self.cum_buy_fees_base_1e8[0]),
                Qty1e8(self.cum_buy_fees_base_1e8[1]),
                Qty1e8(self.cum_buy_fees_base_1e8[2]),
                Qty1e8(self.cum_buy_fees_base_1e8[3]),
                Qty1e8(self.cum_buy_fees_base_1e8[4]),
            ],
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
    mode: String,
    last_processed_hour_ts_ms: i64,
    last_daily_summary_day_utc: i64,
    last_order_ts_ms: i64,
    portfolio: PortfolioSnapshotDto,
    last_target_weights: [f64; UNIVERSE_LEN],
    last_current_weights: [f64; UNIVERSE_LEN],
    last_sigma_ann: [f64; UNIVERSE_LEN],
    last_tsmom: [f64; UNIVERSE_LEN],
    last_prices_1e8: [i128; UNIVERSE_LEN],
    last_drift_1e8: i128,
}

impl EnginePersistedStateDto {
    fn from_domain(s: &EnginePersistedState) -> Self {
        Self {
            schema_version: s.schema_version,
            mode: s.mode.as_str().to_string(),
            last_processed_hour_ts_ms: s.last_processed_hour_ts_ms,
            last_daily_summary_day_utc: s.last_daily_summary_day_utc,
            last_order_ts_ms: s.last_order_ts_ms,
            portfolio: PortfolioSnapshotDto::from_domain(&s.portfolio),
            last_target_weights: s.last_target_weights,
            last_current_weights: s.last_current_weights,
            last_sigma_ann: s.last_sigma_ann,
            last_tsmom: s.last_tsmom,
            last_prices_1e8: [
                s.last_prices_1e8[0].raw(),
                s.last_prices_1e8[1].raw(),
                s.last_prices_1e8[2].raw(),
                s.last_prices_1e8[3].raw(),
                s.last_prices_1e8[4].raw(),
            ],
            last_drift_1e8: s.last_drift_1e8,
        }
    }

    fn into_domain(self) -> Result<EnginePersistedState, InfraError> {
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
            mode,
            last_processed_hour_ts_ms: self.last_processed_hour_ts_ms,
            last_daily_summary_day_utc: self.last_daily_summary_day_utc,
            last_order_ts_ms: self.last_order_ts_ms,
            portfolio: self.portfolio.into_domain(),
            last_target_weights: self.last_target_weights,
            last_current_weights: self.last_current_weights,
            last_sigma_ann: self.last_sigma_ann,
            last_tsmom: self.last_tsmom,
            last_prices_1e8: [
                Price1e8(self.last_prices_1e8[0]),
                Price1e8(self.last_prices_1e8[1]),
                Price1e8(self.last_prices_1e8[2]),
                Price1e8(self.last_prices_1e8[3]),
                Price1e8(self.last_prices_1e8[4]),
            ],
            last_drift_1e8: self.last_drift_1e8,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct Shadow4hPersistedStateDto {
    schema_version: u32,
    last_processed_4h_close_ts_ms: i64,
    portfolio: PortfolioSnapshotDto,
    last_target_weights: [f64; UNIVERSE_LEN],
    last_current_weights: [f64; UNIVERSE_LEN],
    last_sigma_ann: [f64; UNIVERSE_LEN],
    last_tsmom: [f64; UNIVERSE_LEN],
    last_prices_1e8: [i128; UNIVERSE_LEN],
    last_drift_1e8: i128,
}

impl Shadow4hPersistedStateDto {
    fn from_domain(s: &Shadow4hPersistedState) -> Self {
        Self {
            schema_version: s.schema_version,
            last_processed_4h_close_ts_ms: s.last_processed_4h_close_ts_ms,
            portfolio: PortfolioSnapshotDto::from_domain(&s.portfolio),
            last_target_weights: s.last_target_weights,
            last_current_weights: s.last_current_weights,
            last_sigma_ann: s.last_sigma_ann,
            last_tsmom: s.last_tsmom,
            last_prices_1e8: [
                s.last_prices_1e8[0].raw(),
                s.last_prices_1e8[1].raw(),
                s.last_prices_1e8[2].raw(),
                s.last_prices_1e8[3].raw(),
                s.last_prices_1e8[4].raw(),
            ],
            last_drift_1e8: s.last_drift_1e8,
        }
    }

    fn into_domain(self) -> Shadow4hPersistedState {
        Shadow4hPersistedState {
            schema_version: self.schema_version,
            last_processed_4h_close_ts_ms: self.last_processed_4h_close_ts_ms,
            portfolio: self.portfolio.into_domain(),
            last_target_weights: self.last_target_weights,
            last_current_weights: self.last_current_weights,
            last_sigma_ann: self.last_sigma_ann,
            last_tsmom: self.last_tsmom,
            last_prices_1e8: [
                Price1e8(self.last_prices_1e8[0]),
                Price1e8(self.last_prices_1e8[1]),
                Price1e8(self.last_prices_1e8[2]),
                Price1e8(self.last_prices_1e8[3]),
                Price1e8(self.last_prices_1e8[4]),
            ],
            last_drift_1e8: self.last_drift_1e8,
        }
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
}
