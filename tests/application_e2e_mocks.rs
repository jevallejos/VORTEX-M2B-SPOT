use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use vortex_m2b_spot::application::cycle::{CycleConfig, CycleOutcome, RunHourlyCycleUseCase};
use vortex_m2b_spot::domain::error::{AppError, InfraError};
use vortex_m2b_spot::domain::ports::{
    ClockPort, MarketDataPort, NotifierPort, RunLockGuard, SpotExecutionPort, StateStorePort,
};
use vortex_m2b_spot::domain::types::{
    make_order_link_id, mul_div_floor_nonneg, mul_div_round, AssetSymbol, AuditEntry, Candle,
    EnginePersistedState, ExecutionMode, FillReport, InstrumentRules, Money1e8, OrderRequest,
    OrderSide, Price1e8, Qty1e8, Shadow4hPersistedState, TickerSnapshot, Universe, WalletSnapshot,
    ONE_HOUR_MS, SCALE_1E8, STATE_SCHEMA_VERSION,
};
use vortex_m2b_spot::infrastructure::bybit_spot_v5::{
    parse_wallet_balance_for_universe, sign_bybit_v5, BybitSpotV5Client,
};
use vortex_m2b_spot::infrastructure::config::AppConfig;
use vortex_m2b_spot::infrastructure::state_store::{verify_audit_ledger_line, FsStateStore};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const BASE_UTC_MIDNIGHT_MS: i64 = 1_728_000_000_000;

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn create(prefix: &str) -> Result<Self, Box<dyn std::error::Error>> {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "vortex_m2b_e2e_{prefix}_{}_{}_{}",
            std::process::id(),
            nanos,
            seq
        ));
        fs::create_dir_all(&dir)?;
        Ok(Self { path: dir })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct MockClock {
    now_ms: Mutex<i64>,
}

impl MockClock {
    fn new(ts: i64) -> Self {
        Self {
            now_ms: Mutex::new(ts),
        }
    }

    fn set(&self, ts: i64) -> Result<(), Box<dyn std::error::Error>> {
        let mut g = self.now_ms.lock().map_err(|_| "clock mutex poisoned")?;
        *g = ts;
        Ok(())
    }
}

impl ClockPort for MockClock {
    fn now_ms(&self) -> i64 {
        self.now_ms.lock().map(|g| *g).unwrap_or(0)
    }
}

struct MockMarket {
    universe: Universe,
    server_time_ms: Mutex<i64>,
    candles_by_sym: Mutex<Vec<Vec<Candle>>>,
}

impl MockMarket {
    fn new(universe: Universe, server_time_ms: i64, candles_by_sym: Vec<Vec<Candle>>) -> Self {
        Self {
            universe,
            server_time_ms: Mutex::new(server_time_ms),
            candles_by_sym: Mutex::new(candles_by_sym),
        }
    }

    fn set_state(
        &self,
        server_time_ms: i64,
        candles_by_sym: Vec<Vec<Candle>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        {
            let mut ts = self
                .server_time_ms
                .lock()
                .map_err(|_| "market server_time mutex poisoned")?;
            *ts = server_time_ms;
        }
        {
            let mut c = self
                .candles_by_sym
                .lock()
                .map_err(|_| "market candles mutex poisoned")?;
            *c = candles_by_sym;
        }
        Ok(())
    }
}

impl MarketDataPort for MockMarket {
    fn fetch_server_time_ms(&self) -> Result<i64, InfraError> {
        self.server_time_ms
            .lock()
            .map(|g| *g)
            .map_err(|_| InfraError::Io("market server_time mutex poisoned".to_string()))
    }

    fn fetch_instruments_info(
        &self,
        symbols: &[AssetSymbol],
    ) -> Result<Vec<InstrumentRules>, InfraError> {
        Ok(symbols
            .iter()
            .map(|sym| InstrumentRules {
                symbol: sym.clone(),
                base_precision: Qty1e8(100_000),      // 0.001
                quote_precision: Money1e8(1_000_000), // 0.01
                min_order_qty: Qty1e8(100_000),
                min_order_amt: Money1e8(500_000_000), // $5.00
                tick_size: Price1e8(1_000_000),
                is_trading: true,
            })
            .collect())
    }

    fn fetch_klines_1h(
        &self,
        symbol: &AssetSymbol,
        _limit: usize,
    ) -> Result<Vec<Candle>, InfraError> {
        let id = self
            .universe
            .symbol_id(symbol)
            .ok_or_else(|| InfraError::Parse(format!("unknown symbol {}", symbol)))?;
        let guard = self
            .candles_by_sym
            .lock()
            .map_err(|_| InfraError::Io("market candles mutex poisoned".to_string()))?;
        Ok(guard[id.index()].clone())
    }

    fn fetch_tickers(&self, symbols: &[AssetSymbol]) -> Result<Vec<TickerSnapshot>, InfraError> {
        let server_ts = self.fetch_server_time_ms()?;
        let guard = self
            .candles_by_sym
            .lock()
            .map_err(|_| InfraError::Io("market candles mutex poisoned".to_string()))?;
        let mut out = Vec::with_capacity(symbols.len());
        for sym in symbols {
            let id = self
                .universe
                .symbol_id(sym)
                .ok_or_else(|| InfraError::Parse(format!("unknown symbol {}", sym)))?;
            let series = &guard[id.index()];
            let last_closed_price = series
                .iter()
                .rev()
                .find(|c| c.open_ts_ms + ONE_HOUR_MS <= server_ts)
                .and_then(|c| Price1e8::from_f64_checked(c.close).ok())
                .unwrap_or(Price1e8(10_000_000_000));
            out.push(TickerSnapshot {
                symbol: sym.clone(),
                bid1: last_closed_price,
                ask1: last_closed_price,
                last: last_closed_price,
            });
        }
        Ok(out)
    }
}

#[derive(Default)]
struct SpyExecutionPort {
    calls: AtomicUsize,
}

impl SpyExecutionPort {
    fn total_calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl SpotExecutionPort for SpyExecutionPort {
    fn fetch_wallet_snapshot(&self, _universe: &Universe) -> Result<WalletSnapshot, InfraError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(InfraError::Config(
            "SpyExecutionPort::fetch_wallet_snapshot unexpected call".to_string(),
        ))
    }

    fn query_order_by_link_id(
        &self,
        _symbol: &AssetSymbol,
        _order_link_id: &str,
    ) -> Result<Option<FillReport>, InfraError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(InfraError::Config(
            "SpyExecutionPort::query_order_by_link_id unexpected call".to_string(),
        ))
    }

    fn execute_spot_order(&self, _req: &OrderRequest) -> Result<FillReport, InfraError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(InfraError::Config(
            "SpyExecutionPort::execute_spot_order unexpected call".to_string(),
        ))
    }
}

struct MockLiveExecutionPort {
    universe: Universe,
    wallet: Mutex<WalletSnapshot>,
    submitted: Mutex<Vec<OrderRequest>>,
    fills: Mutex<HashMap<String, FillReport>>,
    seq: AtomicUsize,
}

impl MockLiveExecutionPort {
    fn new(universe: Universe, initial_usdt_1e8: Money1e8, fetched_at_ms: i64) -> Self {
        let n = universe.len();
        Self {
            universe,
            wallet: Mutex::new(WalletSnapshot {
                usdt_free_1e8: initial_usdt_1e8,
                usdt_locked_1e8: Money1e8::ZERO,
                coin_free_1e8: vec![Qty1e8::ZERO; n],
                coin_locked_1e8: vec![Qty1e8::ZERO; n],
                fetched_at_ms,
            }),
            submitted: Mutex::new(Vec::new()),
            fills: Mutex::new(HashMap::new()),
            seq: AtomicUsize::new(1),
        }
    }

    fn from_wallet(universe: Universe, wallet: WalletSnapshot) -> Self {
        Self {
            universe,
            wallet: Mutex::new(wallet),
            submitted: Mutex::new(Vec::new()),
            fills: Mutex::new(HashMap::new()),
            seq: AtomicUsize::new(1),
        }
    }

    fn set_wallet_ts(&self, ts_ms: i64) -> Result<(), Box<dyn std::error::Error>> {
        let mut g = self
            .wallet
            .lock()
            .map_err(|_| "mock live wallet mutex poisoned")?;
        g.fetched_at_ms = ts_ms;
        Ok(())
    }

    fn inject_coin_free(
        &self,
        symbol_idx: usize,
        add_qty: Qty1e8,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut g = self
            .wallet
            .lock()
            .map_err(|_| "mock live wallet mutex poisoned")?;
        g.coin_free_1e8[symbol_idx] = g.coin_free_1e8[symbol_idx].checked_add(add_qty)?;
        Ok(())
    }

    fn wallet_snapshot(&self) -> Result<WalletSnapshot, Box<dyn std::error::Error>> {
        let g = self
            .wallet
            .lock()
            .map_err(|_| "mock live wallet mutex poisoned")?;
        Ok(g.clone())
    }

    fn submitted_orders(&self) -> Result<Vec<OrderRequest>, Box<dyn std::error::Error>> {
        let g = self
            .submitted
            .lock()
            .map_err(|_| "mock live submitted mutex poisoned")?;
        Ok(g.clone())
    }

    fn clear_submitted_orders(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut g = self
            .submitted
            .lock()
            .map_err(|_| "mock live submitted mutex poisoned")?;
        g.clear();
        Ok(())
    }

