use crate::application::reconcile::reconcile_live_wallet;
use crate::domain::accounting::{SpotPortfolioLedger, DEFAULT_SPOT_FEE_RATE};
use crate::domain::error::{AppError, DomainError, InfraError};
use crate::domain::ports::{
    ClockPort, MarketDataPort, NotifierPort, SpotExecutionPort, StateStorePort,
};
use crate::domain::strategy::{
    aggregate_1h_to_4h, compute_a0_4h_signals, compute_b2_01_signals, filter_strictly_closed_1h,
    is_4h_utc_close, plan_rebalance_orders, RebalancePlanInput, MIN_CLOSED_1H_CANDLES,
    MIN_CLOSED_4H_CANDLES,
};
use crate::domain::types::{
    make_order_link_id, mul_div_floor_nonneg, mul_div_round, AuditEntry, Candle, DomainEvent,
    EnginePersistedState, ExecutionMode, InstrumentRules, Money1e8, OrderRequest, OrderSide,
    PlannedOrder, PortfolioSnapshot, PositionState, Price1e8, Qty1e8, Rate1e10,
    Shadow4hPersistedState, Symbol, WalletSnapshot, ONE_HOUR_MS, SCALE_1E8, UNIVERSE_LEN,
};

const SCHEMA_VERSION: u32 = 1;
const KLINE_FETCH_LIMIT: usize = 700;
const PAPER_FEE_BPS: i128 = 10;
const PAPER_SLIPPAGE_BPS: i128 = 2;
const LIVE_BUY_CASH_CAP_NUM: i128 = 9_970;
const LIVE_BUY_CASH_CAP_DEN: i128 = 10_000;
const RECENT_ORDER_WINDOW_MS: i64 = 120_000;
const MS_PER_DAY: i64 = 86_400_000;

#[derive(Debug, Clone)]
pub struct CycleConfig {
    pub mode: ExecutionMode,
    pub paper_initial_cash_1e8: Money1e8,
    pub live_execution_env_flag: bool,
    pub live_ack_valid: bool,
    pub max_clock_skew_ms: i64,
    pub recv_window_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CycleOutcome {
    AlreadyProcessed {
        closed_hour_ts_ms: i64,
    },
    Executed {
        closed_hour_ts_ms: i64,
        orders_executed: usize,
        equity_1e8: Money1e8,
        drift_1e8: i128,
        shadow_4h_updated: bool,
        halted: bool,
        drain_mode: bool,
    },
}

pub struct RunHourlyCycleUseCase<'a> {
    clock: &'a dyn ClockPort,
    market: &'a dyn MarketDataPort,
    exec: &'a dyn SpotExecutionPort,
    store: &'a dyn StateStorePort,
    notifier: &'a dyn NotifierPort,
}

impl<'a> RunHourlyCycleUseCase<'a> {
    pub fn new(
        clock: &'a dyn ClockPort,
        market: &'a dyn MarketDataPort,
        exec: &'a dyn SpotExecutionPort,
        store: &'a dyn StateStorePort,
        notifier: &'a dyn NotifierPort,
    ) -> Self {
        Self {
            clock,
            market,
            exec,
            store,
            notifier,
        }
    }

