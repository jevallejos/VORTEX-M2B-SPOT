use vortex_m2b_spot::domain::accounting::{SpotPortfolioLedger, DEFAULT_SPOT_FEE_RATE};
use vortex_m2b_spot::domain::error::DomainError;
use vortex_m2b_spot::domain::indicators::{
    garman_klass_ewma_vol, tsmom_multi_horizon, tsmom_multi_horizon_at, A0_4H_BARS_PER_DAY,
    B2_01_BARS_PER_DAY, B2_01_LOOKBACKS, B2_01_N_EFF, SIGMA_BAR_FLOOR,
};
use vortex_m2b_spot::domain::strategy::{
    aggregate_1h_to_4h, compute_b2_01_signals, filter_strictly_closed_1h, is_4h_utc_close,
    plan_rebalance_orders, RebalancePlanInput,
};
use vortex_m2b_spot::domain::types::{
    Candle, DomainEvent, InstrumentRules, Money1e8, OrderSide, PositionState, Price1e8, Qty1e8,
    Symbol, FOUR_HOURS_MS, ONE_HOUR_MS, SCALE_1E8, UNIVERSE_LEN,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn mk_candle(
    open_ts_ms: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    turnover: f64,
) -> Candle {
    Candle {
        open_ts_ms,
        open,
        high,
        low,
        close,
        volume,
        turnover,
    }
}

fn mk_rule(symbol: Symbol, base_step: i128, min_qty: i128, min_amt: i128) -> InstrumentRules {
    InstrumentRules {
        symbol,
        base_precision: Qty1e8(base_step),
        quote_precision: Money1e8(1_000_000),
        min_order_qty: Qty1e8(min_qty),
        min_order_amt: Money1e8(min_amt),
        tick_size: Price1e8(1_000_000),
        is_trading: true,
    }
}

fn default_rules() -> [InstrumentRules; UNIVERSE_LEN] {
    Symbol::ALL.map(|sym| mk_rule(sym, 1_000_000, 1_000_000, 500_000_000))
}

fn build_synthetic_1h_series(len: usize, seed_offset: f64) -> Vec<Candle> {
    let mut out = Vec::with_capacity(len);
    let base_ts = 1_728_000_000_000i64; // Exact UTC midnight (divisible by 4h and 1h)
    for k in 0..len {
        let kf = k as f64;
        let open =
            100.0 + 0.04 * kf + 3.5 * (kf * 0.09 + seed_offset).sin() + 1.2 * (kf * 0.021).cos();
        let close = open * (1.0 + 0.0025 * (kf * 0.17 + seed_offset).cos());
        let high = open.max(close) * (1.0 + 0.006 + 0.002 * (kf * 0.31).sin().abs());
        let low = open.min(close) * (1.0 - 0.005 - 0.002 * (kf * 0.27).cos().abs());
        let vol = 15.0 + (kf % 7.0);
        out.push(mk_candle(
            base_ts + (k as i64) * ONE_HOUR_MS,
            open,
            high,
            low,
            close,
            vol,
            vol * close,
        ));
    }
    out
}

#[test]
fn garman_klass_ewma_closed_form_floor_nan_and_annualization() -> TestResult {
    // 1) Normal + flat bars closed-form check
    let bars = vec![
        mk_candle(0, 100.0, 100.0, 100.0, 100.0, 1.0, 100.0), // flat bar -> variance 0, floor 1e-4
        mk_candle(ONE_HOUR_MS, 100.0, 104.0, 98.0, 102.0, 2.0, 204.0),
        mk_candle(2 * ONE_HOUR_MS, 102.0, 105.0, 101.0, 103.5, 3.0, 310.5),
        mk_candle(3 * ONE_HOUR_MS, 103.5, 103.5, 103.5, 103.5, 0.5, 51.75),
        mk_candle(4 * ONE_HOUR_MS, 103.5, 107.0, 99.5, 100.5, 4.0, 402.0),
    ];

    let n_eff = 10usize;
    let vols_1h = garman_klass_ewma_vol(&bars, n_eff, B2_01_BARS_PER_DAY)?;
    let vols_4h = garman_klass_ewma_vol(&bars, n_eff, A0_4H_BARS_PER_DAY)?;
    assert_eq!(vols_1h.len(), bars.len());
    assert_eq!(vols_4h.len(), bars.len());

    // Independent closed-form computation
    let c_gk = 2.0 * std::f64::consts::LN_2 - 1.0;
    let alpha = 2.0 / ((n_eff as f64) + 1.0);
    let sqrt_8760 = 8760.0f64.sqrt();
    let sqrt_2190 = 2190.0f64.sqrt();

    let mut ewma = 0.0f64;
    for (i, b) in bars.iter().enumerate() {
        let hl = (b.high / b.low).ln();
        let co = (b.close / b.open).ln();
        let v_t = (0.5 * hl * hl - c_gk * co * co).max(0.0);
        ewma = if i == 0 {
            v_t
        } else {
            alpha * v_t + (1.0 - alpha) * ewma
        };
        let exp_sigma_bar = ewma.max(0.0).sqrt().max(1e-4);
        let exp_ann_1h = exp_sigma_bar * sqrt_8760;
        let exp_ann_4h = exp_sigma_bar * sqrt_2190;

        assert!(
            (vols_1h[i].sigma_bar - exp_sigma_bar).abs() < 1e-14,
            "sigma_bar mismatch at {i}"
        );
        assert!(
            (vols_1h[i].sigma_ann - exp_ann_1h).abs() < 1e-12,
            "sigma_ann 1h mismatch at {i}"
        );
        assert!(
            (vols_4h[i].sigma_ann - exp_ann_4h).abs() < 1e-12,
            "sigma_ann 4h mismatch at {i}"
        );
        assert!(
            ((vols_1h[i].sigma_ann / vols_4h[i].sigma_ann) - 2.0).abs() < 1e-13,
            "sqrt(8760)/sqrt(2190) ratio must be 2.0"
        );
    }

    // Flat bar 0 must hit exact floor 1e-4
    assert!((vols_1h[0].sigma_bar - SIGMA_BAR_FLOOR).abs() < 1e-15);

    // 2) NaN / Inf / invalid OHLC rejections
    let bad_candles = [
        mk_candle(0, f64::NAN, 101.0, 99.0, 100.0, 1.0, 100.0),
        mk_candle(0, 100.0, f64::INFINITY, 99.0, 100.0, 1.0, 100.0),
        mk_candle(0, 100.0, 101.0, f64::NEG_INFINITY, 100.0, 1.0, 100.0),
        mk_candle(0, 100.0, 101.0, 99.0, f64::NAN, 1.0, 100.0),
        mk_candle(0, 100.0, 101.0, 99.0, 100.0, f64::NAN, 100.0),
        mk_candle(0, 100.0, 101.0, 99.0, 100.0, 1.0, f64::INFINITY),
        mk_candle(0, 100.0, 99.0, 101.0, 100.0, 1.0, 100.0),
    ];
    for bad in bad_candles {
        let res = garman_klass_ewma_vol(&[bad], n_eff, B2_01_BARS_PER_DAY);
        assert!(matches!(res, Err(DomainError::InvalidCandle(_))));
    }

    Ok(())
}

#[test]
fn tsmom_1h_72_168_504_closed_form_and_1h_to_4h_aggregator() -> TestResult {
    let series = build_synthetic_1h_series(540, 0.3);
    let vols = garman_klass_ewma_vol(&series, B2_01_N_EFF, B2_01_BARS_PER_DAY)?;
    let closes: Vec<f64> = series.iter().map(|c| c.close).collect();
    let sigma_bars: Vec<f64> = vols.iter().map(|v| v.sigma_bar).collect();

    let phi = (1.0 + 5.0f64.sqrt()) * 0.5;
    let weights_and_k = [(72usize, 0.25f64), (168usize, 0.35f64), (504usize, 0.40f64)];

    let full_series = tsmom_multi_horizon(&closes, &sigma_bars, &B2_01_LOOKBACKS)?;
    assert_eq!(full_series.len(), closes.len());
    for (idx, val) in full_series.iter().enumerate().take(504) {
        assert!(
            val.is_none(),
            "expected None before index 504, got at {idx}"
        );
    }

    for &idx in &[504usize, 520usize, 539usize] {
        let mut expected = 0.0f64;
        for &(k, w) in &weights_and_k {
            let z_k = (closes[idx] / closes[idx - k]).ln() / (sigma_bars[idx] * (k as f64).sqrt());
            expected += w * (z_k / phi).tanh();
        }
        let got_at = tsmom_multi_horizon_at(&closes, &sigma_bars, &B2_01_LOOKBACKS, idx)?;
        let got_vec = full_series[idx].ok_or("expected Some tsmom value")?;
        assert!((got_at - expected).abs() < 1e-13);
        assert_eq!(got_at.to_bits(), got_vec.to_bits());
    }

    // 1h -> 4h aggregator exact OHLCVT and incomplete <4-bar bucket discarding
    let t0 = 1_728_000_000_000i64; // 00:00 UTC
    let raw_1h = vec![
        // Incomplete bucket [00:00..04:00): missing 00:00 -> must discard
        mk_candle(t0 + ONE_HOUR_MS, 99.0, 101.0, 98.0, 100.0, 1.0, 100.0),
        mk_candle(t0 + 2 * ONE_HOUR_MS, 100.0, 102.0, 99.0, 101.0, 1.0, 101.0),
        mk_candle(t0 + 3 * ONE_HOUR_MS, 101.0, 103.0, 100.0, 102.0, 1.0, 102.0),
        // Complete bucket [04:00..08:00)
        mk_candle(t0 + 4 * ONE_HOUR_MS, 110.0, 114.0, 109.0, 112.0, 2.0, 224.0),
        mk_candle(t0 + 5 * ONE_HOUR_MS, 112.0, 119.5, 111.0, 115.0, 3.5, 402.5),
        mk_candle(
            t0 + 6 * ONE_HOUR_MS,
            115.0,
            116.0,
            106.25,
            108.0,
            4.0,
            432.0,
        ),
        mk_candle(
            t0 + 7 * ONE_HOUR_MS,
            108.0,
            113.0,
            107.0,
            111.75,
            1.5,
            167.625,
        ),
        // Incomplete bucket [08:00..12:00): only 3 bars (missing 10:00) -> must discard
        mk_candle(
            t0 + 8 * ONE_HOUR_MS,
            111.75,
            115.0,
            110.0,
            113.0,
            2.0,
            226.0,
        ),
        mk_candle(t0 + 9 * ONE_HOUR_MS, 113.0, 114.0, 112.0, 112.5, 1.0, 112.5),
        mk_candle(
            t0 + 11 * ONE_HOUR_MS,
            112.5,
            116.0,
            112.0,
            115.0,
            2.0,
            230.0,
        ),
        // Complete bucket [12:00..16:00)
        mk_candle(
            t0 + 12 * ONE_HOUR_MS,
            120.0,
            125.0,
            118.0,
            124.0,
            5.0,
            620.0,
        ),
        mk_candle(
            t0 + 13 * ONE_HOUR_MS,
            124.0,
            128.0,
            122.0,
            123.0,
            6.0,
            738.0,
        ),
        mk_candle(
            t0 + 14 * ONE_HOUR_MS,
            123.0,
            126.0,
            117.0,
            119.0,
            4.0,
            476.0,
        ),
        mk_candle(
            t0 + 15 * ONE_HOUR_MS,
            119.0,
            122.0,
            118.5,
            121.5,
            3.0,
            364.5,
        ),
        // Trailing incomplete bucket [16:00..20:00): 2 bars -> must discard
        mk_candle(
            t0 + 16 * ONE_HOUR_MS,
            121.5,
            123.0,
            120.0,
            122.0,
            1.0,
            122.0,
        ),
        mk_candle(
            t0 + 17 * ONE_HOUR_MS,
            122.0,
            124.0,
            121.0,
            123.0,
            1.0,
            123.0,
        ),
    ];

    let bars_4h = aggregate_1h_to_4h(&raw_1h)?;
    assert_eq!(bars_4h.len(), 2);

    assert_eq!(bars_4h[0].open_ts_ms, t0 + FOUR_HOURS_MS);
    assert_eq!(bars_4h[0].open, 110.0);
    assert_eq!(bars_4h[0].high, 119.5);
    assert_eq!(bars_4h[0].low, 106.25);
    assert_eq!(bars_4h[0].close, 111.75);
    assert!((bars_4h[0].volume - 11.0).abs() < 1e-12);
    assert!((bars_4h[0].turnover - 1226.125).abs() < 1e-12);

    assert_eq!(bars_4h[1].open_ts_ms, t0 + 3 * FOUR_HOURS_MS);
    assert_eq!(bars_4h[1].open, 120.0);
    assert_eq!(bars_4h[1].high, 128.0);
    assert_eq!(bars_4h[1].low, 117.0);
    assert_eq!(bars_4h[1].close, 121.5);
    assert!((bars_4h[1].volume - 18.0).abs() < 1e-12);
    assert!((bars_4h[1].turnover - 2198.5).abs() < 1e-12);

    // Boundary predicate check
    assert!(is_4h_utc_close(t0 + 3 * ONE_HOUR_MS));
    assert!(is_4h_utc_close(t0 + 7 * ONE_HOUR_MS));
    assert!(!is_4h_utc_close(t0 + 2 * ONE_HOUR_MS));
    assert!(!is_4h_utc_close(t0 + 4 * ONE_HOUR_MS));

    Ok(())
}

#[test]
fn zero_lookahead_b2_01_bit_identical_after_future_perturbation() -> TestResult {
    let cutoff = 580usize;
    let base_universe: [Vec<Candle>; UNIVERSE_LEN] = [
        build_synthetic_1h_series(610, 0.1),
        build_synthetic_1h_series(610, 0.5),
        build_synthetic_1h_series(610, 1.1),
        build_synthetic_1h_series(610, 1.7),
        build_synthetic_1h_series(610, 2.3),
    ];

    // Baseline: signals computed on prefix 0..=cutoff (581 bars)
    let prefix_universe: [Vec<Candle>; UNIVERSE_LEN] =
        std::array::from_fn(|i| base_universe[i][..=cutoff].to_vec());
    let base_sig = compute_b2_01_signals(&prefix_universe)?;

    // Perturbed: wildly mutate bars after cutoff AND append 40 future bars beyond 610
    let server_time_at_cutoff = base_universe[0][cutoff].open_ts_ms + ONE_HOUR_MS;
    let mut perturbed_filtered: Vec<Vec<Candle>> = Vec::with_capacity(UNIVERSE_LEN);
    for (sym_idx, sym_series) in base_universe.iter().enumerate() {
        let mut s = sym_series.clone();
        for (j, bar) in s.iter_mut().enumerate().skip(cutoff + 1) {
            let mult = 5.0 + (j as f64) * 0.25 + (sym_idx as f64);
            bar.open *= mult;
            bar.high = bar.open * 1.5;
            bar.low = bar.open * 0.5;
            bar.close = bar.open * 1.3;
            bar.volume *= 100.0;
        }
        let last_ts = s.last().ok_or("non-empty series")?.open_ts_ms;
        for extra in 1..=40i64 {
            let p = 9999.0 + (extra as f64) * 50.0;
            s.push(mk_candle(
                last_ts + extra * ONE_HOUR_MS,
                p,
                p + 500.0,
                p - 500.0,
                p + 200.0,
                999.0,
                999.0 * p,
            ));
        }

        // Verify per-bar indicator series up to cutoff are bit-identical even when evaluated on full 650-bar slice
        let vols_orig =
            garman_klass_ewma_vol(&prefix_universe[sym_idx], B2_01_N_EFF, B2_01_BARS_PER_DAY)?;
        let vols_full = garman_klass_ewma_vol(&s, B2_01_N_EFF, B2_01_BARS_PER_DAY)?;
        let closes_orig: Vec<f64> = prefix_universe[sym_idx].iter().map(|c| c.close).collect();
        let sb_orig: Vec<f64> = vols_orig.iter().map(|v| v.sigma_bar).collect();
        let closes_full: Vec<f64> = s.iter().map(|c| c.close).collect();
        let sb_full: Vec<f64> = vols_full.iter().map(|v| v.sigma_bar).collect();

        let mom_orig = tsmom_multi_horizon(&closes_orig, &sb_orig, &B2_01_LOOKBACKS)?;
        let mom_full = tsmom_multi_horizon(&closes_full, &sb_full, &B2_01_LOOKBACKS)?;

        for t in 0..=cutoff {
            assert_eq!(
                vols_orig[t].sigma_bar.to_bits(),
                vols_full[t].sigma_bar.to_bits(),
                "sigma_bar look-ahead detected at sym={sym_idx}, t={t}"
            );
            assert_eq!(
                vols_orig[t].sigma_ann.to_bits(),
                vols_full[t].sigma_ann.to_bits(),
                "sigma_ann look-ahead detected at sym={sym_idx}, t={t}"
            );
            assert_eq!(
                mom_orig[t].map(f64::to_bits),
                mom_full[t].map(f64::to_bits),
                "tsmom look-ahead detected at sym={sym_idx}, t={t}"
            );
        }

        // Also test strict close filter + B2-01 signal pipeline at cutoff close
        let closed = filter_strictly_closed_1h(&s, server_time_at_cutoff)?;
        perturbed_filtered.push(closed);
    }

    let perturbed_arr: [Vec<Candle>; UNIVERSE_LEN] = perturbed_filtered
        .try_into()
        .map_err(|_| "expected 5 series")?;
    let pert_sig = compute_b2_01_signals(&perturbed_arr)?;

    assert_eq!(base_sig.last_open_ts_ms, pert_sig.last_open_ts_ms);
    assert_eq!(base_sig.last_close_ts_ms, pert_sig.last_close_ts_ms);
    for i in 0..UNIVERSE_LEN {
        assert_eq!(
            base_sig.sigma_bar[i].to_bits(),
            pert_sig.sigma_bar[i].to_bits()
        );
        assert_eq!(
            base_sig.sigma_ann[i].to_bits(),
            pert_sig.sigma_ann[i].to_bits()
        );
        assert_eq!(base_sig.tsmom[i].to_bits(), pert_sig.tsmom[i].to_bits());
        assert_eq!(
            base_sig.raw_weights[i].to_bits(),
            pert_sig.raw_weights[i].to_bits()
        );
        assert_eq!(
            base_sig.target_weights[i].to_bits(),
            pert_sig.target_weights[i].to_bits()
        );
    }

    Ok(())
}

#[test]
fn spot_v5_asymmetric_fee_accounting_and_zero_drift() -> TestResult {
    let mut ledger = SpotPortfolioLedger::new(Money1e8(100_000_000_000))?; // 1000.00 USDT
    let prices_buy = [Price1e8(10_000_000_000); UNIVERSE_LEN]; // 100.00 USDT

    // Buy 500.00 USDT -> gross 5.00000000 BTC, fee_base = 0.00500000 BTC,
    // base_precision = 0.01 -> (5.000 - 0.005) = 4.995 floored to 0.01 => 4.99 BTC!
    ledger.apply_buy_fill(
        Symbol::BtcUsdt,
        Money1e8(50_000_000_000),
        Qty1e8(500_000_000),
        Qty1e8(500_000),
        Qty1e8(1_000_000),
    )?;

    assert_eq!(ledger.cash_1e8(), Money1e8(50_000_000_000));
    assert_eq!(
        ledger.position(Symbol::BtcUsdt).qty_1e8,
        Qty1e8(499_000_000)
    );
    assert_eq!(
        ledger.position(Symbol::BtcUsdt).cost_basis_usdt_1e8,
        Money1e8(50_000_000_000)
    );
    assert_eq!(ledger.cum_buy_fees_usdt_est_1e8(), Money1e8(50_000_000));
    assert_eq!(ledger.drift_book_1e8()?, 0);
    assert_eq!(ledger.drift_mtm_1e8(&prices_buy)?, 0);

    // Partial sell 2.00 BTC at 120.00 USDT -> gross 240.00 USDT, fee 0.24 USDT in USDT
    let mut prices_sell = [Price1e8(10_000_000_000); UNIVERSE_LEN];
    prices_sell[Symbol::BtcUsdt.index()] = Price1e8(12_000_000_000);

    ledger.apply_sell_fill(
        Symbol::BtcUsdt,
        Qty1e8(200_000_000),
        Money1e8(24_000_000_000),
        Money1e8(24_000_000),
    )?;

    // Closed-form released cost: round(50_000_000_000 * 200_000_000 / 499_000_000)
    let expected_released_cost = (50_000_000_000i128 * 200_000_000 + 249_500_000) / 499_000_000;
    let expected_gross_pnl = 24_000_000_000i128 - expected_released_cost;
    let expected_net_pnl = expected_gross_pnl - 24_000_000i128;

    assert_eq!(
        ledger.cash_1e8(),
        Money1e8(50_000_000_000 + 24_000_000_000 - 24_000_000)
    );
    assert_eq!(
        ledger.position(Symbol::BtcUsdt).qty_1e8,
        Qty1e8(299_000_000)
    );
    assert_eq!(
        ledger.position(Symbol::BtcUsdt).cost_basis_usdt_1e8,
        Money1e8(50_000_000_000 - expected_released_cost)
    );
    assert_eq!(
        ledger.realized_pnl_gross_1e8(),
        Money1e8(expected_gross_pnl)
    );
    assert_eq!(ledger.cum_sell_fees_usdt_1e8(), Money1e8(24_000_000));
    assert_eq!(ledger.realized_pnl_net_1e8(), Money1e8(expected_net_pnl));
    assert_eq!(ledger.drift_book_1e8()?, 0);
    assert_eq!(ledger.drift_mtm_1e8(&prices_sell)?, 0);

    Ok(())
}

#[test]
fn strict_sell_first_buy_second_with_zero_initial_cash_and_reserve() -> TestResult {
    let rules = default_rules();
    let prices = [
        Price1e8(10_000_000_000), // BTC = $100
        Price1e8(10_000_000_000), // ETH = $100
        Price1e8(10_000_000_000), // SOL = $100
        Price1e8(10_000_000_000), // LINK = $100
        Price1e8(2_000_000_000),  // AVAX = $20
    ];

    // Start with $0 cash and 20.0 AVAX ($400) at index 4; targets are BTC (idx 0) and ETH (idx 1)
    let mut positions = [PositionState::default(); UNIVERSE_LEN];
    positions[Symbol::AvaxUsdt.index()] = PositionState {
        qty_1e8: Qty1e8(2_000_000_000),
        cost_basis_usdt_1e8: Money1e8(40_000_000_000),
    };
    let current_weights = [0.0, 0.0, 0.0, 0.0, 1.0];
    let target_weights = [0.40, 0.40, 0.20, 0.0, 0.0];

    let orders = plan_rebalance_orders(&RebalancePlanInput {
        cash_1e8: Money1e8::ZERO,
        positions: &positions,
        prices_1e8: &prices,
        rules: &rules,
        target_weights: &target_weights,
        current_weights: &current_weights,
        fee_rate: DEFAULT_SPOT_FEE_RATE, // 10 bps
        allow_buys: true,
    })?;

    assert_eq!(orders.len(), 4);
    assert_eq!(orders[0].side, OrderSide::Sell);
    assert_eq!(orders[0].symbol, Symbol::AvaxUsdt);
    assert!(orders[0].full_liquidation);

    // Every Buy must appear strictly after all Sells
    let first_buy_pos = orders
        .iter()
        .position(|o| o.side == OrderSide::Buy)
        .ok_or("expected Buy orders")?;
    assert!(orders[..first_buy_pos]
        .iter()
        .all(|o| o.side == OrderSide::Sell));
    assert!(orders[first_buy_pos..]
        .iter()
        .all(|o| o.side == OrderSide::Buy));

    // Closed-form projected post-sell cash minus 0.3% (30 bps) reserve:
    // sell_gross = $400 = 40_000_000_000; sell_fee = 0.001 * 400 = $0.40 = 40_000_000;
    // net_cash = 39_960_000_000; budget = floor(39_960_000_000 * 9970 / 10000) = 39_840_120_000.
    // Total desired buy = $160 + $160 + $80 = $400 = 40_000_000_000 > budget => scaled proportionally!
    let expected_budget = 39_840_120_000i128;
    let total_desired = 40_000_000_000i128;
    let exp_btc = (16_000_000_000i128 * expected_budget) / total_desired;
    let exp_eth = (16_000_000_000i128 * expected_budget) / total_desired;
    let exp_sol = (8_000_000_000i128 * expected_budget) / total_desired;

    assert_eq!(orders[1].symbol, Symbol::BtcUsdt);
    assert_eq!(orders[1].quote_spend_1e8, Money1e8(exp_btc));
    assert_eq!(orders[2].symbol, Symbol::EthUsdt);
    assert_eq!(orders[2].quote_spend_1e8, Money1e8(exp_eth));
    assert_eq!(orders[3].symbol, Symbol::SolUsdt);
    assert_eq!(orders[3].quote_spend_1e8, Money1e8(exp_sol));

    let total_buy_spend: i128 = orders[1..].iter().map(|o| o.quote_spend_1e8.raw()).sum();
    assert!(total_buy_spend <= expected_budget);

    Ok(())
}

#[test]
fn external_deposit_plus_50_usdt_is_neutral_to_twr_and_trading_pnl() -> TestResult {
    let mut ledger = SpotPortfolioLedger::new(Money1e8(10_000_000_000))?; // $100.00
    let p_entry = [Price1e8(5_000_000_000); UNIVERSE_LEN]; // $50.00

    ledger.apply_buy_fill(
        Symbol::EthUsdt,
        Money1e8(6_000_000_000), // $60.00
        Qty1e8(120_000_000),     // 1.20 ETH
        Qty1e8(120_000),         // 0.0012 ETH fee
        Qty1e8(100_000),         // step 0.001 -> net 1.198 ETH
    )?;
    ledger.assert_zero_drift(&p_entry)?;

    // Mark at +25% ($62.50) and realize partial gain
    let p_up = [Price1e8(6_250_000_000); UNIVERSE_LEN];
    assert!(ledger.mark_to_market(&p_up)?.is_none());

    ledger.apply_sell_fill(
        Symbol::EthUsdt,
        Qty1e8(50_000_000),      // 0.50 ETH
        Money1e8(3_125_000_000), // $31.25
        Money1e8(3_125_000),     // $0.03125 fee
    )?;

    // Mark at $65.00 before external deposit
    let p_mark = [Price1e8(6_500_000_000); UNIVERSE_LEN];
    assert!(ledger.mark_to_market(&p_mark)?.is_none());

    let twr_before = ledger.twr_index_1e10();
    let peak_before = ledger.twr_peak_1e10();
    let trading_pnl_before = ledger
        .realized_pnl_net_1e8()
        .checked_add(ledger.unrealized_pnl_1e8(&p_mark)?)?;
    let deposits_before = ledger.total_deposits_1e8();
    let cash_before = ledger.cash_1e8();

    let deposit = Money1e8(5_000_000_000); // +$50.00
    let events = ledger.record_external_deposit(deposit, &p_mark)?;
    assert_eq!(
        events,
        vec![DomainEvent::ExternalDepositRecorded {
            delta_1e8: deposit,
            new_total_deposits_1e8: deposits_before.checked_add(deposit)?,
        }]
    );

    let trading_pnl_after = ledger
        .realized_pnl_net_1e8()
        .checked_add(ledger.unrealized_pnl_1e8(&p_mark)?)?;

    assert_eq!(ledger.twr_index_1e10(), twr_before);
    assert_eq!(ledger.twr_peak_1e10(), peak_before);
    assert_eq!(trading_pnl_after, trading_pnl_before);
    assert_eq!(ledger.cash_1e8(), cash_before.checked_add(deposit)?);
    assert_eq!(ledger.drift_book_1e8()?, 0);
    assert_eq!(ledger.drift_mtm_1e8(&p_mark)?, 0);

    Ok(())
}

#[test]
fn anti_dust_escalation_and_below_5_usdt_buy_skipped() -> TestResult {
    let rules = default_rules();
    let prices = [Price1e8(10_000_000_000); UNIVERSE_LEN]; // $100

    // Case 1: Anti-dust sell escalation (1.04 ETH = $104, target 0.03 = $3.12 remainder < $5)
    let mut pos_dust = [PositionState::default(); UNIVERSE_LEN];
    pos_dust[Symbol::EthUsdt.index()] = PositionState {
        qty_1e8: Qty1e8(104_000_000),
        cost_basis_usdt_1e8: Money1e8(10_400_000_000),
    };
    let orders_dust = plan_rebalance_orders(&RebalancePlanInput {
        cash_1e8: Money1e8::ZERO,
        positions: &pos_dust,
        prices_1e8: &prices,
        rules: &rules,
        target_weights: &[0.0, 0.03, 0.0, 0.0, 0.0],
        current_weights: &[0.0, 1.0, 0.0, 0.0, 0.0],
        fee_rate: DEFAULT_SPOT_FEE_RATE,
        allow_buys: true,
    })?;
    assert_eq!(orders_dust.len(), 1);
    assert_eq!(orders_dust[0].side, OrderSide::Sell);
    assert!(orders_dust[0].full_liquidation);
    assert_eq!(orders_dust[0].base_qty_1e8, Qty1e8(104_000_000));

    // Case 2: < $5 buy skipped cleanly
    // Equity = $50.00 cash, target weight = 0.09 (>= DEADBAND 0.08 so rebalance triggers)
    // Desired buy = $50.00 * 0.09 = $4.50 < $5.00 min_order_amt => cleanly skipped (0 orders).
    let pos_empty = [PositionState::default(); UNIVERSE_LEN];
    let orders_small_buy = plan_rebalance_orders(&RebalancePlanInput {
        cash_1e8: Money1e8(5_000_000_000), // $50.00
        positions: &pos_empty,
        prices_1e8: &prices,
        rules: &rules,
        target_weights: &[0.09, 0.0, 0.0, 0.0, 0.0],
        current_weights: &[0.0, 0.0, 0.0, 0.0, 0.0],
        fee_rate: DEFAULT_SPOT_FEE_RATE,
        allow_buys: true,
    })?;
    assert!(orders_small_buy.is_empty());

    Ok(())
}

#[test]
fn hard_drawdown_halt_strictly_above_25_percent() -> TestResult {
    let mut ledger = SpotPortfolioLedger::new(Money1e8(100_000_000_000))?; // $1000
    ledger.apply_buy_fill(
        Symbol::SolUsdt,
        Money1e8(100_000_000_000),
        Qty1e8(1_000_000_000), // 10 SOL @ $100
        Qty1e8::ZERO,
        Qty1e8(100_000),
    )?;

    // Exactly 25.00% drawdown -> must NOT halt
    let p_25 = [Price1e8(7_500_000_000); UNIVERSE_LEN];
    assert!(ledger.mark_to_market(&p_25)?.is_none());
    assert!(!ledger.is_halted());

    // 25.01% drawdown ($74.99 * 10 = $749.90) -> must emit HardDrawdownHalt and halt
    let p_over_25 = [Price1e8(7_499_000_000); UNIVERSE_LEN];
    let ev = ledger.mark_to_market(&p_over_25)?;
    assert!(matches!(
        ev,
        Some(DomainEvent::HardDrawdownHalt { drawdown_1e10, .. })
            if drawdown_1e10 > 2_500_000_000
    ));
    assert!(ledger.is_halted());

    // Verify zero drift even when halted
    assert_eq!(ledger.drift_book_1e8()?, 0);
    assert_eq!(ledger.drift_mtm_1e8(&p_over_25)?, 0);
    let _ = SCALE_1E8; // reference domain constant cleanly

    Ok(())
}