    fn fill_by_link_id(
        &self,
        order_link_id: &str,
    ) -> Result<Option<FillReport>, Box<dyn std::error::Error>> {
        let g = self
            .fills
            .lock()
            .map_err(|_| "mock live fills mutex poisoned")?;
        Ok(g.get(order_link_id).cloned())
    }
}

impl SpotExecutionPort for MockLiveExecutionPort {
    fn fetch_wallet_snapshot(&self, _universe: &Universe) -> Result<WalletSnapshot, InfraError> {
        self.wallet
            .lock()
            .map(|g| g.clone())
            .map_err(|_| InfraError::Io("mock live wallet mutex poisoned".to_string()))
    }

    fn query_order_by_link_id(
        &self,
        _symbol: &AssetSymbol,
        order_link_id: &str,
    ) -> Result<Option<FillReport>, InfraError> {
        let g = self
            .fills
            .lock()
            .map_err(|_| InfraError::Io("mock live fills mutex poisoned".to_string()))?;
        Ok(g.get(order_link_id).cloned())
    }

    fn execute_spot_order(&self, req: &OrderRequest) -> Result<FillReport, InfraError> {
        {
            let mut sub = self
                .submitted
                .lock()
                .map_err(|_| InfraError::Io("mock live submitted mutex poisoned".to_string()))?;
            sub.push(req.clone());
        }

        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let order_id = format!("mock_order_{seq}");
        let idx = self
            .universe
            .symbol_id(&req.symbol)
            .ok_or_else(|| InfraError::Parse(format!("unknown symbol {}", req.symbol)))?
            .index();

        let mut wallet = self
            .wallet
            .lock()
            .map_err(|_| InfraError::Io("mock live wallet mutex poisoned".to_string()))?;

        let fill = match req.side {
            OrderSide::Buy => {
                let quote_spent = req.quote_spend_1e8;
                let gross_base = Qty1e8(
                    mul_div_floor_nonneg(
                        quote_spent.raw(),
                        SCALE_1E8,
                        req.reference_price_1e8.raw(),
                    )
                    .map_err(|e| InfraError::Parse(e.to_string()))?,
                );
                let fee_base = Qty1e8(
                    mul_div_floor_nonneg(gross_base.raw(), 10, 10_000)
                        .map_err(|e| InfraError::Parse(e.to_string()))?,
                );
                let net_base = gross_base
                    .checked_sub(fee_base)
                    .and_then(|q| q.floor_to_step(req.base_precision))
                    .map_err(|e| InfraError::Parse(e.to_string()))?;

                wallet.usdt_free_1e8 = wallet
                    .usdt_free_1e8
                    .checked_sub(quote_spent)
                    .map_err(|e| InfraError::Parse(e.to_string()))?;
                wallet.coin_free_1e8[idx] = wallet.coin_free_1e8[idx]
                    .checked_add(net_base)
                    .map_err(|e| InfraError::Parse(e.to_string()))?;

                FillReport {
                    symbol: req.symbol.clone(),
                    side: OrderSide::Buy,
                    order_id,
                    order_link_id: req.order_link_id.clone(),
                    exec_qty_base_1e8: gross_base,
                    exec_quote_1e8: quote_spent,
                    avg_price_1e8: req.reference_price_1e8,
                    fee_1e8: fee_base.raw(),
                    fee_in_base: true,
                    filled_at_ms: wallet.fetched_at_ms,
                }
            }
            OrderSide::Sell => {
                let sell_qty = req.base_qty_1e8;
                let gross_quote = Money1e8(
                    mul_div_floor_nonneg(sell_qty.raw(), req.reference_price_1e8.raw(), SCALE_1E8)
                        .map_err(|e| InfraError::Parse(e.to_string()))?,
                );
                let fee_usdt = Money1e8(
                    mul_div_round(gross_quote.raw(), 10, 10_000)
                        .map_err(|e| InfraError::Parse(e.to_string()))?,
                );
                let net_quote = gross_quote
                    .checked_sub(fee_usdt)
                    .map_err(|e| InfraError::Parse(e.to_string()))?;

                wallet.coin_free_1e8[idx] = wallet.coin_free_1e8[idx]
                    .checked_sub(sell_qty)
                    .map_err(|e| InfraError::Parse(e.to_string()))?;
                wallet.usdt_free_1e8 = wallet
                    .usdt_free_1e8
                    .checked_add(net_quote)
                    .map_err(|e| InfraError::Parse(e.to_string()))?;

                FillReport {
                    symbol: req.symbol.clone(),
                    side: OrderSide::Sell,
                    order_id,
                    order_link_id: req.order_link_id.clone(),
                    exec_qty_base_1e8: sell_qty,
                    exec_quote_1e8: gross_quote,
                    avg_price_1e8: req.reference_price_1e8,
                    fee_1e8: fee_usdt.raw(),
                    fee_in_base: false,
                    filled_at_ms: wallet.fetched_at_ms,
                }
            }
        };

        let mut fills = self
            .fills
            .lock()
            .map_err(|_| InfraError::Io("mock live fills mutex poisoned".to_string()))?;
        fills.insert(req.order_link_id.clone(), fill.clone());
        Ok(fill)
    }
}

struct DummyLockGuard;
impl RunLockGuard for DummyLockGuard {}

#[derive(Default)]
struct MemStateStore {
    drain_lock: Mutex<bool>,
    halt_reason: Mutex<Option<String>>,
    primary: Mutex<Option<EnginePersistedState>>,
    shadow: Mutex<Option<Shadow4hPersistedState>>,
    audits: Mutex<Vec<AuditEntry>>,
}

impl MemStateStore {
    fn set_drain_lock(&self, active: bool) -> Result<(), Box<dyn std::error::Error>> {
        let mut g = self.drain_lock.lock().map_err(|_| "drain mutex poisoned")?;
        *g = active;
        Ok(())
    }

    fn audits_snapshot(&self) -> Result<Vec<AuditEntry>, Box<dyn std::error::Error>> {
        let g = self.audits.lock().map_err(|_| "audits mutex poisoned")?;
        Ok(g.clone())
    }
}

impl StateStorePort for MemStateStore {
    fn acquire_run_lock(&self) -> Result<Box<dyn RunLockGuard>, InfraError> {
        Ok(Box::new(DummyLockGuard))
    }

    fn has_drain_lock(&self) -> Result<bool, InfraError> {
        self.drain_lock
            .lock()
            .map(|g| *g)
            .map_err(|_| InfraError::Io("drain mutex poisoned".to_string()))
    }

    fn has_halt_lock(&self) -> Result<bool, InfraError> {
        self.halt_reason
            .lock()
            .map(|g| g.is_some())
            .map_err(|_| InfraError::Io("halt mutex poisoned".to_string()))
    }

    fn write_halt_lock(&self, reason: &str) -> Result<(), InfraError> {
        let mut g = self
            .halt_reason
            .lock()
            .map_err(|_| InfraError::Io("halt mutex poisoned".to_string()))?;
        *g = Some(reason.to_string());
        Ok(())
    }

    fn load_primary_state(
        &self,
        expected_universe: &Universe,
    ) -> Result<Option<EnginePersistedState>, InfraError> {
        let g = self
            .primary
            .lock()
            .map_err(|_| InfraError::Io("primary mutex poisoned".to_string()))?;
        match &*g {
            None => Ok(None),
            Some(st) => {
                st.validate_against(expected_universe)
                    .map_err(|e| InfraError::Parse(e.to_string()))?;
                Ok(Some(st.clone()))
            }
        }
    }

    fn save_primary_state(&self, state: &EnginePersistedState) -> Result<(), InfraError> {
        let mut g = self
            .primary
            .lock()
            .map_err(|_| InfraError::Io("primary mutex poisoned".to_string()))?;
        *g = Some(state.clone());
        Ok(())
    }

    fn load_shadow_4h_state(
        &self,
        expected_universe: &Universe,
    ) -> Result<Option<Shadow4hPersistedState>, InfraError> {
        let g = self
            .shadow
            .lock()
            .map_err(|_| InfraError::Io("shadow mutex poisoned".to_string()))?;
        match &*g {
            None => Ok(None),
            Some(st) => {
                st.validate_against(expected_universe)
                    .map_err(|e| InfraError::Parse(e.to_string()))?;
                Ok(Some(st.clone()))
            }
        }
    }

    fn save_shadow_4h_state(&self, state: &Shadow4hPersistedState) -> Result<(), InfraError> {
        let mut g = self
            .shadow
            .lock()
            .map_err(|_| InfraError::Io("shadow mutex poisoned".to_string()))?;
        *g = Some(state.clone());
        Ok(())
    }

    fn append_audit_entry(&self, entry: &AuditEntry) -> Result<(), InfraError> {
        let mut g = self
            .audits
            .lock()
            .map_err(|_| InfraError::Io("audits mutex poisoned".to_string()))?;
        g.push(entry.clone());
        Ok(())
    }
}

#[derive(Default)]
struct SpyNotifier {
    messages: Mutex<Vec<String>>,
}

impl SpyNotifier {
    fn count(&self) -> Result<usize, Box<dyn std::error::Error>> {
        let g = self
            .messages
            .lock()
            .map_err(|_| "notifier mutex poisoned")?;
        Ok(g.len())
    }

    fn messages_snapshot(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let g = self
            .messages
            .lock()
            .map_err(|_| "notifier mutex poisoned")?;
        Ok(g.clone())
    }
}