    pub fn run(&self, cfg: &CycleConfig) -> Result<CycleOutcome, AppError> {
        let _run_lock = self.store.acquire_run_lock()?;

        if cfg.mode == ExecutionMode::LiveSpot {
            if !(cfg.live_execution_env_flag && cfg.live_ack_valid) {
                return Err(AppError::LiveSafetyLockDenied(
                    "BYBIT_SPOT_LIVE_EXECUTION=true and valid BYBIT_SPOT_ACK_LIVE required"
                        .to_string(),
                ));
            }
            if self.store.has_halt_lock()? {
                return Err(AppError::HaltLockActive("HALT_LOCK is active".to_string()));
            }
        }

        let server_time_ms = self.market.fetch_server_time_ms()?;
        let local_time_ms = self.clock.now_ms();
        let skew_ms = local_time_ms.saturating_sub(server_time_ms).abs();
        if skew_ms > cfg.max_clock_skew_ms {
            return Err(InfraError::ClockSkew {
                skew_ms,
                max_allowed_ms: cfg.max_clock_skew_ms,
            }
            .into());
        }

        let rules = self.market.fetch_instruments_info(&Symbol::ALL)?;
        let tickers = self.market.fetch_tickers(&Symbol::ALL)?;
        let mut prices = [Price1e8::ZERO; UNIVERSE_LEN];
        for (i, t) in tickers.iter().enumerate() {
            if t.last.raw() <= 0 {
                return Err(
                    DomainError::InvalidParameter("ticker last price must be positive").into(),
                );
            }
            prices[i] = t.last;
        }

        let mut closed_per_sym: Vec<Vec<Candle>> = Vec::with_capacity(UNIVERSE_LEN);
        for sym in Symbol::ALL {
            let raw = self.market.fetch_klines_1h(sym, KLINE_FETCH_LIMIT)?;
            let closed = filter_strictly_closed_1h(&raw, server_time_ms)?;
            closed_per_sym.push(closed);
        }

        let (aligned_1h, common_last_open_ts) =
            align_candles_to_common_latest(closed_per_sym, MIN_CLOSED_1H_CANDLES)?;
        let closed_hour_ts_ms = common_last_open_ts
            .checked_add(ONE_HOUR_MS)
            .ok_or(DomainError::ArithmeticOverflow)?;

        let primary_opt = self.store.load_primary_state()?;
        if let Some(ref prev_state) = primary_opt {
            if closed_hour_ts_ms <= prev_state.last_processed_hour_ts_ms {
                return Ok(CycleOutcome::AlreadyProcessed { closed_hour_ts_ms });
            }
        }

        let (mut ledger, mut last_daily_summary_day_utc, mut last_order_ts_ms, bootstrap_wallet) =
            match primary_opt {
                Some(prev) => (
                    SpotPortfolioLedger::from_snapshot(prev.portfolio)?,
                    prev.last_daily_summary_day_utc,
                    prev.last_order_ts_ms,
                    None,
                ),
                None => match cfg.mode {
                    ExecutionMode::Paper => (
                        SpotPortfolioLedger::new(cfg.paper_initial_cash_1e8)?,
                        -1i64,
                        0i64,
                        None,
                    ),
                    ExecutionMode::LiveSpot => {
                        let wallet = self.exec.fetch_wallet_snapshot()?;
                        let init_ledger = init_live_ledger_from_wallet(&wallet, &prices)?;
                        (init_ledger, -1i64, 0i64, Some(wallet))
                    }
                },
            };

        let drain_mode = self.store.has_drain_lock()?;
        let mut halt_lock = self.store.has_halt_lock()?;

        if cfg.mode == ExecutionMode::LiveSpot {
            let wallet = match bootstrap_wallet {
                Some(w) => w,
                None => self.exec.fetch_wallet_snapshot()?,
            };
            let had_recent_orders = last_order_ts_ms > 0
                && server_time_ms.saturating_sub(last_order_ts_ms) <= RECENT_ORDER_WINDOW_MS;
            let report =
                reconcile_live_wallet(&mut ledger, &wallet, &rules, &prices, had_recent_orders)?;

            if report.external_cash_delta_1e8.raw() != 0 {
                let delta = report.external_cash_delta_1e8.raw();
                let ev_type = if delta > 0 {
                    "EXTERNAL_DEPOSIT"
                } else {
                    "EXTERNAL_WITHDRAWAL"
                };
                let eq = ledger.equity_1e8(&prices)?;
                let drift = ledger.drift_mtm_1e8(&prices)?;
                self.store.append_audit_entry(&AuditEntry {
                    ts_ms: server_time_ms,
                    engine: "B2_01".to_string(),
                    event_type: ev_type.to_string(),
                    mode: cfg.mode.as_str().to_string(),
                    hour_ts_ms: closed_hour_ts_ms,
                    symbol: "USDT".to_string(),
                    side: String::new(),
                    order_link_id: String::new(),
                    qty_1e8: 0,
                    quote_1e8: delta,
                    price_1e8: SCALE_1E8,
                    fee_1e8: 0,
                    fee_in_base: false,
                    equity_1e8: eq.raw(),
                    drift_1e8: drift,
                    detail: format!(
                        "external cash adjustment {}",
                        report.external_cash_delta_1e8
                    ),
                })?;
                let _ = self.notifier.send_message(&format!(
                    "[VORTEX-M2B-SPOT {}] {} detected: {} USDT (equity={})",
                    cfg.mode, ev_type, report.external_cash_delta_1e8, eq
                ));
            }

            if !report.base_discrepancies.is_empty() {
                let mut parts = Vec::with_capacity(report.base_discrepancies.len());
                for d in &report.base_discrepancies {
                    parts.push(format!(
                        "{}: ledger={} wallet={} diff={}",
                        d.symbol, d.ledger_qty_1e8, d.wallet_qty_1e8, d.diff_1e8
                    ));
                }
                let reason = format!("reconciliation base mismatch: {}", parts.join("; "));
                self.store.write_halt_lock(&reason)?;
                let eq = ledger.equity_1e8(&prices)?;
                let drift = ledger.drift_mtm_1e8(&prices)?;
                let _ = self.store.append_audit_entry(&AuditEntry {
                    ts_ms: server_time_ms,
                    engine: "B2_01".to_string(),
                    event_type: "RECONCILIATION_MISMATCH".to_string(),
                    mode: cfg.mode.as_str().to_string(),
                    hour_ts_ms: closed_hour_ts_ms,
                    symbol: "UNIVERSE".to_string(),
                    side: String::new(),
                    order_link_id: String::new(),
                    qty_1e8: 0,
                    quote_1e8: 0,
                    price_1e8: 0,
                    fee_1e8: 0,
                    fee_in_base: false,
                    equity_1e8: eq.raw(),
                    drift_1e8: drift,
                    detail: reason.clone(),
                });
                let _ = self.notifier.send_message(&format!(
                    "[VORTEX-M2B-SPOT CRITICAL] HALT_LOCK engaged: {reason}"
                ));
                return Err(AppError::ReconciliationMismatch(reason));
            }
        }

        let was_halted = ledger.is_halted();
        let mtm_event = ledger.mark_to_market(&prices)?;
        if matches!(mtm_event, Some(DomainEvent::HardDrawdownHalt { .. }))
            || (!was_halted && ledger.is_halted())
        {
            self.handle_hard_drawdown_halt(
                &mut ledger,
                cfg.mode,
                server_time_ms,
                closed_hour_ts_ms,
                &prices,
                &mtm_event,
            )?;
            halt_lock = true;
        }

        let signals = compute_b2_01_signals(&aligned_1h)?;
        let pre_weights = ledger.current_weights(&prices)?;
        let allow_buys = !drain_mode && !halt_lock && !ledger.is_halted();

        let planned_orders = plan_rebalance_orders(&RebalancePlanInput {
            cash_1e8: ledger.cash_1e8(),
            positions: ledger.positions(),
            prices_1e8: &prices,
            rules: &rules,
            target_weights: &signals.target_weights,
            current_weights: &pre_weights,
            fee_rate: DEFAULT_SPOT_FEE_RATE,
            allow_buys,
        })?;

        let mut orders_executed = 0usize;
        match cfg.mode {
            ExecutionMode::Paper => {
                let (exec_cnt, audit_rows) = simulate_paper_orders(
                    &mut ledger,
                    &planned_orders,
                    &rules,
                    "B2_01",
                    cfg.mode,
                    server_time_ms,
                    closed_hour_ts_ms,
                    &prices,
                )?;
                orders_executed = exec_cnt;
                if exec_cnt > 0 {
                    last_order_ts_ms = server_time_ms;
                }
                for row in audit_rows {
                    self.store.append_audit_entry(&row)?;
                }
            }
            ExecutionMode::LiveSpot => {
                for ord in &planned_orders {
                    let order_link_id = make_order_link_id(closed_hour_ts_ms, ord.symbol, ord.side);
                    let existing = self
                        .exec
                        .query_order_by_link_id(ord.symbol, &order_link_id)?;
                    let fill = match existing {
                        Some(f) => f,
                        None => {
                            let quote_spend = if ord.side == OrderSide::Buy {
                                let max_spend = mul_div_floor_nonneg(
                                    ledger.cash_1e8().raw(),
                                    LIVE_BUY_CASH_CAP_NUM,
                                    LIVE_BUY_CASH_CAP_DEN,
                                )?;
                                let capped = Money1e8(ord.quote_spend_1e8.raw().min(max_spend));
                                if capped < rules[ord.symbol.index()].min_order_amt {
                                    continue;
                                }
                                capped
                            } else {
                                ord.quote_spend_1e8
                            };

                            let req = OrderRequest {
                                symbol: ord.symbol,
                                side: ord.side,
                                quote_spend_1e8: quote_spend,
                                base_qty_1e8: ord.base_qty_1e8,
                                base_precision: rules[ord.symbol.index()].base_precision,
                                order_link_id: order_link_id.clone(),
                                reference_price_1e8: ord.reference_price_1e8,
                            };
                            guard_live_order_request(
                                &req,
                                drain_mode,
                                halt_lock,
                                ledger.is_halted(),
                            )?;
                            self.exec.execute_spot_order(&req)?
                        }
                    };

                    apply_live_fill(
                        &mut ledger,
                        &fill,
                        rules[fill.symbol.index()].base_precision,
                    )?;
                    orders_executed += 1;
                    last_order_ts_ms = fill.filled_at_ms.max(server_time_ms);

                    let eq = ledger.equity_1e8(&prices)?;
                    let drift = ledger.drift_mtm_1e8(&prices)?;
                    self.store.append_audit_entry(&AuditEntry {
                        ts_ms: server_time_ms,
                        engine: "B2_01".to_string(),
                        event_type: "ORDER_FILL".to_string(),
                        mode: cfg.mode.as_str().to_string(),
                        hour_ts_ms: closed_hour_ts_ms,
                        symbol: fill.symbol.as_str().to_string(),
                        side: fill.side.as_str().to_string(),
                        order_link_id: fill.order_link_id.clone(),
                        qty_1e8: fill.exec_qty_base_1e8.raw(),
                        quote_1e8: fill.exec_quote_1e8.raw(),
                        price_1e8: fill.avg_price_1e8.raw(),
                        fee_1e8: fill.fee_1e8,
                        fee_in_base: fill.fee_in_base,
                        equity_1e8: eq.raw(),
                        drift_1e8: drift,
                        detail: format!("live order_id={}", fill.order_id),
                    })?;
                }
            }
        }

        if orders_executed > 0 {
            let was_halted_post = ledger.is_halted();
            let post_ev = ledger.mark_to_market(&prices)?;
            if matches!(post_ev, Some(DomainEvent::HardDrawdownHalt { .. }))
                || (!was_halted_post && ledger.is_halted())
            {
                self.handle_hard_drawdown_halt(
                    &mut ledger,
                    cfg.mode,
                    server_time_ms,
                    closed_hour_ts_ms,
                    &prices,
                    &post_ev,
                )?;
            }
        }

        ledger.assert_zero_drift(&prices)?;
        let equity_1e8 = ledger.equity_1e8(&prices)?;
        let drift_1e8 = ledger.drift_mtm_1e8(&prices)?;
        let post_weights = ledger.current_weights(&prices)?;

        let current_day_utc = closed_hour_ts_ms.div_euclid(MS_PER_DAY);
        let send_daily_summary = current_day_utc > last_daily_summary_day_utc;
        if send_daily_summary {
            last_daily_summary_day_utc = current_day_utc;
        }

        let persisted = EnginePersistedState {
            schema_version: SCHEMA_VERSION,
            mode: cfg.mode,
            last_processed_hour_ts_ms: closed_hour_ts_ms,
            last_daily_summary_day_utc,
            last_order_ts_ms,
            portfolio: ledger.snapshot(),
            last_target_weights: signals.target_weights,
            last_current_weights: post_weights,
            last_sigma_ann: signals.sigma_ann,
            last_tsmom: signals.tsmom,
            last_prices_1e8: prices,
            last_drift_1e8: drift_1e8,
        };
        self.store.save_primary_state(&persisted)?;

        let shadow_4h_updated = self.run_shadow_4h_if_due(
            cfg,
            &aligned_1h,
            &rules,
            common_last_open_ts,
            closed_hour_ts_ms,
            server_time_ms,
        )?;

        if orders_executed > 0 {
            let _ = self.notifier.send_message(&format!(
                "[VORTEX-M2B-SPOT {}] hour={} orders={} equity={} USDT drift={} halted={} drain_mode={}",
                cfg.mode,
                closed_hour_ts_ms,
                orders_executed,
                equity_1e8.format_fixed(2),
                drift_1e8,
                ledger.is_halted(),
                drain_mode
            ));
        }

        if send_daily_summary {
            let dd_pct = (ledger.current_drawdown_1e10()? as f64) / 1e8;
            let _ = self.notifier.send_message(&format!(
                "[VORTEX-M2B-SPOT DAILY {}] day_utc={} equity={} USDT cash={} USDT dd={:.2}% twr={:.4} shadow_4h={}",
                cfg.mode,
                current_day_utc,
                equity_1e8.format_fixed(2),
                ledger.cash_1e8().format_fixed(2),
                dd_pct,
                ledger.twr_index_1e10().to_f64(),
                shadow_4h_updated
            ));
        }

        Ok(CycleOutcome::Executed {
            closed_hour_ts_ms,
            orders_executed,
            equity_1e8,
            drift_1e8,
            shadow_4h_updated,
            halted: ledger.is_halted(),
            drain_mode,
        })
    }

