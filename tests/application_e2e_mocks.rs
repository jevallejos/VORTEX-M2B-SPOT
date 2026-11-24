use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use vortex_m2b_spot::application::cycle::{CycleConfig, CycleOutcome, RunHourlyCycleUseCase};
use vortex_m2b_spot::domain::error::{AppError, InfraError};
use vortex_m2b_spot::domain::ports::{
    ClockPort, MarketDataPort, NotifierPort, RunLockGuard, SpotExecutionPort, StateStorePort,
};
use vortex_m2b_spot::domain::types::{
    AuditEntry, Candle, EnginePersistedState, ExecutionMode, FillReport, InstrumentRules, Money1e8,
    OrderRequest, Price1e8, Qty1e8, Shadow4hPersistedState, Symbol, TickerSnapshot, WalletSnapshot,
    ONE_HOUR_MS, UNIVERSE_LEN,
};
use vortex_m2b_spot::infrastructure::bybit_spot_v5::sign_bybit_v5;
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
    server_time_ms: Mutex<i64>,
    candles_by_sym: Mutex<[Vec<Candle>; UNIVERSE_LEN]>,
}

impl MockMarket {
    fn new(server_time_ms: i64, candles_by_sym: [Vec<Candle>; UNIVERSE_LEN]) -> Self {
        Self {
            server_time_ms: Mutex::new(server_time_ms),
            candles_by_sym: Mutex::new(candles_by_sym),
        }
    }

    fn set_state(
        &self,
        server_time_ms: i64,
        candles_by_sym: [Vec<Candle>; UNIVERSE_LEN],
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
        symbols: &[Symbol; UNIVERSE_LEN],
    ) -> Result<[InstrumentRules; UNIVERSE_LEN], InfraError> {
        Ok(symbols.map(|sym| InstrumentRules {
            symbol: sym,
            base_precision: Qty1e8(100_000),      // 0.001
            quote_precision: Money1e8(1_000_000), // 0.01
            min_order_qty: Qty1e8(100_000),
            min_order_amt: Money1e8(500_000_000), // $5.00
            tick_size: Price1e8(1_000_000),
            is_trading: true,
        }))
    }

    fn fetch_klines_1h(&self, symbol: Symbol, _limit: usize) -> Result<Vec<Candle>, InfraError> {
        let guard = self
            .candles_by_sym
            .lock()
            .map_err(|_| InfraError::Io("market candles mutex poisoned".to_string()))?;
        Ok(guard[symbol.index()].clone())
    }

    fn fetch_tickers(
        &self,
        symbols: &[Symbol; UNIVERSE_LEN],
    ) -> Result<[TickerSnapshot; UNIVERSE_LEN], InfraError> {
        let server_ts = self.fetch_server_time_ms()?;
        let guard = self
            .candles_by_sym
            .lock()
            .map_err(|_| InfraError::Io("market candles mutex poisoned".to_string()))?;
        let mut out = Vec::with_capacity(UNIVERSE_LEN);
        for sym in symbols {
            let series = &guard[sym.index()];
            let last_closed_price = series
                .iter()
                .rev()
                .find(|c| c.open_ts_ms + ONE_HOUR_MS <= server_ts)
                .and_then(|c| Price1e8::from_f64_checked(c.close).ok())
                .unwrap_or(Price1e8(10_000_000_000));
            out.push(TickerSnapshot {
                symbol: *sym,
                bid1: last_closed_price,
                ask1: last_closed_price,
                last: last_closed_price,
            });
        }
        out.try_into()
            .map_err(|_| InfraError::Parse("unexpected ticker len".to_string()))
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
    fn fetch_wallet_snapshot(&self) -> Result<WalletSnapshot, InfraError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(InfraError::Config(
            "SpyExecutionPort::fetch_wallet_snapshot unexpected call".to_string(),
        ))
    }