impl NotifierPort for SpyNotifier {
    fn send_message(&self, text: &str) -> Result<(), InfraError> {
        let mut g = self
            .messages
            .lock()
            .map_err(|_| InfraError::Notifier("notifier mutex poisoned".to_string()))?;
        g.push(text.to_string());
        Ok(())
    }
}

struct FailingNotifier {
    attempts: AtomicUsize,
}

impl FailingNotifier {
    fn new() -> Self {
        Self {
            attempts: AtomicUsize::new(0),
        }
    }

    fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }
}

impl NotifierPort for FailingNotifier {
    fn send_message(&self, _text: &str) -> Result<(), InfraError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Err(InfraError::Notifier(
            "simulated Telegram API 502 outage".to_string(),
        ))
    }
}

fn build_uptrend_1h(count: usize, start_price: f64, step: f64) -> Vec<Candle> {
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        let open = start_price + (k as f64) * step;
        let close = open + step * 0.8;
        let high = open.max(close) + 0.5;
        let low = open.min(close) - 0.3;
        out.push(Candle {
            open_ts_ms: BASE_UTC_MIDNIGHT_MS + (k as i64) * ONE_HOUR_MS,
            open,
            high,
            low,
            close,
            volume: 25.0,
            turnover: 25.0 * close,
        });
    }
    out
}

fn build_universe(n: usize, count: usize, positive_count: usize) -> Vec<Vec<Candle>> {
    (0..n)
        .map(|i| {
            if i < positive_count {
                build_uptrend_1h(count, 100.0 + (i as f64) * 5.0, 0.15)
            } else {
                build_uptrend_1h(count, 200.0, -0.05)
            }
        })
        .collect()
}

fn paper_cfg(universe: Universe) -> CycleConfig {
    CycleConfig {
        universe,
        mode: ExecutionMode::Paper,
        paper_initial_cash_1e8: Money1e8(100_000_000_000), // $1000.00
        live_execution_env_flag: false,
        live_ack_valid: false,
        max_clock_skew_ms: 2_000,
        recv_window_ms: 5_000,
    }
}

#[test]
fn multi_hour_crossing_4h_utc_boundary_and_idempotency() -> TestResult {
    let universe = Universe::default_universe()?;
    let n = universe.len();
    // 603 bars starting at 00:00 UTC -> bar 602 opens at 02:00 UTC, closes at 03:00 UTC (NOT a 4h boundary)
    let hour_03_close = BASE_UTC_MIDNIGHT_MS + 603 * ONE_HOUR_MS;
    let server_03 = hour_03_close + 2_000;
    let candles_03 = build_universe(n, 603, n);

    let clock = MockClock::new(server_03 + 50);
    let market = MockMarket::new(universe.clone(), server_03, candles_03);
    let exec = SpyExecutionPort::default();
    let store = MemStateStore::default();
    let notifier = SpyNotifier::default();
    let uc = RunHourlyCycleUseCase::new(&clock, &market, &exec, &store, &notifier);
    let cfg = paper_cfg(universe.clone());

    // Hour 1: 03:00 UTC close -> B2_01 executes, A0_4H must NOT update
    let out_03 = uc.run(&cfg)?;
    match out_03 {
        CycleOutcome::Executed {
            closed_hour_ts_ms,
            orders_executed,
            drift_1e8,
            shadow_4h_updated,
            halted,
            ..
        } => {
            assert_eq!(closed_hour_ts_ms, hour_03_close);
            assert!(orders_executed > 0);
            assert_eq!(drift_1e8, 0);
            assert!(!shadow_4h_updated);
            assert!(!halted);
        }
        other => return Err(format!("expected Executed at 03:00 UTC, got {other:?}").into()),
    }
    assert!(store.load_primary_state(&universe)?.is_some());
    assert!(store.load_shadow_4h_state(&universe)?.is_none());
    assert_eq!(exec.total_calls(), 0);

    // Idempotency check on 03:00 UTC
    let msgs_before_dup = notifier.count()?;
    let audits_before_dup = store.audits_snapshot()?.len();
    let dup = uc.run(&cfg)?;
    assert_eq!(
        dup,
        CycleOutcome::AlreadyProcessed {
            closed_hour_ts_ms: hour_03_close
        }
    );
    assert_eq!(notifier.count()?, msgs_before_dup);
    assert_eq!(store.audits_snapshot()?.len(), audits_before_dup);
    assert_eq!(exec.total_calls(), 0);

    // Hour 2: 04:00 UTC close -> 4h boundary!
    let hour_04_close = BASE_UTC_MIDNIGHT_MS + 604 * ONE_HOUR_MS;
    let server_04 = hour_04_close + 2_000;
    let candles_04 = build_universe(n, 604, n);
    clock.set(server_04 + 50)?;
    market.set_state(server_04, candles_04)?;

    let out_04 = uc.run(&cfg)?;
    match out_04 {
        CycleOutcome::Executed {
            closed_hour_ts_ms,
            drift_1e8,
            shadow_4h_updated,
            halted,
            ..
        } => {
            assert_eq!(closed_hour_ts_ms, hour_04_close);
            assert_eq!(drift_1e8, 0);
            assert!(shadow_4h_updated);
            assert!(!halted);
        }
        other => return Err(format!("expected Executed at 04:00 UTC, got {other:?}").into()),
    }

    let primary_04 = store
        .load_primary_state(&universe)?
        .ok_or("primary state missing after 04:00 UTC")?;
    assert_eq!(primary_04.schema_version, STATE_SCHEMA_VERSION);
    assert_eq!(primary_04.symbols, universe.symbols());
    assert_eq!(primary_04.last_processed_hour_ts_ms, hour_04_close);
    assert_eq!(primary_04.last_drift_1e8, 0);

    let shadow_04 = store
        .load_shadow_4h_state(&universe)?
        .ok_or("shadow 4h state missing after 04:00 UTC")?;
    assert_eq!(shadow_04.schema_version, STATE_SCHEMA_VERSION);
    assert_eq!(shadow_04.symbols, universe.symbols());
    assert_eq!(shadow_04.last_processed_4h_close_ts_ms, hour_04_close);
    assert_eq!(shadow_04.last_drift_1e8, 0);
    assert_eq!(exec.total_calls(), 0);

    Ok(())
}

#[test]
fn live_spot_double_safety_lock_and_lockfile_rejections() -> TestResult {
    let universe = Universe::default_universe()?;
    let n = universe.len();
    let clock = MockClock::new(1_728_000_000_000);
    let market = MockMarket::new(
        universe.clone(),
        1_728_000_000_000,
        build_universe(n, 604, n),
    );
    let exec = SpyExecutionPort::default();
    let store = MemStateStore::default();
    let notifier = SpyNotifier::default();
    let uc = RunHourlyCycleUseCase::new(&clock, &market, &exec, &store, &notifier);

    // 1) Missing BYBIT_SPOT_LIVE_EXECUTION=true
    let res_no_env = uc.run(&CycleConfig {
        universe: universe.clone(),
        mode: ExecutionMode::LiveSpot,
        paper_initial_cash_1e8: Money1e8(100_000_000_000),
        live_execution_env_flag: false,
        live_ack_valid: true,
        max_clock_skew_ms: 2_000,
        recv_window_ms: 5_000,
    });
    assert!(matches!(res_no_env, Err(AppError::LiveSafetyLockDenied(_))));

    // 2) Missing/invalid BYBIT_SPOT_ACK_LIVE via AppConfig::from_env_map
    let mut env_bad_ack = HashMap::new();
    env_bad_ack.insert("BYBIT_SPOT_LIVE_EXECUTION".to_string(), "true".to_string());
    env_bad_ack.insert("BYBIT_SPOT_ACK_LIVE".to_string(), "WRONG_MAGIC".to_string());
    let parsed_cfg = AppConfig::from_env_map(&env_bad_ack)?;
    assert!(parsed_cfg.live_execution_env_flag);
    assert!(!parsed_cfg.live_ack_valid);

    let res_bad_ack = uc.run(&CycleConfig {
        universe: parsed_cfg.universe,
        mode: ExecutionMode::LiveSpot,
        paper_initial_cash_1e8: parsed_cfg.paper_initial_cash_1e8,
        live_execution_env_flag: parsed_cfg.live_execution_env_flag,
        live_ack_valid: parsed_cfg.live_ack_valid,
        max_clock_skew_ms: parsed_cfg.max_clock_skew_ms,
        recv_window_ms: parsed_cfg.recv_window_ms,
    });
    assert!(matches!(
        res_bad_ack,
        Err(AppError::LiveSafetyLockDenied(_))
    ));

    // 3) Active HALT_LOCK -> HaltLockActive
    store.write_halt_lock("manual test halt")?;
    let res_halt = uc.run(&CycleConfig {
        universe: universe.clone(),
        mode: ExecutionMode::LiveSpot,
        paper_initial_cash_1e8: Money1e8(100_000_000_000),
        live_execution_env_flag: true,
        live_ack_valid: true,
        max_clock_skew_ms: 2_000,
        recv_window_ms: 5_000,
    });
    assert!(matches!(res_halt, Err(AppError::HaltLockActive(_))));

    // 4) Active DRAIN_MODE.lock + HALT_LOCK -> HALT_LOCK still fails closed
    store.set_drain_lock(true)?;
    let res_drain_and_halt = uc.run(&CycleConfig {
        universe,
        mode: ExecutionMode::LiveSpot,
        paper_initial_cash_1e8: Money1e8(100_000_000_000),
        live_execution_env_flag: true,
        live_ack_valid: true,
        max_clock_skew_ms: 2_000,
        recv_window_ms: 5_000,
    });
    assert!(matches!(
        res_drain_and_halt,
        Err(AppError::HaltLockActive(_))
    ));
    assert_eq!(exec.total_calls(), 0);

    Ok(())
}