    fn handle_hard_drawdown_halt(
        &self,
        ledger: &mut SpotPortfolioLedger,
        mode: ExecutionMode,
        server_time_ms: i64,
        closed_hour_ts_ms: i64,
        prices: &[Price1e8; UNIVERSE_LEN],
        mtm_event: &Option<DomainEvent>,
    ) -> Result<(), AppError> {
        ledger.set_halted(true);
        let reason = match mtm_event {
            Some(DomainEvent::HardDrawdownHalt {
                drawdown_1e10,
                equity_1e8,
                ..
            }) => format!("hard drawdown halt: dd_1e10={drawdown_1e10}, equity={equity_1e8}"),
            _ => "hard drawdown halt (>25% TWR drawdown)".to_string(),
        };
        self.store.write_halt_lock(&reason)?;
        let eq = ledger.equity_1e8(prices)?;
        let drift = ledger.drift_mtm_1e8(prices)?;
        self.store.append_audit_entry(&AuditEntry {
            ts_ms: server_time_ms,
            engine: "B2_01".to_string(),
            event_type: "HARD_DRAWDOWN_HALT".to_string(),
            mode: mode.as_str().to_string(),
            hour_ts_ms: closed_hour_ts_ms,
            symbol: "UNIVERSE".to_string(),
            side: String::new(),
            order_link_id: String::new(),
            qty_1e8: 0,
            quote_1e8: 0,
            price_1e8: 0,
            fee_1e8: 0,
            fee_in_base: false,
            equity_1e8: eq.raw(),
            drift_1e8: drift,
            detail: reason.clone(),
        })?;
        let _ = self
            .notifier
            .send_message(&format!("[VORTEX-M2B-SPOT CRITICAL {}] {reason}", mode));
        Ok(())
    }