    fn query_order_by_link_id(
        &self,
        _symbol: Symbol,
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

    fn load_primary_state(&self) -> Result<Option<EnginePersistedState>, InfraError> {
        self.primary
            .lock()
            .map(|g| g.clone())
            .map_err(|_| InfraError::Io("primary mutex poisoned".to_string()))
    }

    fn save_primary_state(&self, state: &EnginePersistedState) -> Result<(), InfraError> {
        let mut g = self
            .primary
            .lock()
            .map_err(|_| InfraError::Io("primary mutex poisoned".to_string()))?;
        *g = Some(state.clone());
        Ok(())
    }

    fn load_shadow_4h_state(&self) -> Result<Option<Shadow4hPersistedState>, InfraError> {
        self.shadow
            .lock()
            .map(|g| g.clone())
            .map_err(|_| InfraError::Io("shadow mutex poisoned".to_string()))
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
        let high = close + 0.5;
        let low = open - 0.3;
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

fn build_universe(count: usize, positive_count: usize) -> [Vec<Candle>; UNIVERSE_LEN] {
    std::array::from_fn(|i| {
        if i < positive_count {
            build_uptrend_1h(count, 100.0 + (i as f64) * 5.0, 0.15)
        } else {
            // Flat/mild downtrend -> zero target weight
            build_uptrend_1h(count, 200.0, -0.05)
        }
    })
}

fn paper_cfg() -> CycleConfig {
    CycleConfig {
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
    // 603 bars starting at 00:00 UTC -> bar 602 opens at 02:00 UTC, closes at 03:00 UTC (NOT a 4h boundary)
    let hour_03_close = BASE_UTC_MIDNIGHT_MS + 603 * ONE_HOUR_MS;
    let server_03 = hour_03_close + 2_000;
    let candles_03 = build_universe(603, 5);

    let clock = MockClock::new(server_03 + 50);
    let market = MockMarket::new(server_03, candles_03);
    let exec = SpyExecutionPort::default();
    let store = MemStateStore::default();
    let notifier = SpyNotifier::default();
    let uc = RunHourlyCycleUseCase::new(&clock, &market, &exec, &store, &notifier);
    let cfg = paper_cfg();

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
    assert!(store.load_primary_state()?.is_some());
    assert!(store.load_shadow_4h_state()?.is_none());
    assert_eq!(exec.total_calls(), 0);

    // Idempotency check on 03:00 UTC: running same closed hour twice -> AlreadyProcessed, 0 orders, 0 Telegram alerts
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

    // Hour 2: 04:00 UTC close (604 bars -> bar 603 opens at 03:00 UTC, closes at 04:00 UTC -> 4h boundary!)
    let hour_04_close = BASE_UTC_MIDNIGHT_MS + 604 * ONE_HOUR_MS;
    let server_04 = hour_04_close + 2_000;
    let candles_04 = build_universe(604, 5);
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
        .load_primary_state()?
        .ok_or("primary state missing after 04:00 UTC")?;
    assert_eq!(primary_04.last_processed_hour_ts_ms, hour_04_close);
    assert_eq!(primary_04.last_drift_1e8, 0);

    let shadow_04 = store
        .load_shadow_4h_state()?
        .ok_or("shadow 4h state missing after 04:00 UTC")?;
    assert_eq!(shadow_04.last_processed_4h_close_ts_ms, hour_04_close);
    assert_eq!(shadow_04.last_drift_1e8, 0);
    assert_eq!(exec.total_calls(), 0);

    Ok(())
}

#[test]
fn live_spot_double_safety_lock_and_lockfile_rejections() -> TestResult {
    let clock = MockClock::new(1_728_000_000_000);
    let market = MockMarket::new(1_728_000_000_000, build_universe(604, 5));
    let exec = SpyExecutionPort::default();
    let store = MemStateStore::default();
    let notifier = SpyNotifier::default();
    let uc = RunHourlyCycleUseCase::new(&clock, &market, &exec, &store, &notifier);

    // 1) Missing BYBIT_SPOT_LIVE_EXECUTION=true
    let res_no_env = uc.run(&CycleConfig {
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

    // 3) Active DRAIN_MODE.lock -> LiveSafetyLockDenied
    store.set_drain_lock(true)?;
    let res_drain = uc.run(&CycleConfig {
        mode: ExecutionMode::LiveSpot,
        paper_initial_cash_1e8: Money1e8(100_000_000_000),
        live_execution_env_flag: true,
        live_ack_valid: true,
        max_clock_skew_ms: 2_000,
        recv_window_ms: 5_000,
    });
    assert!(matches!(res_drain, Err(AppError::LiveSafetyLockDenied(_))));
    store.set_drain_lock(false)?;

    // 4) Active HALT_LOCK -> HaltLockActive
    store.write_halt_lock("manual test halt")?;
    let res_halt = uc.run(&CycleConfig {
        mode: ExecutionMode::LiveSpot,
        paper_initial_cash_1e8: Money1e8(100_000_000_000),
        live_execution_env_flag: true,
        live_ack_valid: true,
        max_clock_skew_ms: 2_000,
        recv_window_ms: 5_000,
    });
    assert!(matches!(res_halt, Err(AppError::HaltLockActive(_))));
    assert_eq!(exec.total_calls(), 0);

    Ok(())
}

#[test]
fn drain_mode_in_paper_blocks_new_buys_and_allows_exit_sells() -> TestResult {
    let dir = TempDirGuard::create("drain_paper")?;
    let store = FsStateStore::new(dir.path().join("state"), dir.path().join("logs"))?;
    let cfg = paper_cfg();

    // Hour 1: only BTCUSDT in uptrend -> buys BTC (~$400), leaves ~$600 cash
    let hour1_close = BASE_UTC_MIDNIGHT_MS + 603 * ONE_HOUR_MS;
    let clock = MockClock::new(hour1_close + 1_000);
    let market = MockMarket::new(hour1_close + 1_000, build_universe(603, 1));
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

    // Hour 2: 2 symbols (BTC + ETH) now uptrend (0.40 + 0.40 = 0.80 <= L_MAX, so BTC stays 0.40
    // and ETH wants a new 0.40 Buy). With DRAIN_MODE.lock active, the new Buy MUST be blocked!
    let hour2_close = BASE_UTC_MIDNIGHT_MS + 604 * ONE_HOUR_MS;
    clock.set(hour2_close + 1_000)?;
    market.set_state(hour2_close + 1_000, build_universe(604, 2))?;

    let out2 = uc.run(&cfg)?;
    match out2 {
        CycleOutcome::Executed {
            orders_executed,
            drift_1e8,
            ..
        } => {
            assert_eq!(orders_executed, 0, "DRAIN_MODE must block all new buys");
            assert_eq!(drift_1e8, 0);
        }
        other => return Err(format!("expected Executed at hour2, got {other:?}").into()),
    }
    let state_after_h2 = store
        .load_primary_state()?
        .ok_or("missing primary state after hour2")?;
    assert_eq!(state_after_h2.portfolio.positions[1].qty_1e8, Qty1e8::ZERO);

    // Hour 3: crash BTCUSDT sharply over recent bars while DRAIN_MODE.lock is still active ->
    // exit Sell MUST be permitted and executed!
    let hour3_close = BASE_UTC_MIDNIGHT_MS + 605 * ONE_HOUR_MS;
    let mut crash_universe = build_universe(605, 0);
    // Keep same opening base for BTC and drop last 200 hours sharply so 72h/168h TSMOM < 0
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
            ..
        } => {
            assert_eq!(orders_executed, 1, "DRAIN_MODE must permit exit Sell");
            assert_eq!(drift_1e8, 0);
        }
        other => return Err(format!("expected Executed at hour3, got {other:?}").into()),
    }

    let state_after = store
        .load_primary_state()?
        .ok_or("missing primary state after exit sell")?;
    assert_eq!(state_after.portfolio.positions[0].qty_1e8, Qty1e8::ZERO);
    assert_eq!(exec.total_calls(), 0);

    Ok(())
}

#[test]
fn telegram_notifier_failure_does_not_abort_cycle_or_persistence() -> TestResult {
    let dir = TempDirGuard::create("tg_resilience")?;
    let store = FsStateStore::new(dir.path().join("state"), dir.path().join("logs"))?;

    // Cross 4h UTC close (604 bars) so both order notification AND daily summary trigger
    let hour_04_close = BASE_UTC_MIDNIGHT_MS + 604 * ONE_HOUR_MS;
    let clock = MockClock::new(hour_04_close + 1_000);
    let market = MockMarket::new(hour_04_close + 1_000, build_universe(604, 5));
    let exec = SpyExecutionPort::default();
    let failing_notifier = FailingNotifier::new();

    let uc = RunHourlyCycleUseCase::new(&clock, &market, &exec, &store, &failing_notifier);
    let out = uc.run(&paper_cfg())?;

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
    assert!(store.load_primary_state()?.is_some());
    assert!(store.load_shadow_4h_state()?.is_some());
    assert!(store.primary_state_path().exists());
    assert!(store.shadow_4h_state_path().exists());

    Ok(())
}

#[test]
fn bybit_v5_hmac_sha256_known_test_vectors() -> TestResult {
    // Independently computed SHA-256 HMAC digests for canonical preimage:
    // timestamp_ms || api_key || recv_window_ms || payload
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

    // Empty credentials must fail closed
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
    let dir = TempDirGuard::create("fs_store")?;
    let state_dir = dir.path().join("state");
    let logs_dir = dir.path().join("logs");
    let store = FsStateStore::new(&state_dir, &logs_dir)?;

    let hour_04_close = BASE_UTC_MIDNIGHT_MS + 604 * ONE_HOUR_MS;
    let clock = MockClock::new(hour_04_close + 1_000);
    let market = MockMarket::new(hour_04_close + 1_000, build_universe(604, 5));
    let exec = SpyExecutionPort::default();
    let notifier = SpyNotifier::default();
    let uc = RunHourlyCycleUseCase::new(&clock, &market, &exec, &store, &notifier);

    let out = uc.run(&paper_cfg())?;
    assert!(matches!(
        out,
        CycleOutcome::Executed {
            shadow_4h_updated: true,
            drift_1e8: 0,
            ..
        }
    ));

    // Verify exact filenames: primary_state.json, m2b_shadow_4h_state.json, m2b_audit_ledger.jsonl
    assert!(state_dir.join("primary_state.json").exists());
    assert!(state_dir.join("m2b_shadow_4h_state.json").exists());
    let ledger_path = logs_dir.join("m2b_audit_ledger.jsonl");
    assert!(ledger_path.exists());

    let loaded_primary = store
        .load_primary_state()?
        .ok_or("primary_state.json reload returned None")?;
    assert_eq!(loaded_primary.last_processed_hour_ts_ms, hour_04_close);
    assert_eq!(loaded_primary.last_drift_1e8, 0);

    let loaded_shadow = store
        .load_shadow_4h_state()?
        .ok_or("m2b_shadow_4h_state.json reload returned None")?;
    assert_eq!(loaded_shadow.last_processed_4h_close_ts_ms, hour_04_close);
    assert_eq!(loaded_shadow.last_drift_1e8, 0);

    // Verify FNV-1a 64-bit seal on every audit line and tamper detection
    let raw_ledger = fs::read_to_string(&ledger_path)?;
    let lines: Vec<&str> = raw_ledger.lines().collect();
    assert!(!lines.is_empty(), "audit ledger must contain entries");
    for line in &lines {
        assert!(verify_audit_ledger_line(line)?);
    }

    let first = lines[0];
    let tampered = first.replace("\"drift_1e8\":0", "\"drift_1e8\":1");
    assert_ne!(tampered, first);
    assert!(!verify_audit_ledger_line(&tampered)?);

    // Exclusive run.lock contention check
    let lock_guard = store.acquire_run_lock()?;
    let second_lock = store.acquire_run_lock();
    assert!(matches!(second_lock, Err(InfraError::LockBusy(_))));

    let cycle_while_locked = uc.run(&paper_cfg());
    assert!(matches!(
        cycle_while_locked,
        Err(AppError::Infra(InfraError::LockBusy(_)))
    ));

    drop(lock_guard);
    let reacquired = store.acquire_run_lock()?;
    drop(reacquired);

    Ok(())
}