#[test]
fn drain_mode_in_paper_blocks_new_buys_and_allows_exit_sells() -> TestResult {
    let universe = Universe::default_universe()?;
    let n = universe.len();
    let dir = TempDirGuard::create("drain_paper")?;
    let store = FsStateStore::new(dir.path().join("state"), dir.path().join("logs"))?;
    let cfg = paper_cfg(universe.clone());

    // Hour 1: only BTCUSDT in uptrend -> buys BTC (~$400), leaves ~$600 cash
    let hour1_close = BASE_UTC_MIDNIGHT_MS + 603 * ONE_HOUR_MS;
    let clock = MockClock::new(hour1_close + 1_000);
    let market = MockMarket::new(
        universe.clone(),
        hour1_close + 1_000,
        build_universe(n, 603, 1),
    );
    let exec = SpyExecutionPort::default();
    let notifier = SpyNotifier::default();
    let uc = RunHourlyCycleUseCase::new(&clock, &market, &exec, &store, &notifier);

    let out1 = uc.run(&cfg)?;
    assert!(matches!(
        out1,
        CycleOutcome::Executed {
            orders_executed,
            ..
        } if orders_executed == 1
    ));

    // Activate DRAIN_MODE.lock on disk
    fs::write(store.state_dir().join("DRAIN_MODE.lock"), b"drain active\n")?;
    assert!(store.has_drain_lock()?);

    // Hour 2: BTC + ETH uptrend -> new Buy for ETH MUST be blocked!
    let hour2_close = BASE_UTC_MIDNIGHT_MS + 604 * ONE_HOUR_MS;
    clock.set(hour2_close + 1_000)?;
    market.set_state(hour2_close + 1_000, build_universe(n, 604, 2))?;

    let out2 = uc.run(&cfg)?;
    match out2 {
        CycleOutcome::Executed {
            orders_executed,
            drift_1e8,
            drain_mode,
            ..
        } => {
            assert_eq!(orders_executed, 0, "DRAIN_MODE must block all new buys");
            assert_eq!(drift_1e8, 0);
            assert!(drain_mode);
        }
        other => return Err(format!("expected Executed at hour2, got {other:?}").into()),
    }
    let state_after_h2 = store
        .load_primary_state(&universe)?
        .ok_or("missing primary state after hour2")?;
    assert_eq!(state_after_h2.portfolio.positions[1].qty_1e8, Qty1e8::ZERO);

    // Hour 3: crash BTCUSDT sharply while DRAIN_MODE.lock is active -> exit Sell permitted!
    let hour3_close = BASE_UTC_MIDNIGHT_MS + 605 * ONE_HOUR_MS;
    let mut crash_universe = build_universe(n, 605, 0);
    let mut btc_crash = build_uptrend_1h(605, 100.0, 0.15);
    for (idx, bar) in btc_crash.iter_mut().enumerate().skip(400) {
        let drop = (idx - 400) as f64 * 0.45;
        let p = (160.0 - drop).max(10.0);
        bar.open = p + 0.2;
        bar.close = p;
        bar.high = p + 0.5;
        bar.low = p - 0.5;
    }
    crash_universe[0] = btc_crash;

    clock.set(hour3_close + 1_000)?;
    market.set_state(hour3_close + 1_000, crash_universe)?;

    let out3 = uc.run(&cfg)?;
    match out3 {
        CycleOutcome::Executed {
            orders_executed,
            drift_1e8,
            drain_mode,
            ..
        } => {
            assert_eq!(orders_executed, 1, "DRAIN_MODE must permit exit Sell");
            assert_eq!(drift_1e8, 0);
            assert!(drain_mode);
        }
        other => return Err(format!("expected Executed at hour3, got {other:?}").into()),
    }

    let alerts = notifier.messages_snapshot()?;
    assert!(
        alerts.iter().any(|m| m.contains("drain_mode=true")),
        "expected Telegram alert to include drain_mode=true, got {alerts:?}"
    );

    let state_after = store
        .load_primary_state(&universe)?
        .ok_or("missing primary state after exit sell")?;
    assert_eq!(state_after.portfolio.positions[0].qty_1e8, Qty1e8::ZERO);
    assert_eq!(exec.total_calls(), 0);

    Ok(())
}

#[test]
fn drain_mode_in_live_spot_blocks_new_buys_and_executes_exit_sells() -> TestResult {
    let universe = Universe::default_universe()?;
    let n = universe.len();
    let dir = TempDirGuard::create("drain_live_spot")?;
    let store = FsStateStore::new(dir.path().join("state"), dir.path().join("logs"))?;
    let live_cfg = CycleConfig {
        universe: universe.clone(),
        mode: ExecutionMode::LiveSpot,
        paper_initial_cash_1e8: Money1e8(100_000_000_000),
        live_execution_env_flag: true,
        live_ack_valid: true,
        max_clock_skew_ms: 2_000,
        recv_window_ms: 5_000,
    };

    // Hour 1: only BTCUSDT in uptrend -> opens BTC position via live Buy
    let hour1_close = BASE_UTC_MIDNIGHT_MS + 603 * ONE_HOUR_MS;
    let clock = MockClock::new(hour1_close + 1_000);
    let market = MockMarket::new(
        universe.clone(),
        hour1_close + 1_000,
        build_universe(n, 603, 1),
    );
    let exec = MockLiveExecutionPort::new(
        universe.clone(),
        Money1e8(100_000_000_000),
        hour1_close + 1_000,
    );
    let notifier = SpyNotifier::default();
    let uc = RunHourlyCycleUseCase::new(&clock, &market, &exec, &store, &notifier);

    let out1 = uc.run(&live_cfg)?;
    match out1 {
        CycleOutcome::Executed {
            closed_hour_ts_ms,
            orders_executed,
            drift_1e8,
            halted,
            drain_mode,
            ..
        } => {
            assert_eq!(closed_hour_ts_ms, hour1_close);
            assert_eq!(orders_executed, 1);
            assert_eq!(drift_1e8, 0);
            assert!(!halted);
            assert!(!drain_mode);
        }
        other => return Err(format!("expected Executed at hour1, got {other:?}").into()),
    }

    let state_before_drain = store
        .load_primary_state(&universe)?
        .ok_or("missing primary state after hour1")?;
    let cash_before_drain = state_before_drain.portfolio.cash_1e8;
    assert!(
        state_before_drain.portfolio.positions[0].qty_1e8 > Qty1e8::ZERO,
        "expected BTCUSDT position opened in hour1"
    );
    exec.clear_submitted_orders()?;

    // Activate DRAIN_MODE.lock on disk
    fs::write(
        store.state_dir().join("DRAIN_MODE.lock"),
        b"reason=live_drain_test\n",
    )?;
    assert!(store.has_drain_lock()?);

    // Hour 2 (with DRAIN_MODE.lock): BTC flips to downtrend (exit Sell), ETH switches to uptrend (blocked Buy)
    let hour2_close = BASE_UTC_MIDNIGHT_MS + 604 * ONE_HOUR_MS;
    let hour2_universe: Vec<Vec<Candle>> = (0..n)
        .map(|i| {
            if i == 1 {
                build_uptrend_1h(604, 105.0, 0.15)
            } else {
                build_uptrend_1h(604, 200.0, -0.05)
            }
        })
        .collect();

    clock.set(hour2_close + 1_000)?;
    market.set_state(hour2_close + 1_000, hour2_universe)?;
    exec.set_wallet_ts(hour2_close + 1_000)?;

    let out2 = uc.run(&live_cfg)?;
    match out2 {
        CycleOutcome::Executed {
            closed_hour_ts_ms,
            orders_executed,
            drift_1e8,
            halted,
            drain_mode,
            ..
        } => {
            assert_eq!(closed_hour_ts_ms, hour2_close);
            assert_eq!(
                orders_executed, 1,
                "DRAIN_MODE in live-spot must execute only the exit Sell"
            );
            assert_eq!(drift_1e8, 0);
            assert!(!halted);
            assert!(drain_mode);
        }
        other => return Err(format!("expected Executed at hour2, got {other:?}").into()),
    }

    let submitted_h2 = exec.submitted_orders()?;
    assert_eq!(submitted_h2.len(), 1);
    let btc_sym = universe.symbols()[0].clone();
    let expected_link_id = make_order_link_id(hour2_close, &btc_sym, OrderSide::Sell);
    assert_eq!(submitted_h2[0].symbol, btc_sym);
    assert_eq!(submitted_h2[0].side, OrderSide::Sell);
    assert_eq!(submitted_h2[0].order_link_id, expected_link_id);
    assert!(submitted_h2.iter().all(|o| o.side != OrderSide::Buy));
    assert!(exec.fill_by_link_id(&expected_link_id)?.is_some());

    let state_after = store
        .load_primary_state(&universe)?
        .ok_or("missing primary state after hour2")?;
    assert_eq!(state_after.last_target_weights[0], 0.0);
    assert!(state_after.last_target_weights[1] >= 0.08);
    assert_eq!(state_after.portfolio.positions[0].qty_1e8, Qty1e8::ZERO);
    assert_eq!(state_after.portfolio.positions[1].qty_1e8, Qty1e8::ZERO);
    assert!(state_after.portfolio.cash_1e8 > cash_before_drain);
    assert_eq!(
        state_after.portfolio.cash_1e8,
        exec.wallet_snapshot()?.usdt_free_1e8
    );
    assert_eq!(state_after.last_drift_1e8, 0);

    let alerts = notifier.messages_snapshot()?;
    assert!(alerts.iter().any(|m| m.contains("live-spot")
        && m.contains("orders=1")
        && m.contains("drain_mode=true")));

    let raw_ledger = fs::read_to_string(store.logs_dir().join("m2b_audit_ledger.jsonl"))?;
    let lines: Vec<&str> = raw_ledger.lines().collect();
    assert!(!lines.is_empty());
    for line in lines {
        assert!(verify_audit_ledger_line(line)?);
        assert!(line.contains("\"drift_1e8\":0"));
    }

    Ok(())
}