    fn run_shadow_4h_if_due(
        &self,
        cfg: &CycleConfig,
        aligned_1h: &[Vec<Candle>; UNIVERSE_LEN],
        rules: &[InstrumentRules; UNIVERSE_LEN],
        common_last_open_ts: i64,
        closed_hour_ts_ms: i64,
        server_time_ms: i64,
    ) -> Result<bool, AppError> {
        if !is_4h_utc_close(common_last_open_ts) {
            return Ok(false);
        }

        let shadow_opt = self.store.load_shadow_4h_state()?;
        if let Some(ref prev_shadow) = shadow_opt {
            if closed_hour_ts_ms <= prev_shadow.last_processed_4h_close_ts_ms {
                return Ok(false);
            }
        }

        let mut bars_4h_vec: Vec<Vec<Candle>> = Vec::with_capacity(UNIVERSE_LEN);
        for sym_1h in aligned_1h {
            bars_4h_vec.push(aggregate_1h_to_4h(sym_1h)?);
        }
        let (aligned_4h, _) = align_candles_to_common_latest(bars_4h_vec, MIN_CLOSED_4H_CANDLES)?;

        let sig_4h = compute_a0_4h_signals(&aligned_4h)?;
        let mut shadow_prices = [Price1e8::ZERO; UNIVERSE_LEN];
        for (i, sym_bars) in aligned_4h.iter().enumerate() {
            let last_close =
                sym_bars
                    .last()
                    .map(|c| c.close)
                    .ok_or(DomainError::InsufficientHistory {
                        required: MIN_CLOSED_4H_CANDLES,
                        actual: 0,
                    })?;
            shadow_prices[i] = Price1e8::from_f64_checked(last_close)?;
        }

        let mut shadow_ledger = match shadow_opt {
            Some(prev) => SpotPortfolioLedger::from_snapshot(prev.portfolio)?,
            None => SpotPortfolioLedger::new(cfg.paper_initial_cash_1e8)?,
        };

        let _ = shadow_ledger.mark_to_market(&shadow_prices)?;
        let pre_weights = shadow_ledger.current_weights(&shadow_prices)?;
        let planned = plan_rebalance_orders(&RebalancePlanInput {
            cash_1e8: shadow_ledger.cash_1e8(),
            positions: shadow_ledger.positions(),
            prices_1e8: &shadow_prices,
            rules,
            target_weights: &sig_4h.target_weights,
            current_weights: &pre_weights,
            fee_rate: DEFAULT_SPOT_FEE_RATE,
            allow_buys: !shadow_ledger.is_halted(),
        })?;

        let (exec_cnt, audit_rows) = simulate_paper_orders(
            &mut shadow_ledger,
            &planned,
            rules,
            "A0_4H",
            ExecutionMode::Paper,
            server_time_ms,
            closed_hour_ts_ms,
            &shadow_prices,
        )?;
        for row in audit_rows {
            self.store.append_audit_entry(&row)?;
        }

        if exec_cnt > 0 {
            let _ = shadow_ledger.mark_to_market(&shadow_prices)?;
        }
        shadow_ledger.assert_zero_drift(&shadow_prices)?;

        let shadow_eq = shadow_ledger.equity_1e8(&shadow_prices)?;
        let shadow_drift = shadow_ledger.drift_mtm_1e8(&shadow_prices)?;
        let post_weights = shadow_ledger.current_weights(&shadow_prices)?;

        let shadow_state = Shadow4hPersistedState {
            schema_version: SCHEMA_VERSION,
            last_processed_4h_close_ts_ms: closed_hour_ts_ms,
            portfolio: shadow_ledger.snapshot(),
            last_target_weights: sig_4h.target_weights,
            last_current_weights: post_weights,
            last_sigma_ann: sig_4h.sigma_ann,
            last_tsmom: sig_4h.tsmom,
            last_prices_1e8: shadow_prices,
            last_drift_1e8: shadow_drift,
        };
        self.store.save_shadow_4h_state(&shadow_state)?;

        self.store.append_audit_entry(&AuditEntry {
            ts_ms: server_time_ms,
            engine: "A0_4H".to_string(),
            event_type: "SHADOW_4H_CYCLE".to_string(),
            mode: ExecutionMode::Paper.as_str().to_string(),
            hour_ts_ms: closed_hour_ts_ms,
            symbol: "UNIVERSE".to_string(),
            side: String::new(),
            order_link_id: String::new(),
            qty_1e8: 0,
            quote_1e8: 0,
            price_1e8: 0,
            fee_1e8: 0,
            fee_in_base: false,
            equity_1e8: shadow_eq.raw(),
            drift_1e8: shadow_drift,
            detail: format!("orders_simulated={exec_cnt}"),
        })?;

        Ok(true)
    }
}