#[test]
fn telegram_notifier_failure_does_not_abort_cycle_or_persistence() -> TestResult {
    let universe = Universe::default_universe()?;
    let n = universe.len();
    let dir = TempDirGuard::create("tg_resilience")?;
    let store = FsStateStore::new(dir.path().join("state"), dir.path().join("logs"))?;

    let hour_04_close = BASE_UTC_MIDNIGHT_MS + 604 * ONE_HOUR_MS;
    let clock = MockClock::new(hour_04_close + 1_000);
    let market = MockMarket::new(
        universe.clone(),
        hour_04_close + 1_000,
        build_universe(n, 604, n),
    );
    let exec = SpyExecutionPort::default();
    let failing_notifier = FailingNotifier::new();

    let uc = RunHourlyCycleUseCase::new(&clock, &market, &exec, &store, &failing_notifier);
    let out = uc.run(&paper_cfg(universe.clone()))?;

    match out {
        CycleOutcome::Executed {
            closed_hour_ts_ms,
            orders_executed,
            drift_1e8,
            shadow_4h_updated,
            ..
        } => {
            assert_eq!(closed_hour_ts_ms, hour_04_close);
            assert!(orders_executed > 0);
            assert_eq!(drift_1e8, 0);
            assert!(shadow_4h_updated);
        }
        other => {
            return Err(format!("expected Executed despite Telegram error, got {other:?}").into())
        }
    }

    assert!(failing_notifier.attempts() >= 2);
    assert!(store.load_primary_state(&universe)?.is_some());
    assert!(store.load_shadow_4h_state(&universe)?.is_some());
    assert!(store.primary_state_path().exists());
    assert!(store.shadow_4h_state_path().exists());

    Ok(())
}

#[test]
fn bybit_v5_hmac_sha256_known_test_vectors() -> TestResult {
    let sig1 = sign_bybit_v5(
        1_658_385_579_423,
        "XXXXXXXXXX",
        5_000,
        "accountType=UNIFIED",
        "SSSSSSSSSS",
    )?;
    assert_eq!(
        sig1,
        "fb565caa9e5a295032d7b24d4823eae2470a6e2d97a4fb9873628cc1b1388fe5"
    );

    let post_payload = r#"{"category":"spot","symbol":"BTCUSDT","side":"Buy","orderType":"Market","qty":"100.00","marketUnit":"quoteCoin","orderLinkId":"m2b_1700000000000_BTCUSDT_buy"}"#;
    let sig2 = sign_bybit_v5(
        1_700_000_000_123,
        "TESTKEY123",
        5_000,
        post_payload,
        "SUPERSECRET456",
    )?;
    assert_eq!(
        sig2,
        "48e11dddf8bb1195bbd26eceac24eae634289c539850537d49418cbec4677680"
    );

    assert!(matches!(
        sign_bybit_v5(1_700_000_000_000, "", 5_000, "x", "secret"),
        Err(InfraError::Config(_))
    ));
    assert!(matches!(
        sign_bybit_v5(1_700_000_000_000, "key", 5_000, "x", ""),
        Err(InfraError::Config(_))
    ));

    Ok(())
}

#[test]
fn fs_state_store_atomic_persistence_audit_seal_and_run_lock_contention() -> TestResult {
    let universe = Universe::default_universe()?;
    let n = universe.len();
    let dir = TempDirGuard::create("fs_store")?;
    let state_dir = dir.path().join("state");
    let logs_dir = dir.path().join("logs");
    let store = FsStateStore::new(&state_dir, &logs_dir)?;

    let hour_04_close = BASE_UTC_MIDNIGHT_MS + 604 * ONE_HOUR_MS;
    let clock = MockClock::new(hour_04_close + 1_000);
    let market = MockMarket::new(
        universe.clone(),
        hour_04_close + 1_000,
        build_universe(n, 604, n),
    );
    let exec = SpyExecutionPort::default();
    let notifier = SpyNotifier::default();
    let uc = RunHourlyCycleUseCase::new(&clock, &market, &exec, &store, &notifier);

    let out = uc.run(&paper_cfg(universe.clone()))?;
    assert!(matches!(
        out,
        CycleOutcome::Executed {
            shadow_4h_updated: true,
            drift_1e8: 0,
            ..
        }
    ));

    assert!(state_dir.join("primary_state.json").exists());
    assert!(state_dir.join("m2b_shadow_4h_state.json").exists());
    let ledger_path = logs_dir.join("m2b_audit_ledger.jsonl");
    assert!(ledger_path.exists());

    let loaded_primary = store
        .load_primary_state(&universe)?
        .ok_or("primary_state.json reload returned None")?;
    assert_eq!(loaded_primary.schema_version, 2);
    assert_eq!(loaded_primary.symbols, universe.symbols());
    assert_eq!(loaded_primary.last_processed_hour_ts_ms, hour_04_close);
    assert_eq!(loaded_primary.last_drift_1e8, 0);

    let loaded_shadow = store
        .load_shadow_4h_state(&universe)?
        .ok_or("m2b_shadow_4h_state.json reload returned None")?;
    assert_eq!(loaded_shadow.schema_version, 2);
    assert_eq!(loaded_shadow.symbols, universe.symbols());
    assert_eq!(loaded_shadow.last_processed_4h_close_ts_ms, hour_04_close);
    assert_eq!(loaded_shadow.last_drift_1e8, 0);

    let raw_ledger = fs::read_to_string(&ledger_path)?;
    let lines: Vec<&str> = raw_ledger.lines().collect();
    assert!(!lines.is_empty());
    for line in &lines {
        assert!(verify_audit_ledger_line(line)?);
    }

    let first = lines[0];
    let tampered = first.replace("\"drift_1e8\":0", "\"drift_1e8\":1");
    assert_ne!(tampered, first);
    assert!(!verify_audit_ledger_line(&tampered)?);

    let lock_guard = store.acquire_run_lock()?;
    let second_lock = store.acquire_run_lock();
    assert!(matches!(second_lock, Err(InfraError::LockBusy(_))));

    let cycle_while_locked = uc.run(&paper_cfg(universe));
    assert!(matches!(
        cycle_while_locked,
        Err(AppError::Infra(InfraError::LockBusy(_)))
    ));

    drop(lock_guard);
    let reacquired = store.acquire_run_lock()?;
    drop(reacquired);

    Ok(())
}