fn init_live_ledger_from_wallet(
    wallet: &WalletSnapshot,
    prices: &[Price1e8; UNIVERSE_LEN],
) -> Result<SpotPortfolioLedger, DomainError> {
    let cash_1e8 = wallet.usdt_free_1e8.checked_add(wallet.usdt_locked_1e8)?;
    let mut positions = [PositionState::default(); UNIVERSE_LEN];
    let mut total_deposits = cash_1e8.raw();

    for i in 0..UNIVERSE_LEN {
        let qty = wallet.coin_free_1e8[i].checked_add(wallet.coin_locked_1e8[i])?;
        let cost = mul_div_floor_nonneg(qty.raw(), prices[i].raw(), SCALE_1E8)?;
        positions[i] = PositionState {
            qty_1e8: qty,
            cost_basis_usdt_1e8: Money1e8(cost),
        };
        total_deposits = total_deposits
            .checked_add(cost)
            .ok_or(DomainError::ArithmeticOverflow)?;
    }

    let snap = PortfolioSnapshot {
        cash_1e8,
        positions,
        total_deposits_1e8: Money1e8(total_deposits),
        realized_pnl_gross_1e8: Money1e8::ZERO,
        cum_sell_fees_usdt_1e8: Money1e8::ZERO,
        cum_buy_fees_base_1e8: [Qty1e8::ZERO; UNIVERSE_LEN],
        cum_buy_fees_usdt_est_1e8: Money1e8::ZERO,
        twr_index_1e10: Rate1e10::ONE,
        twr_peak_1e10: Rate1e10::ONE,
        last_marked_equity_1e8: Money1e8(total_deposits),
        halted: false,
    };

    let ledger = SpotPortfolioLedger::from_snapshot(snap)?;
    ledger.assert_zero_drift(prices)?;
    Ok(ledger)
}

fn align_candles_to_common_latest(
    mut per_sym: Vec<Vec<Candle>>,
    min_candles: usize,
) -> Result<([Vec<Candle>; UNIVERSE_LEN], i64), DomainError> {
    if per_sym.len() != UNIVERSE_LEN {
        return Err(DomainError::InvalidParameter(
            "expected exactly UNIVERSE_LEN candle series",
        ));
    }

    let mut min_last_ts: Option<i64> = None;
    for series in &per_sym {
        if series.len() < min_candles {
            return Err(DomainError::InsufficientHistory {
                required: min_candles,
                actual: series.len(),
            });
        }
        let last_ts = series.last().map(|c| c.open_ts_ms).unwrap_or(0);
        min_last_ts = Some(match min_last_ts {
            Some(cur) => cur.min(last_ts),
            None => last_ts,
        });
    }

    let target_last_ts = min_last_ts.ok_or(DomainError::InsufficientHistory {
        required: min_candles,
        actual: 0,
    })?;

    // Find the greatest timestamp <= target_last_ts present in all 5 series.
    let mut common_ts = None;
    for c in per_sym[0].iter().rev() {
        if c.open_ts_ms > target_last_ts {
            continue;
        }
        let present_in_all = per_sym[1..].iter().all(|s| {
            s.binary_search_by_key(&c.open_ts_ms, |x| x.open_ts_ms)
                .is_ok()
        });
        if present_in_all {
            common_ts = Some(c.open_ts_ms);
            break;
        }
    }

    let common_last_open_ts = common_ts.ok_or(DomainError::InsufficientHistory {
        required: min_candles,
        actual: 0,
    })?;

    for series in &mut per_sym {
        series.retain(|c| c.open_ts_ms <= common_last_open_ts);
        if series.len() < min_candles {
            return Err(DomainError::InsufficientHistory {
                required: min_candles,
                actual: series.len(),
            });
        }
    }

    let arr: [Vec<Candle>; UNIVERSE_LEN] = per_sym.try_into().map_err(|_| {
        DomainError::InvalidParameter("failed to convert candle vector to fixed universe array")
    })?;
    Ok((arr, common_last_open_ts))
}