#[test]
fn state_store_schema_v2_fail_closed_rejects_v1_universe_mismatch_order_and_corrupt_lengths(
) -> TestResult {
    let default_u = Universe::default_universe()?;
    let n = default_u.len();
    let dir = TempDirGuard::create("schema_v2_fail_closed")?;
    let store = FsStateStore::new(dir.path().join("state"), dir.path().join("logs"))?;

    // Run a cycle crossing 4h UTC boundary to persist real primary_state.json & m2b_shadow_4h_state.json
    let hour_04_close = BASE_UTC_MIDNIGHT_MS + 604 * ONE_HOUR_MS;
    let clock = MockClock::new(hour_04_close + 1_000);
    let market = MockMarket::new(
        default_u.clone(),
        hour_04_close + 1_000,
        build_universe(n, 604, n),
    );
    let exec = SpyExecutionPort::default();
    let notifier = SpyNotifier::default();
    let uc = RunHourlyCycleUseCase::new(&clock, &market, &exec, &store, &notifier);
    assert!(matches!(
        uc.run(&paper_cfg(default_u.clone()))?,
        CycleOutcome::Executed {
            shadow_4h_updated: true,
            ..
        }
    ));

    let primary_v2_bytes = fs::read_to_string(store.primary_state_path())?;
    let shadow_v2_bytes = fs::read_to_string(store.shadow_4h_state_path())?;
    assert!(primary_v2_bytes.contains("\"schema_version\": 2"));
    assert!(primary_v2_bytes.contains("\"XAUTUSDT\""));
    assert!(shadow_v2_bytes.contains("\"schema_version\": 2"));
    assert!(shadow_v2_bytes.contains("\"XAUTUSDT\""));

    // 1) Universe count mismatch: 5-pair config vs persisted 6-pair state
    let five_u = Universe::parse_csv("BTCUSDT,ETHUSDT,SOLUSDT,LINKUSDT,AVAXUSDT")?;
    assert!(matches!(
        store.load_primary_state(&five_u),
        Err(InfraError::UniverseMismatch { .. })
    ));
    assert!(matches!(
        store.load_shadow_4h_state(&five_u),
        Err(InfraError::UniverseMismatch { .. })
    ));

    clock.set(hour_04_close + ONE_HOUR_MS + 1_000)?;
    let five_market = MockMarket::new(
        five_u.clone(),
        hour_04_close + ONE_HOUR_MS + 1_000,
        build_universe(5, 605, 5),
    );
    let five_uc = RunHourlyCycleUseCase::new(&clock, &five_market, &exec, &store, &notifier);
    assert!(matches!(
        five_uc.run(&paper_cfg(five_u)),
        Err(AppError::Infra(InfraError::UniverseMismatch { .. }))
    ));
    clock.set(hour_04_close + 1_000)?;

    // 2) Same 6 symbols in different order -> UniverseMismatch fail-closed
    let reordered_u = Universe::parse_csv("XAUTUSDT,BTCUSDT,ETHUSDT,SOLUSDT,LINKUSDT,AVAXUSDT")?;
    assert!(matches!(
        store.load_primary_state(&reordered_u),
        Err(InfraError::UniverseMismatch { .. })
    ));
    assert!(matches!(
        store.load_shadow_4h_state(&reordered_u),
        Err(InfraError::UniverseMismatch { .. })
    ));

    // 3) Corrupt vector length (e.g. last_sigma_ann truncated to 5 entries)
    let mut primary_json: serde_json::Value = serde_json::from_str(&primary_v2_bytes)?;
    if let Some(arr) = primary_json["last_sigma_ann"].as_array_mut() {
        arr.pop();
    }
    fs::write(
        store.primary_state_path(),
        serde_json::to_string_pretty(&primary_json)?,
    )?;
    assert!(matches!(
        store.load_primary_state(&default_u),
        Err(InfraError::CorruptStateLength {
            field: "last_sigma_ann",
            expected: 6,
            actual: 5,
            ..
        })
    ));

    let mut shadow_json: serde_json::Value = serde_json::from_str(&shadow_v2_bytes)?;
    if let Some(arr) = shadow_json["portfolio"]["positions"].as_array_mut() {
        arr.pop();
    }
    fs::write(
        store.shadow_4h_state_path(),
        serde_json::to_string_pretty(&shadow_json)?,
    )?;
    assert!(matches!(
        store.load_shadow_4h_state(&default_u),
        Err(InfraError::CorruptStateLength {
            field: "portfolio.positions",
            expected: 6,
            actual: 5,
            ..
        })
    ));

    // 4) Legacy v1.0.2 JSON (schema_version=1, no symbols field) -> SchemaVersionMismatch and untouched on disk!
    let v1_primary = r#"{
  "schema_version": 1,
  "mode": "paper",
  "last_processed_hour_ts_ms": 1730174400000,
  "last_daily_summary_day_utc": 20025,
  "last_order_ts_ms": 0,
  "portfolio": {
    "cash_1e8": 100000000000,
    "positions": [
      {"qty_1e8": 0, "cost_basis_usdt_1e8": 0},
      {"qty_1e8": 0, "cost_basis_usdt_1e8": 0},
      {"qty_1e8": 0, "cost_basis_usdt_1e8": 0},
      {"qty_1e8": 0, "cost_basis_usdt_1e8": 0},
      {"qty_1e8": 0, "cost_basis_usdt_1e8": 0}
    ],
    "total_deposits_1e8": 100000000000,
    "realized_pnl_gross_1e8": 0,
    "cum_sell_fees_usdt_1e8": 0,
    "cum_buy_fees_base_1e8": [0, 0, 0, 0, 0],
    "cum_buy_fees_usdt_est_1e8": 0,
    "twr_index_1e10": 10000000000,
    "twr_peak_1e10": 10000000000,
    "last_marked_equity_1e8": 100000000000,
    "halted": false
  },
  "last_target_weights": [0.0, 0.0, 0.0, 0.0, 0.0],
  "last_current_weights": [0.0, 0.0, 0.0, 0.0, 0.0],
  "last_sigma_ann": [0.2, 0.2, 0.2, 0.2, 0.2],
  "last_tsmom": [0.0, 0.0, 0.0, 0.0, 0.0],
  "last_prices_1e8": [10000000000, 10000000000, 10000000000, 10000000000, 10000000000],
  "last_drift_1e8": 0
}"#;
    fs::write(store.primary_state_path(), v1_primary)?;
    fs::write(store.shadow_4h_state_path(), r#"{"schema_version":1}"#)?;

    assert!(matches!(
        store.load_primary_state(&default_u),
        Err(InfraError::SchemaVersionMismatch {
            expected: 2,
            actual: 1,
            ..
        })
    ));
    assert!(matches!(
        store.load_shadow_4h_state(&default_u),
        Err(InfraError::SchemaVersionMismatch {
            expected: 2,
            actual: 1,
            ..
        })
    ));
    assert!(matches!(
        uc.run(&paper_cfg(default_u)),
        Err(AppError::Infra(InfraError::SchemaVersionMismatch {
            expected: 2,
            actual: 1,
            ..
        }))
    ));
    assert_eq!(fs::read_to_string(store.primary_state_path())?, v1_primary);

    Ok(())
}

#[test]
fn e2e_six_pair_universe_with_xaut_4h_boundary_live_reconciliation_and_order_link_id() -> TestResult
{
    let universe = Universe::default_universe()?;
    let n = universe.len();
    let dir = TempDirGuard::create("e2e_xaut_live")?;
    let store = FsStateStore::new(dir.path().join("state"), dir.path().join("logs"))?;
    let hour1_close = BASE_UTC_MIDNIGHT_MS + 604 * ONE_HOUR_MS; // exact 4h UTC boundary

    // Parse UNIFIED wallet JSON containing USDT + 0 XAUT initially
    let wallet_raw = r#"{"retCode":0,"retMsg":"OK","result":{"list":[{"accountType":"UNIFIED","coin":[{"coin":"USDT","walletBalance":"1000.00","locked":"0"}]}]},"time":1728000000000}"#;
    let initial_wallet =
        parse_wallet_balance_for_universe(wallet_raw, &universe, hour1_close + 1_000)?;
    assert_eq!(initial_wallet.coin_free_1e8.len(), 6);

    // Hour 1: only XAUTUSDT (idx 5) in uptrend; BTC..AVAX in mild downtrend
    let candles_h1: Vec<Vec<Candle>> = (0..n)
        .map(|i| {
            if i == 5 {
                build_uptrend_1h(604, 2400.0, 1.50)
            } else {
                build_uptrend_1h(604, 200.0, -0.05)
            }
        })
        .collect();

    let clock = MockClock::new(hour1_close + 1_000);
    let market = MockMarket::new(universe.clone(), hour1_close + 1_000, candles_h1);
    let exec = MockLiveExecutionPort::from_wallet(universe.clone(), initial_wallet);
    let notifier = SpyNotifier::default();
    let uc = RunHourlyCycleUseCase::new(&clock, &market, &exec, &store, &notifier);

    let live_cfg = CycleConfig {
        universe: universe.clone(),
        mode: ExecutionMode::LiveSpot,
        paper_initial_cash_1e8: Money1e8(100_000_000_000),
        live_execution_env_flag: true,
        live_ack_valid: true,
        max_clock_skew_ms: 2_000,
        recv_window_ms: 5_000,
    };

    let out1 = uc.run(&live_cfg)?;
    match out1 {
        CycleOutcome::Executed {
            closed_hour_ts_ms,
            orders_executed,
            drift_1e8,
            shadow_4h_updated,
            ..
        } => {
            assert_eq!(closed_hour_ts_ms, hour1_close);
            assert_eq!(orders_executed, 1);
            assert_eq!(drift_1e8, 0);
            assert!(shadow_4h_updated);
        }
        other => return Err(format!("expected Executed at hour1, got {other:?}").into()),
    }

    let orders_h1 = exec.submitted_orders()?;
    assert_eq!(orders_h1.len(), 1);
    assert_eq!(orders_h1[0].symbol.as_str(), "XAUTUSDT");
    assert_eq!(orders_h1[0].side, OrderSide::Buy);
    assert_eq!(
        orders_h1[0].order_link_id,
        format!("m2b_{hour1_close}_XAUTUSDT_buy")
    );

    let st_h1 = store
        .load_primary_state(&universe)?
        .ok_or("missing primary after h1")?;
    let sh_h1 = store
        .load_shadow_4h_state(&universe)?
        .ok_or("missing shadow after h1")?;
    assert!(st_h1.portfolio.positions[5].qty_1e8 > Qty1e8::ZERO);
    assert!(sh_h1.portfolio.positions[5].qty_1e8 > Qty1e8::ZERO);
    assert_eq!(st_h1.last_drift_1e8, 0);
    assert_eq!(sh_h1.last_drift_1e8, 0);

    // Hour 2 under DRAIN_MODE.lock: XAUT flips downtrend (exit Sell allowed), BTC turns uptrend (Buy blocked)
    exec.clear_submitted_orders()?;
    fs::write(store.state_dir().join("DRAIN_MODE.lock"), b"drain\n")?;
    let hour2_close = BASE_UTC_MIDNIGHT_MS + 605 * ONE_HOUR_MS;
    let candles_h2: Vec<Vec<Candle>> = (0..n)
        .map(|i| {
            if i == 0 {
                build_uptrend_1h(605, 100.0, 0.25)
            } else {
                build_uptrend_1h(605, 3400.0, -0.30)
            }
        })
        .collect();
    clock.set(hour2_close + 1_000)?;
    market.set_state(hour2_close + 1_000, candles_h2.clone())?;
    exec.set_wallet_ts(hour2_close + 1_000)?;

    let out2 = uc.run(&live_cfg)?;
    match out2 {
        CycleOutcome::Executed {
            orders_executed,
            drift_1e8,
            drain_mode,
            ..
        } => {
            assert_eq!(orders_executed, 1);
            assert_eq!(drift_1e8, 0);
            assert!(drain_mode);
        }
        other => return Err(format!("expected Executed at hour2, got {other:?}").into()),
    }
    let orders_h2 = exec.submitted_orders()?;
    assert_eq!(orders_h2.len(), 1);
    assert_eq!(orders_h2[0].symbol.as_str(), "XAUTUSDT");
    assert_eq!(orders_h2[0].side, OrderSide::Sell);
    assert_eq!(
        orders_h2[0].order_link_id,
        format!("m2b_{hour2_close}_XAUTUSDT_sell")
    );

    // Hour 3: Inject unrecorded +0.05 XAUT in wallet -> ReconciliationMismatch + HALT_LOCK
    let hour3_close = BASE_UTC_MIDNIGHT_MS + 606 * ONE_HOUR_MS;
    clock.set(hour3_close + 1_000)?;
    market.set_state(hour3_close + 1_000, build_universe(n, 606, 0))?;
    exec.set_wallet_ts(hour3_close + 1_000)?;
    exec.inject_coin_free(5, Qty1e8(5_000_000))?; // 0.05 XAUT > 0.001 precision

    let res3 = uc.run(&live_cfg);
    match res3 {
        Err(AppError::ReconciliationMismatch(msg)) => {
            assert!(msg.contains("XAUTUSDT"), "unexpected mismatch msg: {msg}");
        }
        other => return Err(format!("expected ReconciliationMismatch, got {other:?}").into()),
    }
    assert!(store.has_halt_lock()?);

    Ok(())
}