#[allow(clippy::too_many_arguments)]
fn simulate_paper_orders(
    ledger: &mut SpotPortfolioLedger,
    planned: &[PlannedOrder],
    rules: &[InstrumentRules; UNIVERSE_LEN],
    engine_name: &str,
    mode: ExecutionMode,
    server_time_ms: i64,
    closed_hour_ts_ms: i64,
    prices: &[Price1e8; UNIVERSE_LEN],
) -> Result<(usize, Vec<AuditEntry>), DomainError> {
    let mut count = 0usize;
    let mut audits = Vec::with_capacity(planned.len());

    for ord in planned {
        let idx = ord.symbol.index();
        let base_prec = rules[idx].base_precision;
        let order_link_id = make_order_link_id(closed_hour_ts_ms, ord.symbol, ord.side);
        let ref_p = ord.reference_price_1e8.raw();

        match ord.side {
            OrderSide::Buy => {
                let exec_price = mul_div_round(ref_p, 10_000 + PAPER_SLIPPAGE_BPS, 10_000)?;
                if exec_price <= 0 {
                    continue;
                }
                let spend = Money1e8(ord.quote_spend_1e8.raw().min(ledger.cash_1e8().raw()));
                if spend < rules[idx].min_order_amt {
                    continue;
                }
                let gross_base = Qty1e8(mul_div_floor_nonneg(spend.raw(), SCALE_1E8, exec_price)?);
                if gross_base.raw() <= 0 {
                    continue;
                }
                let fee_base = Qty1e8(mul_div_floor_nonneg(
                    gross_base.raw(),
                    PAPER_FEE_BPS,
                    10_000,
                )?);
                let net_check = gross_base.checked_sub(fee_base)?.floor_to_step(base_prec)?;
                if net_check.raw() <= 0 {
                    continue;
                }

                ledger.apply_buy_fill(ord.symbol, spend, gross_base, fee_base, base_prec)?;
                count += 1;
                let eq = ledger.equity_1e8(prices)?;
                let drift = ledger.drift_mtm_1e8(prices)?;
                audits.push(AuditEntry {
                    ts_ms: server_time_ms,
                    engine: engine_name.to_string(),
                    event_type: "ORDER_FILL".to_string(),
                    mode: mode.as_str().to_string(),
                    hour_ts_ms: closed_hour_ts_ms,
                    symbol: ord.symbol.as_str().to_string(),
                    side: ord.side.as_str().to_string(),
                    order_link_id,
                    qty_1e8: gross_base.raw(),
                    quote_1e8: spend.raw(),
                    price_1e8: exec_price,
                    fee_1e8: fee_base.raw(),
                    fee_in_base: true,
                    equity_1e8: eq.raw(),
                    drift_1e8: drift,
                    detail: "paper simulated buy".to_string(),
                });
            }
            OrderSide::Sell => {
                let exec_price = mul_div_round(ref_p, 10_000 - PAPER_SLIPPAGE_BPS, 10_000)?;
                if exec_price <= 0 {
                    continue;
                }
                let sell_qty = ord
                    .base_qty_1e8
                    .min(ledger.position(ord.symbol).qty_1e8)
                    .floor_to_step(base_prec)?;
                if sell_qty < rules[idx].min_order_qty {
                    continue;
                }
                let gross_quote =
                    Money1e8(mul_div_floor_nonneg(sell_qty.raw(), exec_price, SCALE_1E8)?);
                let fee_usdt = Money1e8(mul_div_round(gross_quote.raw(), PAPER_FEE_BPS, 10_000)?);

                ledger.apply_sell_fill(ord.symbol, sell_qty, gross_quote, fee_usdt)?;
                count += 1;
                let eq = ledger.equity_1e8(prices)?;
                let drift = ledger.drift_mtm_1e8(prices)?;
                audits.push(AuditEntry {
                    ts_ms: server_time_ms,
                    engine: engine_name.to_string(),
                    event_type: "ORDER_FILL".to_string(),
                    mode: mode.as_str().to_string(),
                    hour_ts_ms: closed_hour_ts_ms,
                    symbol: ord.symbol.as_str().to_string(),
                    side: ord.side.as_str().to_string(),
                    order_link_id,
                    qty_1e8: sell_qty.raw(),
                    quote_1e8: gross_quote.raw(),
                    price_1e8: exec_price,
                    fee_1e8: fee_usdt.raw(),
                    fee_in_base: false,
                    equity_1e8: eq.raw(),
                    drift_1e8: drift,
                    detail: "paper simulated sell".to_string(),
                });
            }
        }
    }

    Ok((count, audits))
}

fn guard_live_order_request(
    req: &OrderRequest,
    drain_mode: bool,
    halt_lock: bool,
    ledger_halted: bool,
) -> Result<(), AppError> {
    if req.side == OrderSide::Buy && (drain_mode || halt_lock || ledger_halted) {
        return Err(AppError::LiveSafetyLockDenied(
            "buy order rejected: drain_mode or halt is active".to_string(),
        ));
    }
    Ok(())
}