fn read_http_request(
    stream: &mut std::net::TcpStream,
) -> Result<(String, HashMap<String, String>), Box<dyn std::error::Error>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    let header_end;
    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Err("unexpected EOF reading HTTP request".into());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = pos + 4;
            break;
        }
    }

    let header_str = std::str::from_utf8(&buf[..header_end])?;
    let mut lines = header_str.lines();
    let request_line = lines.next().unwrap_or_default().to_string();
    let mut headers = HashMap::new();
    let mut content_length = 0usize;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let key_lower = k.trim().to_ascii_lowercase();
            let val = v.trim().to_string();
            if key_lower == "content-length" {
                content_length = val.parse::<usize>().unwrap_or(0);
            }
            headers.insert(key_lower, val);
        }
    }

    let mut body_len_have = buf.len() - header_end;
    while body_len_have < content_length {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        body_len_have += n;
    }

    Ok((request_line, headers))
}

#[test]
fn bybit_v5_v102_regression_transient_get_retries_and_retcode_before_result() -> TestResult {
    let universe = Universe::default_universe()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let base_url = format!("http://{addr}");

    let server_handle = thread::spawn(move || -> Result<(usize, usize, Vec<String>), String> {
        let mut get_count = 0usize;
        let mut post_count = 0usize;
        let mut get_signs = Vec::new();

        // Expect exactly 3 HTTP connections: 2 GET attempts (retry 10016 -> OK) + 1 POST attempt (no blind retry)
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().map_err(|e| e.to_string())?;
            let (req_line, headers) = read_http_request(&mut stream).map_err(|e| e.to_string())?;
            let body = if req_line.starts_with("GET ") {
                get_count += 1;
                if let Some(sign) = headers.get("x-bapi-sign") {
                    get_signs.push(sign.clone());
                }
                if get_count == 1 {
                    r#"{"retCode":10016,"retMsg":"Service temporarily unavailable","result":{},"retExtInfo":{},"time":1795618808000}"#
                } else {
                    r#"{"retCode":0,"retMsg":"OK","result":{"list":[{"accountType":"UNIFIED","coin":[{"coin":"USDT","walletBalance":"500.00","locked":"0"},{"coin":"XAUT","walletBalance":"0.25","locked":"0"}]}]},"time":1795618809000}"#
                }
            } else if req_line.starts_with("POST ") {
                post_count += 1;
                r#"{"retCode":10016,"retMsg":"Order create busy","result":{},"retExtInfo":{},"time":1795618810000}"#
            } else {
                return Err(format!("unexpected request line: {req_line}"));
            };

            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(resp.as_bytes())
                .map_err(|e| e.to_string())?;
            stream.flush().map_err(|e| e.to_string())?;
        }
        Ok((get_count, post_count, get_signs))
    });

    let client = BybitSpotV5Client::new(base_url, "TESTKEY", "TESTSECRET", 5_000);

    // 1) GET wallet-balance retries transient retCode=10016 (with empty result={}) and extracts XAUT
    let wallet = client.fetch_wallet_snapshot(&universe)?;
    assert_eq!(wallet.usdt_free_1e8, Money1e8(50_000_000_000));
    assert_eq!(wallet.coin_free_1e8[5], Qty1e8(25_000_000)); // 0.25 XAUT

    // 2) POST /v5/order/create with retCode=10016 MUST NOT blindly retry
    let xaut = universe.symbols()[5].clone();
    let req = OrderRequest {
        symbol: xaut,
        side: OrderSide::Buy,
        quote_spend_1e8: Money1e8(10_000_000_000),
        base_qty_1e8: Qty1e8(4_000_000),
        base_precision: Qty1e8(100_000),
        order_link_id: "m2b_1795618800000_XAUTUSDT_buy".to_string(),
        reference_price_1e8: Price1e8(250_000_000_000),
    };
    let post_res = client.execute_spot_order(&req);
    assert_eq!(
        post_res,
        Err(InfraError::ExchangeApi {
            code: 10016,
            msg: "Order create busy".to_string(),
        })
    );

    let (get_count, post_count, get_signs) = server_handle
        .join()
        .map_err(|_| "mock HTTP server thread panicked")?
        .map_err(|e| format!("mock HTTP server error: {e}"))?;
    assert_eq!(get_count, 2, "GET must retry once after transient 10016");
    assert_eq!(post_count, 1, "POST must never blindly retry");
    assert_eq!(get_signs.len(), 2);
    assert!(get_signs.iter().all(|s| s.len() == 64));

    Ok(())
}

#[test]
fn config_env_universe_parsing_defaults_and_fail_closed() -> TestResult {
    // 1) Missing -> default 6 pairs including XAUTUSDT
    let cfg_missing = AppConfig::from_env_map(&HashMap::new())?;
    assert_eq!(
        cfg_missing.universe.as_csv(),
        "BTCUSDT,ETHUSDT,SOLUSDT,LINKUSDT,AVAXUSDT,XAUTUSDT"
    );

    // 2) Normalized custom list
    let mut map_custom = HashMap::new();
    map_custom.insert(
        "M2B_UNIVERSE_SYMBOLS".to_string(),
        "  btcusdt , XAUTUSDT , ethusdt  ".to_string(),
    );
    let cfg_custom = AppConfig::from_env_map(&map_custom)?;
    assert_eq!(cfg_custom.universe.as_csv(), "BTCUSDT,XAUTUSDT,ETHUSDT");

    // 3) Invalid values rejected fail-closed
    for bad in [
        "BTCUSDT,",
        "BTCUSDT,BTCUSDT",
        "USDT",
        "XAUTUSD",
        "BTC-USDT",
        &(1..=21)
            .map(|i| format!("C{i:02}USDT"))
            .collect::<Vec<_>>()
            .join(","),
    ] {
        let mut map_bad = HashMap::new();
        map_bad.insert("M2B_UNIVERSE_SYMBOLS".to_string(), bad.to_string());
        assert!(
            matches!(
                AppConfig::from_env_map(&map_bad),
                Err(InfraError::Config(_))
            ),
            "expected InfraError::Config for '{bad}'"
        );
    }

    Ok(())
}

fn sorted_state_key_sets(
    raw_json: &str,
) -> Result<(Vec<String>, Vec<String>), Box<dyn std::error::Error>> {
    let val: serde_json::Value = serde_json::from_str(raw_json)?;
    let root_obj = val.as_object().ok_or("expected JSON root object")?;
    let mut root_keys: Vec<String> = root_obj.keys().cloned().collect();
    root_keys.sort();

    let portfolio_obj = root_obj
        .get("portfolio")
        .and_then(|v| v.as_object())
        .ok_or("expected JSON portfolio object")?;
    let mut portfolio_keys: Vec<String> = portfolio_obj.keys().cloned().collect();
    portfolio_keys.sort();

    Ok((root_keys, portfolio_keys))
}

#[test]
fn hot_load_v110_schema_v2_state_runs_1h_and_4h_and_liquidates_sub_2pct_positions() -> TestResult {
    let universe = Universe::default_universe()?;
    let n = universe.len();
    let dir = TempDirGuard::create("hot_load_v110_v2")?;
    let store = FsStateStore::new(dir.path().join("state"), dir.path().join("logs"))?;

    // Literal v1.1.0 primary_state.json (schema_version=2, 6 default pairs, LINK=1.602%, XAUT=1.153%)
    let v110_primary_json = r#"{
  "schema_version": 2,
  "symbols": ["BTCUSDT", "ETHUSDT", "SOLUSDT", "LINKUSDT", "AVAXUSDT", "XAUTUSDT"],
  "mode": "paper",
  "last_processed_hour_ts_ms": 1730170800000,
  "last_daily_summary_day_utc": 20025,
  "last_order_ts_ms": 0,
  "portfolio": {
    "cash_1e8": 103079268000,
    "positions": [
      {"qty_1e8": 0, "cost_basis_usdt_1e8": 0},
      {"qty_1e8": 0, "cost_basis_usdt_1e8": 0},
      {"qty_1e8": 0, "cost_basis_usdt_1e8": 0},
      {"qty_1e8": 10000000, "cost_basis_usdt_1e8": 1698100000},
      {"qty_1e8": 0, "cost_basis_usdt_1e8": 0},
      {"qty_1e8": 7200000, "cost_basis_usdt_1e8": 1222632000}
    ],
    "total_deposits_1e8": 106000000000,
    "realized_pnl_gross_1e8": 0,
    "cum_sell_fees_usdt_1e8": 0,
    "cum_buy_fees_base_1e8": [0, 0, 0, 0, 0, 0],
    "cum_buy_fees_usdt_est_1e8": 0,
    "twr_index_1e10": 10000000000,
    "twr_peak_1e10": 10000000000,
    "last_marked_equity_1e8": 106000000000,
    "halted": false
  },
  "last_target_weights": [0.0, 0.0, 0.0, 0.016, 0.0, 0.0115],
  "last_current_weights": [0.0, 0.0, 0.0, 0.016019811320754717, 0.0, 0.011534264150943396],
  "last_sigma_ann": [0.2, 0.2, 0.2, 0.2, 0.2, 0.2],
  "last_tsmom": [0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
  "last_prices_1e8": [16981000000, 16981000000, 16981000000, 16981000000, 16981000000, 16981000000],
  "last_drift_1e8": 0
}"#;

    // Literal v1.1.0 m2b_shadow_4h_state.json (schema_version=2, 6 default pairs, XAUT=1.153%)
    let v110_shadow_json = r#"{
  "schema_version": 2,
  "symbols": ["BTCUSDT", "ETHUSDT", "SOLUSDT", "LINKUSDT", "AVAXUSDT", "XAUTUSDT"],
  "last_processed_4h_close_ts_ms": 1730160000000,
  "portfolio": {
    "cash_1e8": 104777368000,
    "positions": [
      {"qty_1e8": 0, "cost_basis_usdt_1e8": 0},
      {"qty_1e8": 0, "cost_basis_usdt_1e8": 0},
      {"qty_1e8": 0, "cost_basis_usdt_1e8": 0},
      {"qty_1e8": 0, "cost_basis_usdt_1e8": 0},
      {"qty_1e8": 0, "cost_basis_usdt_1e8": 0},
      {"qty_1e8": 7200000, "cost_basis_usdt_1e8": 1222632000}
    ],
    "total_deposits_1e8": 106000000000,
    "realized_pnl_gross_1e8": 0,
    "cum_sell_fees_usdt_1e8": 0,
    "cum_buy_fees_base_1e8": [0, 0, 0, 0, 0, 0],
    "cum_buy_fees_usdt_est_1e8": 0,
    "twr_index_1e10": 10000000000,
    "twr_peak_1e10": 10000000000,
    "last_marked_equity_1e8": 106000000000,
    "halted": false
  },
  "last_target_weights": [0.0, 0.0, 0.0, 0.0, 0.0, 0.0115],
  "last_current_weights": [0.0, 0.0, 0.0, 0.0, 0.0, 0.011534264150943396],
  "last_sigma_ann": [0.2, 0.2, 0.2, 0.2, 0.2, 0.2],
  "last_tsmom": [0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
  "last_prices_1e8": [16981000000, 16981000000, 16981000000, 16981000000, 16981000000, 16981000000],
  "last_drift_1e8": 0
}"#;

    let primary_keys_v110 = sorted_state_key_sets(v110_primary_json)?;
    let shadow_keys_v110 = sorted_state_key_sets(v110_shadow_json)?;

    fs::write(store.primary_state_path(), v110_primary_json)?;
    fs::write(store.shadow_4h_state_path(), v110_shadow_json)?;

    let pre_primary = store
        .load_primary_state(&universe)?
        .ok_or("preloaded v1.1.0 primary_state.json missing")?;
    assert_eq!(
        pre_primary.last_processed_hour_ts_ms,
        BASE_UTC_MIDNIGHT_MS + 603 * ONE_HOUR_MS
    );
    let pre_shadow = store
        .load_shadow_4h_state(&universe)?
        .ok_or("preloaded v1.1.0 m2b_shadow_4h_state.json missing")?;
    assert_eq!(
        pre_shadow.last_processed_4h_close_ts_ms,
        BASE_UTC_MIDNIGHT_MS + 600 * ONE_HOUR_MS
    );

    // Cycle 1 at exact 4h UTC boundary (604h): mild downtrend -> target_weights = [0.0; 6]
    let hour_04_close = BASE_UTC_MIDNIGHT_MS + 604 * ONE_HOUR_MS;
    let clock = MockClock::new(hour_04_close + 1_000);
    let market = MockMarket::new(
        universe.clone(),
        hour_04_close + 1_000,
        build_universe(n, 604, 0),
    );
    let exec = SpyExecutionPort::default();
    let notifier = SpyNotifier::default();
    let uc = RunHourlyCycleUseCase::new(&clock, &market, &exec, &store, &notifier);

    let mut cfg = paper_cfg(universe.clone());
    cfg.paper_initial_cash_1e8 = Money1e8(106_000_000_000);

    let out_04 = uc.run(&cfg)?;
    match out_04 {
        CycleOutcome::Executed {
            closed_hour_ts_ms,
            orders_executed,
            drift_1e8,
            shadow_4h_updated,
            halted,
            drain_mode,
            ..
        } => {
            assert_eq!(closed_hour_ts_ms, hour_04_close);
            assert_eq!(orders_executed, 2);
            assert_eq!(drift_1e8, 0);
            assert!(shadow_4h_updated);
            assert!(!halted);
            assert!(!drain_mode);
        }
        other => return Err(format!("expected Executed at hour_04_close, got {other:?}").into()),
    }

    let reloaded_primary = store
        .load_primary_state(&universe)?
        .ok_or("reloaded primary_state.json missing")?;
    assert_eq!(reloaded_primary.schema_version, STATE_SCHEMA_VERSION);
    assert_eq!(reloaded_primary.schema_version, 2);
    assert_eq!(
        reloaded_primary.portfolio.positions[3].qty_1e8,
        Qty1e8::ZERO
    );
    assert_eq!(
        reloaded_primary.portfolio.positions[5].qty_1e8,
        Qty1e8::ZERO
    );
    assert_eq!(reloaded_primary.last_drift_1e8, 0);
    let primary_after_raw = fs::read_to_string(store.primary_state_path())?;
    assert_eq!(
        sorted_state_key_sets(&primary_after_raw)?,
        primary_keys_v110
    );

    let reloaded_shadow = store
        .load_shadow_4h_state(&universe)?
        .ok_or("reloaded m2b_shadow_4h_state.json missing")?;
    assert_eq!(reloaded_shadow.schema_version, STATE_SCHEMA_VERSION);
    assert_eq!(reloaded_shadow.schema_version, 2);
    assert_eq!(reloaded_shadow.last_processed_4h_close_ts_ms, hour_04_close);
    assert_eq!(reloaded_shadow.portfolio.positions[5].qty_1e8, Qty1e8::ZERO);
    assert_eq!(reloaded_shadow.last_drift_1e8, 0);
    let shadow_after_raw = fs::read_to_string(store.shadow_4h_state_path())?;
    assert_eq!(sorted_state_key_sets(&shadow_after_raw)?, shadow_keys_v110);

    let raw_ledger = fs::read_to_string(store.audit_ledger_path())?;
    let lines: Vec<&str> = raw_ledger.lines().collect();
    assert!(!lines.is_empty());
    for line in &lines {
        assert!(verify_audit_ledger_line(line)?);
        assert!(line.contains("\"drift_1e8\":0"));
    }

    // Cycle 2 at 605h: already flat with target_weights = [0.0; 6] -> 0 orders, 0 drift
    let hour_05_close = BASE_UTC_MIDNIGHT_MS + 605 * ONE_HOUR_MS;
    clock.set(hour_05_close + 1_000)?;
    market.set_state(hour_05_close + 1_000, build_universe(n, 605, 0))?;

    let out_05 = uc.run(&cfg)?;
    match out_05 {
        CycleOutcome::Executed {
            closed_hour_ts_ms,
            orders_executed,
            drift_1e8,
            shadow_4h_updated,
            halted,
            drain_mode,
            ..
        } => {
            assert_eq!(closed_hour_ts_ms, hour_05_close);
            assert_eq!(orders_executed, 0);
            assert_eq!(drift_1e8, 0);
            assert!(!shadow_4h_updated);
            assert!(!halted);
            assert!(!drain_mode);
        }
        other => return Err(format!("expected Executed at hour_05_close, got {other:?}").into()),
    }
    assert_eq!(exec.total_calls(), 0);

    Ok(())
}