fn apply_live_fill(
    ledger: &mut SpotPortfolioLedger,
    fill: &crate::domain::types::FillReport,
    base_precision: Qty1e8,
) -> Result<(), DomainError> {
    match fill.side {
        OrderSide::Buy => {
            let fee_base = if fill.fee_in_base {
                Qty1e8(fill.fee_1e8)
            } else {
                Qty1e8(mul_div_floor_nonneg(
                    fill.fee_1e8,
                    SCALE_1E8,
                    fill.avg_price_1e8.raw(),
                )?)
            };
            ledger.apply_buy_fill(
                fill.symbol,
                fill.exec_quote_1e8,
                fill.exec_qty_base_1e8,
                fee_base,
                base_precision,
            )
        }
        OrderSide::Sell => {
            let fee_usdt = if !fill.fee_in_base {
                Money1e8(fill.fee_1e8)
            } else {
                Money1e8(mul_div_round(
                    fill.fee_1e8,
                    fill.avg_price_1e8.raw(),
                    SCALE_1E8,
                )?)
            };
            ledger.apply_sell_fill(
                fill.symbol,
                fill.exec_qty_base_1e8,
                fill.exec_quote_1e8,
                fee_usdt,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::domain::ports::RunLockGuard;
    use crate::domain::types::TickerSnapshot;
    use crate::infrastructure::bybit_spot_v5::PaperNoopExecutionPort;

    struct FixedClock(i64);
    impl ClockPort for FixedClock {
        fn now_ms(&self) -> i64 {
            self.0
        }
    }

    struct MockMarket {
        server_time_ms: i64,
        candles: Vec<Candle>,
    }
    impl MarketDataPort for MockMarket {
        fn fetch_server_time_ms(&self) -> Result<i64, InfraError> {
            Ok(self.server_time_ms)
        }
        fn fetch_instruments_info(
            &self,
            symbols: &[Symbol; UNIVERSE_LEN],
        ) -> Result<[InstrumentRules; UNIVERSE_LEN], InfraError> {
            Ok(symbols.map(|sym| InstrumentRules {
                symbol: sym,
                base_precision: Qty1e8(100_000),
                quote_precision: Money1e8(1_000_000),
                min_order_qty: Qty1e8(100_000),
                min_order_amt: Money1e8(500_000_000),
                tick_size: Price1e8(1_000_000),
                is_trading: true,
            }))
        }
        fn fetch_klines_1h(
            &self,
            _symbol: Symbol,
            _limit: usize,
        ) -> Result<Vec<Candle>, InfraError> {
            Ok(self.candles.clone())
        }
        fn fetch_tickers(
            &self,
            symbols: &[Symbol; UNIVERSE_LEN],
        ) -> Result<[TickerSnapshot; UNIVERSE_LEN], InfraError> {
            let last_p = self
                .candles
                .last()
                .and_then(|c| Price1e8::from_f64_checked(c.close).ok())
                .unwrap_or(Price1e8(10_000_000_000));
            Ok(symbols.map(|sym| TickerSnapshot {
                symbol: sym,
                bid1: last_p,
                ask1: last_p,
                last: last_p,
            }))
        }
    }

    struct DummyLock;
    impl RunLockGuard for DummyLock {}

    #[derive(Default)]
    struct MemStore {
        primary: Mutex<Option<EnginePersistedState>>,
        shadow: Mutex<Option<Shadow4hPersistedState>>,
        audits: Mutex<Vec<AuditEntry>>,
        halt_reason: Mutex<Option<String>>,
    }
    impl StateStorePort for MemStore {
        fn acquire_run_lock(&self) -> Result<Box<dyn RunLockGuard>, InfraError> {
            Ok(Box::new(DummyLock))
        }
        fn has_drain_lock(&self) -> Result<bool, InfraError> {
            Ok(false)
        }
        fn has_halt_lock(&self) -> Result<bool, InfraError> {
            Ok(self
                .halt_reason
                .lock()
                .map(|g| g.is_some())
                .unwrap_or(false))
        }
        fn write_halt_lock(&self, reason: &str) -> Result<(), InfraError> {
            if let Ok(mut g) = self.halt_reason.lock() {
                *g = Some(reason.to_string());
            }
            Ok(())
        }
        fn load_primary_state(&self) -> Result<Option<EnginePersistedState>, InfraError> {
            Ok(self.primary.lock().ok().and_then(|g| g.clone()))
        }
        fn save_primary_state(&self, state: &EnginePersistedState) -> Result<(), InfraError> {
            if let Ok(mut g) = self.primary.lock() {
                *g = Some(state.clone());
            }
            Ok(())
        }
        fn load_shadow_4h_state(&self) -> Result<Option<Shadow4hPersistedState>, InfraError> {
            Ok(self.shadow.lock().ok().and_then(|g| g.clone()))
        }
        fn save_shadow_4h_state(&self, state: &Shadow4hPersistedState) -> Result<(), InfraError> {
            if let Ok(mut g) = self.shadow.lock() {
                *g = Some(state.clone());
            }
            Ok(())
        }
        fn append_audit_entry(&self, entry: &AuditEntry) -> Result<(), InfraError> {
            if let Ok(mut g) = self.audits.lock() {
                g.push(entry.clone());
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct MemNotifier {
        msgs: Mutex<Vec<String>>,
    }
    impl NotifierPort for MemNotifier {
        fn send_message(&self, text: &str) -> Result<(), InfraError> {
            if let Ok(mut g) = self.msgs.lock() {
                g.push(text.to_string());
            }
            Ok(())
        }
    }

    #[test]
    fn paper_cycle_executes_idempotently_and_updates_shadow_4h() -> Result<(), AppError> {
        let mut candles = Vec::with_capacity(620);
        for k in 0..620 {
            let p = 100.0 + (k as f64) * 0.1;
            candles.push(Candle {
                open_ts_ms: (k as i64) * ONE_HOUR_MS,
                open: p,
                high: p + 1.0,
                low: p - 0.5,
                close: p + 0.5,
                volume: 10.0,
                turnover: 1000.0,
            });
        }
        let server_ms = 620 * ONE_HOUR_MS + 5_000;
        let clock = FixedClock(server_ms + 100);
        let market = MockMarket {
            server_time_ms: server_ms,
            candles,
        };
        let exec = PaperNoopExecutionPort;
        let store = MemStore::default();
        let notifier = MemNotifier::default();

        let uc = RunHourlyCycleUseCase::new(&clock, &market, &exec, &store, &notifier);
        let cfg = CycleConfig {
            mode: ExecutionMode::Paper,
            paper_initial_cash_1e8: Money1e8(100_000_000_000),
            live_execution_env_flag: false,
            live_ack_valid: false,
            max_clock_skew_ms: 2_000,
            recv_window_ms: 5_000,
        };

        let first = uc.run(&cfg)?;
        assert!(
            matches!(first, CycleOutcome::Executed { .. }),
            "expected Executed, got {first:?}"
        );
        if let CycleOutcome::Executed {
            closed_hour_ts_ms,
            orders_executed,
            drift_1e8,
            shadow_4h_updated,
            halted,
            drain_mode,
            ..
        } = first
        {
            assert_eq!(closed_hour_ts_ms, 620 * ONE_HOUR_MS);
            assert!(orders_executed > 0);
            assert_eq!(drift_1e8, 0);
            assert!(shadow_4h_updated);
            assert!(!halted);
            assert!(!drain_mode);
        }

        let msg_count_after_first = notifier.msgs.lock().map(|g| g.len()).unwrap_or(0);
        assert!(msg_count_after_first >= 1);

        let second = uc.run(&cfg)?;
        assert_eq!(
            second,
            CycleOutcome::AlreadyProcessed {
                closed_hour_ts_ms: 620 * ONE_HOUR_MS
            }
        );
        let msg_count_after_second = notifier.msgs.lock().map(|g| g.len()).unwrap_or(0);
        assert_eq!(msg_count_after_second, msg_count_after_first);
        Ok(())
    }

    #[test]
    fn live_spot_double_safety_lock_denies_without_ack() {
        let clock = FixedClock(1_700_000_000_000);
        let market = MockMarket {
            server_time_ms: 1_700_000_000_000,
            candles: Vec::new(),
        };
        let exec = PaperNoopExecutionPort;
        let store = MemStore::default();
        let notifier = MemNotifier::default();
        let uc = RunHourlyCycleUseCase::new(&clock, &market, &exec, &store, &notifier);

        let cfg = CycleConfig {
            mode: ExecutionMode::LiveSpot,
            paper_initial_cash_1e8: Money1e8(100_000_000_000),
            live_execution_env_flag: true,
            live_ack_valid: false,
            max_clock_skew_ms: 2_000,
            recv_window_ms: 5_000,
        };

        let res = uc.run(&cfg);
        assert!(matches!(res, Err(AppError::LiveSafetyLockDenied(_))));
    }

    #[test]
    fn defensive_buy_guard_blocks_buys_under_drain_or_halt_and_permits_sells() {
        let buy_req = OrderRequest {
            symbol: Symbol::BtcUsdt,
            side: OrderSide::Buy,
            quote_spend_1e8: Money1e8(10_000_000_000),
            base_qty_1e8: Qty1e8(10_000_000),
            base_precision: Qty1e8(100_000),
            order_link_id: "m2b_1700000000000_BTCUSDT_buy".to_string(),
            reference_price_1e8: Price1e8(100_000_000_000),
        };
        let sell_req = OrderRequest {
            symbol: Symbol::BtcUsdt,
            side: OrderSide::Sell,
            quote_spend_1e8: Money1e8(10_000_000_000),
            base_qty_1e8: Qty1e8(10_000_000),
            base_precision: Qty1e8(100_000),
            order_link_id: "m2b_1700000000000_BTCUSDT_sell".to_string(),
            reference_price_1e8: Price1e8(100_000_000_000),
        };

        assert!(guard_live_order_request(&buy_req, false, false, false).is_ok());
        assert!(matches!(
            guard_live_order_request(&buy_req, true, false, false),
            Err(AppError::LiveSafetyLockDenied(_))
        ));
        assert!(matches!(
            guard_live_order_request(&buy_req, false, true, false),
            Err(AppError::LiveSafetyLockDenied(_))
        ));
        assert!(matches!(
            guard_live_order_request(&buy_req, false, false, true),
            Err(AppError::LiveSafetyLockDenied(_))
        ));

        assert!(guard_live_order_request(&sell_req, false, false, false).is_ok());
        assert!(guard_live_order_request(&sell_req, true, false, false).is_ok());
        assert!(guard_live_order_request(&sell_req, false, true, false).is_ok());
        assert!(guard_live_order_request(&sell_req, false, false, true).is_ok());
    }

    #[test]
    fn live_spot_halt_lock_rejects_cycle_fail_closed() {
        let clock = FixedClock(1_700_000_000_000);
        let market = MockMarket {
            server_time_ms: 1_700_000_000_000,
            candles: Vec::new(),
        };
        let exec = PaperNoopExecutionPort;
        let store = MemStore::default();
        assert!(store.write_halt_lock("manual halt test").is_ok());
        let notifier = MemNotifier::default();
        let uc = RunHourlyCycleUseCase::new(&clock, &market, &exec, &store, &notifier);

        let cfg = CycleConfig {
            mode: ExecutionMode::LiveSpot,
            paper_initial_cash_1e8: Money1e8(100_000_000_000),
            live_execution_env_flag: true,
            live_ack_valid: true,
            max_clock_skew_ms: 2_000,
            recv_window_ms: 5_000,
        };

        let res = uc.run(&cfg);
        assert!(matches!(res, Err(AppError::HaltLockActive(_))));
    }
}
