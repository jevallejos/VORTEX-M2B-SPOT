use crate::domain::error::DomainError;
use crate::domain::indicators::{
    garman_klass_ewma_vol, tsmom_multi_horizon_at, A0_4H_BARS_PER_DAY, A0_4H_LOOKBACKS,
    A0_4H_N_EFF, B2_01_BARS_PER_DAY, B2_01_LOOKBACKS, B2_01_N_EFF,
};
use crate::domain::types::{
    mul_div_floor_nonneg, mul_div_round, Candle, InstrumentRules, Money1e8, OrderSide,
    PlannedOrder, PositionState, Price1e8, Qty1e8, Rate1e10, Symbol, FOUR_HOURS_MS, ONE_HOUR_MS,
    RATE_SCALE_1E10, SCALE_1E8, UNIVERSE_LEN,
};

pub const SIGMA_TARGET: f64 = 0.50;
pub const W_MAX: f64 = 0.40;
pub const L_MAX: f64 = 1.00;
pub const VOL_FLOOR_ANN: f64 = 0.15;
pub const DEADBAND_DELTA: f64 = 0.20;
pub const DEADBAND: f64 = 0.08;
pub const LIQUIDATE_TARGET_THRESH: f64 = 0.015;
pub const LIQUIDATE_CURRENT_THRESH: f64 = 0.02;
pub const BUY_SLIP_RESERVE_BPS: i128 = 30;
pub const MIN_CLOSED_1H_CANDLES: usize = 600;
pub const MIN_CLOSED_4H_CANDLES: usize = 147;

#[derive(Debug, Clone, PartialEq)]
pub struct SignalOutput {
    pub last_open_ts_ms: i64,
    pub last_close_ts_ms: i64,
    pub sigma_bar: [f64; UNIVERSE_LEN],
    pub sigma_ann: [f64; UNIVERSE_LEN],
    pub tsmom: [f64; UNIVERSE_LEN],
    pub raw_weights: [f64; UNIVERSE_LEN],
    pub target_weights: [f64; UNIVERSE_LEN],
}

#[derive(Debug, Clone)]
pub struct RebalancePlanInput<'a> {
    pub cash_1e8: Money1e8,
    pub positions: &'a [PositionState; UNIVERSE_LEN],
    pub prices_1e8: &'a [Price1e8; UNIVERSE_LEN],
    pub rules: &'a [InstrumentRules; UNIVERSE_LEN],
    pub target_weights: &'a [f64; UNIVERSE_LEN],
    pub current_weights: &'a [f64; UNIVERSE_LEN],
    pub fee_rate: Rate1e10,
    pub allow_buys: bool,
}

struct BuyCandidate {
    idx: usize,
    desired_quote_1e8: i128,
    current_weight: f64,
    target_weight: f64,
}

fn sort_and_dedup_candles(mut indexed: Vec<(usize, Candle)>) -> Vec<Candle> {
    indexed.sort_by_key(|(idx, c)| (c.open_ts_ms, *idx));
    let mut out: Vec<Candle> = Vec::with_capacity(indexed.len());
    for (_, candle) in indexed {
        if let Some(last) = out.last_mut() {
            if last.open_ts_ms == candle.open_ts_ms {
                *last = candle;
                continue;
            }
        }
        out.push(candle);
    }
    out
}

pub fn filter_strictly_closed_1h(
    candles: &[Candle],
    server_time_ms: i64,
) -> Result<Vec<Candle>, DomainError> {
    if server_time_ms < 0 {
        return Err(DomainError::NegativeValueNotAllowed("server_time_ms"));
    }

    let mut kept = Vec::with_capacity(candles.len());
    for (idx, c) in candles.iter().enumerate() {
        let close_ts_ms = c
            .open_ts_ms
            .checked_add(ONE_HOUR_MS)
            .ok_or(DomainError::ArithmeticOverflow)?;
        if close_ts_ms <= server_time_ms {
            c.validate()?;
            kept.push((idx, *c));
        }
    }

    Ok(sort_and_dedup_candles(kept))
}

pub fn aggregate_1h_to_4h(closed_1h: &[Candle]) -> Result<Vec<Candle>, DomainError> {
    let mut validated = Vec::with_capacity(closed_1h.len());
    for (idx, c) in closed_1h.iter().enumerate() {
        c.validate()?;
        validated.push((idx, *c));
    }

    let sorted = sort_and_dedup_candles(validated);
    let mut out = Vec::with_capacity(sorted.len() / 4);

    let mut i = 0usize;
    while i < sorted.len() {
        let t0 = sorted[i].open_ts_ms.div_euclid(FOUR_HOURS_MS) * FOUR_HOURS_MS;
        let mut j = i + 1;
        while j < sorted.len() {
            let bucket_j = sorted[j].open_ts_ms.div_euclid(FOUR_HOURS_MS) * FOUR_HOURS_MS;
            if bucket_j != t0 {
                break;
            }
            j += 1;
        }

        let bucket = &sorted[i..j];
        let expected_ts = [
            t0,
            t0 + ONE_HOUR_MS,
            t0 + 2 * ONE_HOUR_MS,
            t0 + 3 * ONE_HOUR_MS,
        ];

        let mut bars: [Option<Candle>; 4] = [None; 4];
        for c in bucket {
            for (slot, &exp) in expected_ts.iter().enumerate() {
                if c.open_ts_ms == exp {
                    bars[slot] = Some(*c);
                }
            }
        }

        if let [Some(b0), Some(b1), Some(b2), Some(b3)] = bars {
            let high = b0.high.max(b1.high).max(b2.high).max(b3.high);
            let low = b0.low.min(b1.low).min(b2.low).min(b3.low);
            let volume = b0.volume + b1.volume + b2.volume + b3.volume;
            let turnover = b0.turnover + b1.turnover + b2.turnover + b3.turnover;
            let bar_4h = Candle {
                open_ts_ms: t0,
                open: b0.open,
                high,
                low,
                close: b3.close,
                volume,
                turnover,
            };
            bar_4h.validate()?;
            out.push(bar_4h);
        }

        i = j;
    }

    Ok(out)
}

#[inline]
pub fn is_4h_utc_close(last_closed_1h_open_ts_ms: i64) -> bool {
    (last_closed_1h_open_ts_ms + ONE_HOUR_MS).rem_euclid(FOUR_HOURS_MS) == 0
}

pub fn compute_vol_targeted_weights(
    sigma_ann: &[f64; UNIVERSE_LEN],
    tsmom: &[f64; UNIVERSE_LEN],
) -> Result<([f64; UNIVERSE_LEN], [f64; UNIVERSE_LEN]), DomainError> {
    let mut raw = [0.0f64; UNIVERSE_LEN];
    for i in 0..UNIVERSE_LEN {
        if !sigma_ann[i].is_finite() || sigma_ann[i] <= 0.0 {
            return Err(DomainError::InvalidParameter(
                "sigma_ann must be finite and positive",
            ));
        }
        if !tsmom[i].is_finite() {
            return Err(DomainError::InvalidParameter("tsmom must be finite"));
        }
        let denom = sigma_ann[i].max(VOL_FLOOR_ANN);
        raw[i] = ((SIGMA_TARGET / 4.0) / denom * tsmom[i]).clamp(0.0, W_MAX);
    }

    let sum_raw: f64 = raw.iter().sum();
    let scale = if sum_raw > 0.0 {
        (L_MAX / sum_raw).min(1.0)
    } else {
        1.0
    };

    let mut weights = [0.0f64; UNIVERSE_LEN];
    for i in 0..UNIVERSE_LEN {
        weights[i] = raw[i] * scale;
    }

    Ok((raw, weights))
}

fn compute_signals_impl(
    candles_by_symbol: &[Vec<Candle>; UNIVERSE_LEN],
    min_candles: usize,
    n_eff: usize,
    bars_per_day: f64,
    lookbacks: &[(usize, f64)],
    interval_ms: i64,
) -> Result<SignalOutput, DomainError> {
    let mut sigma_bar_arr = [0.0f64; UNIVERSE_LEN];
    let mut sigma_ann_arr = [0.0f64; UNIVERSE_LEN];
    let mut tsmom_arr = [0.0f64; UNIVERSE_LEN];
    let mut last_open_ts_ms = 0i64;

    for i in 0..UNIVERSE_LEN {
        let candles = &candles_by_symbol[i];
        if candles.len() < min_candles {
            return Err(DomainError::InsufficientHistory {
                required: min_candles,
                actual: candles.len(),
            });
        }

        for w in candles.windows(2) {
            if w[1].open_ts_ms <= w[0].open_ts_ms {
                return Err(DomainError::InvalidCandle(
                    "candles must be strictly ascending by open_ts_ms".to_string(),
                ));
            }
        }

        let vols = garman_klass_ewma_vol(candles, n_eff, bars_per_day)?;
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let sigma_bars: Vec<f64> = vols.iter().map(|v| v.sigma_bar).collect();

        let last_idx = candles.len() - 1;
        sigma_bar_arr[i] = vols[last_idx].sigma_bar;
        sigma_ann_arr[i] = vols[last_idx].sigma_ann;
        tsmom_arr[i] = tsmom_multi_horizon_at(&closes, &sigma_bars, lookbacks, last_idx)?;

        let sym_last_ts = candles[last_idx].open_ts_ms;
        if i == 0 || sym_last_ts > last_open_ts_ms {
            last_open_ts_ms = sym_last_ts;
        }
    }

    let (raw_weights, target_weights) = compute_vol_targeted_weights(&sigma_ann_arr, &tsmom_arr)?;
    let last_close_ts_ms = last_open_ts_ms
        .checked_add(interval_ms)
        .ok_or(DomainError::ArithmeticOverflow)?;

    Ok(SignalOutput {
        last_open_ts_ms,
        last_close_ts_ms,
        sigma_bar: sigma_bar_arr,
        sigma_ann: sigma_ann_arr,
        tsmom: tsmom_arr,
        raw_weights,
        target_weights,
    })
}

pub fn compute_b2_01_signals(
    closed_1h_by_symbol: &[Vec<Candle>; UNIVERSE_LEN],
) -> Result<SignalOutput, DomainError> {
    compute_signals_impl(
        closed_1h_by_symbol,
        505,
        B2_01_N_EFF,
        B2_01_BARS_PER_DAY,
        &B2_01_LOOKBACKS,
        ONE_HOUR_MS,
    )
}

pub fn compute_a0_4h_signals(
    closed_4h_by_symbol: &[Vec<Candle>; UNIVERSE_LEN],
) -> Result<SignalOutput, DomainError> {
    compute_signals_impl(
        closed_4h_by_symbol,
        127,
        A0_4H_N_EFF,
        A0_4H_BARS_PER_DAY,
        &A0_4H_LOOKBACKS,
        FOUR_HOURS_MS,
    )
}

#[inline]
pub fn should_rebalance(target_w: f64, current_w: f64) -> bool {
    (target_w - current_w).abs() >= DEADBAND
        || (target_w < LIQUIDATE_TARGET_THRESH && current_w >= LIQUIDATE_CURRENT_THRESH)
}

pub fn plan_rebalance_orders(
    input: &RebalancePlanInput<'_>,
) -> Result<Vec<PlannedOrder>, DomainError> {
    if input.cash_1e8.0 < 0 {
        return Err(DomainError::NegativeValueNotAllowed("cash_1e8"));
    }
    if input.fee_rate.0 < 0 {
        return Err(DomainError::NegativeValueNotAllowed("fee_rate"));
    }

    let mut pos_vals_1e8 = [0i128; UNIVERSE_LEN];
    let mut equity_1e8 = input.cash_1e8.0;

    for (i, pos_val_slot) in pos_vals_1e8.iter_mut().enumerate() {
        let rule = &input.rules[i];
        let sym = Symbol::ALL[i];
        if !rule.is_trading {
            return Err(DomainError::InvalidInstrumentRules(format!(
                "symbol {sym} is not trading"
            )));
        }
        if rule.base_precision.0 <= 0 {
            return Err(DomainError::InvalidInstrumentRules(format!(
                "symbol {sym} base_precision must be positive"
            )));
        }
        if rule.min_order_qty.0 <= 0 {
            return Err(DomainError::InvalidInstrumentRules(format!(
                "symbol {sym} min_order_qty must be positive"
            )));
        }
        if rule.min_order_amt.0 <= 0 {
            return Err(DomainError::InvalidInstrumentRules(format!(
                "symbol {sym} min_order_amt must be positive"
            )));
        }
        if input.prices_1e8[i].0 <= 0 {
            return Err(DomainError::InvalidParameter(
                "prices_1e8 entries must be positive",
            ));
        }
        if input.positions[i].qty_1e8.0 < 0 {
            return Err(DomainError::NegativeValueNotAllowed("position qty_1e8"));
        }
        if !input.target_weights[i].is_finite() || !input.current_weights[i].is_finite() {
            return Err(DomainError::InvalidParameter(
                "target_weights and current_weights must be finite",
            ));
        }

        let pos_val = mul_div_floor_nonneg(
            input.positions[i].qty_1e8.0,
            input.prices_1e8[i].0,
            SCALE_1E8,
        )?;
        *pos_val_slot = pos_val;
        equity_1e8 = equity_1e8
            .checked_add(pos_val)
            .ok_or(DomainError::ArithmeticOverflow)?;
    }

    if equity_1e8 <= 0 {
        return Ok(Vec::new());
    }

    let mut sells: Vec<PlannedOrder> = Vec::new();
    let mut buy_candidates: Vec<BuyCandidate> = Vec::new();

    for (i, &current_val_1e8) in pos_vals_1e8.iter().enumerate() {
        let raw_target_w = input.target_weights[i];
        let current_w = input.current_weights[i];
        if !should_rebalance(raw_target_w, current_w) {
            continue;
        }

        let target_w = if raw_target_w < LIQUIDATE_TARGET_THRESH {
            0.0
        } else {
            raw_target_w
        };

        let rule = &input.rules[i];
        let price = input.prices_1e8[i];
        let pos_qty = input.positions[i].qty_1e8;

        if target_w == 0.0 {
            let sellable = pos_qty.floor_to_step(rule.base_precision)?;
            let notional_1e8 = mul_div_floor_nonneg(sellable.0, price.0, SCALE_1E8)?;
            if sellable >= rule.min_order_qty && Money1e8(notional_1e8) >= rule.min_order_amt {
                sells.push(PlannedOrder {
                    symbol: Symbol::ALL[i],
                    side: OrderSide::Sell,
                    quote_spend_1e8: Money1e8(notional_1e8),
                    base_qty_1e8: sellable,
                    reference_price_1e8: price,
                    current_weight: current_w,
                    target_weight: 0.0,
                    full_liquidation: true,
                });
            }
            continue;
        }

        let desired_f64 = ((equity_1e8 as f64) * target_w).round();
        if !desired_f64.is_finite() || desired_f64 < 0.0 || desired_f64 > (i128::MAX as f64) {
            return Err(DomainError::ArithmeticOverflow);
        }
        let desired_val_1e8 = desired_f64 as i128;

        if desired_val_1e8 < current_val_1e8 {
            let sell_val = current_val_1e8 - desired_val_1e8;
            let raw_sell_qty = mul_div_floor_nonneg(sell_val, SCALE_1E8, price.0)?;
            let mut sell_qty = Qty1e8(raw_sell_qty).floor_to_step(rule.base_precision)?;
            let mut full_liquidation = false;

            let rem_qty = pos_qty.checked_sub(sell_qty)?;
            let rem_val = mul_div_floor_nonneg(rem_qty.0, price.0, SCALE_1E8)?;
            if rem_qty.0 > 0
                && (Money1e8(rem_val) < rule.min_order_amt || rem_qty < rule.min_order_qty)
            {
                sell_qty = pos_qty.floor_to_step(rule.base_precision)?;
                full_liquidation = true;
            }

            let sell_notional = mul_div_floor_nonneg(sell_qty.0, price.0, SCALE_1E8)?;
            if sell_qty >= rule.min_order_qty && Money1e8(sell_notional) >= rule.min_order_amt {
                sells.push(PlannedOrder {
                    symbol: Symbol::ALL[i],
                    side: OrderSide::Sell,
                    quote_spend_1e8: Money1e8(sell_notional),
                    base_qty_1e8: sell_qty,
                    reference_price_1e8: price,
                    current_weight: current_w,
                    target_weight: target_w,
                    full_liquidation,
                });
            }
        } else if desired_val_1e8 > current_val_1e8 && input.allow_buys {
            buy_candidates.push(BuyCandidate {
                idx: i,
                desired_quote_1e8: desired_val_1e8 - current_val_1e8,
                current_weight: current_w,
                target_weight: target_w,
            });
        }
    }

    // Strict SELL_FIRST_BUY_SECOND
    let mut orders = sells;

    let mut net_sell_proceeds_1e8 = 0i128;
    for sell in &orders {
        let sell_gross = sell.quote_spend_1e8.0;
        let sell_fee = mul_div_round(sell_gross, input.fee_rate.0, RATE_SCALE_1E10)?;
        let net = sell_gross
            .checked_sub(sell_fee)
            .ok_or(DomainError::ArithmeticOverflow)?;
        if net > 0 {
            net_sell_proceeds_1e8 = net_sell_proceeds_1e8
                .checked_add(net)
                .ok_or(DomainError::ArithmeticOverflow)?;
        }
    }

    let projected_cash = input
        .cash_1e8
        .0
        .checked_add(net_sell_proceeds_1e8)
        .ok_or(DomainError::ArithmeticOverflow)?;
    let budget = mul_div_floor_nonneg(projected_cash, 10_000 - BUY_SLIP_RESERVE_BPS, 10_000)?;

    let mut total_desired_buy_quote = 0i128;
    for cand in &buy_candidates {
        total_desired_buy_quote = total_desired_buy_quote
            .checked_add(cand.desired_quote_1e8)
            .ok_or(DomainError::ArithmeticOverflow)?;
    }

    if budget > 0 && total_desired_buy_quote > 0 {
        for cand in buy_candidates {
            let scaled_quote = if total_desired_buy_quote > budget {
                mul_div_floor_nonneg(cand.desired_quote_1e8, budget, total_desired_buy_quote)?
            } else {
                cand.desired_quote_1e8
            };

            let rule = &input.rules[cand.idx];
            let price = input.prices_1e8[cand.idx];
            if Money1e8(scaled_quote) >= rule.min_order_amt {
                let raw_qty = mul_div_floor_nonneg(scaled_quote, SCALE_1E8, price.0)?;
                let est_qty = Qty1e8(raw_qty).floor_to_step(rule.base_precision)?;
                if est_qty >= rule.min_order_qty {
                    orders.push(PlannedOrder {
                        symbol: Symbol::ALL[cand.idx],
                        side: OrderSide::Buy,
                        quote_spend_1e8: Money1e8(scaled_quote),
                        base_qty_1e8: est_qty,
                        reference_price_1e8: price,
                        current_weight: cand.current_weight,
                        target_weight: cand.target_weight,
                        full_liquidation: false,
                    });
                }
            }
        }
    }

    Ok(orders)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_candle(ts: i64, open: f64, high: f64, low: f64, close: f64, vol: f64) -> Candle {
        Candle {
            open_ts_ms: ts,
            open,
            high,
            low,
            close,
            volume: vol,
            turnover: vol * close,
        }
    }

    fn mk_rules(sym: Symbol) -> InstrumentRules {
        InstrumentRules {
            symbol: sym,
            base_precision: Qty1e8(1_000_000),    // 0.01
            quote_precision: Money1e8(1_000_000), // 0.01
            min_order_qty: Qty1e8(1_000_000),     // 0.01
            min_order_amt: Money1e8(500_000_000), // 5 USDT
            tick_size: Price1e8(1_000_000),
            is_trading: true,
        }
    }

    #[test]
    fn filter_and_aggregate_1h_to_4h_works() -> Result<(), DomainError> {
        let server_time = 5 * ONE_HOUR_MS;
        let candles = vec![
            mk_candle(0, 100.0, 102.0, 99.0, 101.0, 1.0),
            mk_candle(ONE_HOUR_MS, 101.0, 106.0, 100.0, 103.0, 2.0),
            mk_candle(2 * ONE_HOUR_MS, 103.0, 105.0, 98.0, 104.0, 3.0),
            mk_candle(3 * ONE_HOUR_MS, 104.0, 105.0, 101.0, 102.5, 4.0),
            // Duplicate of hour 3 (latest should win)
            mk_candle(3 * ONE_HOUR_MS, 104.0, 107.0, 101.0, 105.0, 5.0),
            // Incomplete bucket (only hour 4)
            mk_candle(4 * ONE_HOUR_MS, 105.0, 106.0, 104.0, 105.5, 1.0),
            // Unclosed bar (open at 5h, closes at 6h > server_time)
            mk_candle(5 * ONE_HOUR_MS, 105.5, 108.0, 105.0, 107.0, 2.0),
        ];

        let closed = filter_strictly_closed_1h(&candles, server_time)?;
        assert_eq!(closed.len(), 5);
        assert_eq!(closed[3].close, 105.0);

        let bars_4h = aggregate_1h_to_4h(&closed)?;
        assert_eq!(bars_4h.len(), 1);
        assert_eq!(bars_4h[0].open_ts_ms, 0);
        assert_eq!(bars_4h[0].open, 100.0);
        assert_eq!(bars_4h[0].high, 107.0);
        assert_eq!(bars_4h[0].low, 98.0);
        assert_eq!(bars_4h[0].close, 105.0);
        assert!((bars_4h[0].volume - 11.0).abs() < 1e-12);

        assert!(is_4h_utc_close(3 * ONE_HOUR_MS));
        assert!(!is_4h_utc_close(2 * ONE_HOUR_MS));
        Ok(())
    }

    #[test]
    fn vol_targeted_weights_respect_caps_and_l_max() -> Result<(), DomainError> {
        let sigma_ann = [0.20, 0.10, 0.25, 0.30, 0.20];
        let tsmom = [0.95, 0.95, 0.95, 0.95, -0.50];
        let (raw, w) = compute_vol_targeted_weights(&sigma_ann, &tsmom)?;
        assert!((raw[0] - W_MAX).abs() < 1e-12);
        assert!((raw[1] - W_MAX).abs() < 1e-12);
        assert_eq!(raw[4], 0.0);
        let sum_w: f64 = w.iter().sum();
        assert!(sum_w <= L_MAX + 1e-12);
        Ok(())
    }

    #[test]
    fn plan_rebalance_sell_first_anti_dust_and_buy_reserve() -> Result<(), DomainError> {
        let rules = [
            mk_rules(Symbol::BtcUsdt),
            mk_rules(Symbol::EthUsdt),
            mk_rules(Symbol::SolUsdt),
            mk_rules(Symbol::LinkUsdt),
            mk_rules(Symbol::AvaxUsdt),
        ];
        let prices = [
            Price1e8(10_000_000_000), // 100
            Price1e8(10_000_000_000), // 100
            Price1e8(10_000_000_000), // 100
            Price1e8(10_000_000_000), // 100
            Price1e8(10_000_000_000), // 100
        ];
        // Portfolio: 0 cash, 1.03 BTC ($103) -> total equity $103.
        // Target BTC = 0.02 ($2.06) => remainder $2.06 < $5 min_order_amt => escalate to full liquidation!
        // Target ETH = 0.40 ($41.2) => Buy scaled into budget after 0.30% reserve.
        let mut positions = [PositionState::default(); UNIVERSE_LEN];
        positions[0] = PositionState {
            qty_1e8: Qty1e8(103_000_000), // 1.03
            cost_basis_usdt_1e8: Money1e8(10_300_000_000),
        };
        let current_weights = [1.0, 0.0, 0.0, 0.0, 0.0];
        let target_weights = [0.02, 0.40, 0.0, 0.0, 0.0];

        let input = RebalancePlanInput {
            cash_1e8: Money1e8::ZERO,
            positions: &positions,
            prices_1e8: &prices,
            rules: &rules,
            target_weights: &target_weights,
            current_weights: &current_weights,
            fee_rate: Rate1e10::from_bps(10),
            allow_buys: true,
        };

        let orders = plan_rebalance_orders(&input)?;
        assert_eq!(orders.len(), 2);
        assert_eq!(orders[0].side, OrderSide::Sell);
        assert_eq!(orders[0].symbol, Symbol::BtcUsdt);
        assert!(orders[0].full_liquidation);
        assert_eq!(orders[0].base_qty_1e8, Qty1e8(103_000_000));

        assert_eq!(orders[1].side, OrderSide::Buy);
        assert_eq!(orders[1].symbol, Symbol::EthUsdt);
        assert!(orders[1].quote_spend_1e8.0 > 0);
        Ok(())
    }

    #[test]
    fn signals_b2_01_and_a0_4h_smoke() -> Result<(), DomainError> {
        let mut bars_1h = Vec::with_capacity(MIN_CLOSED_1H_CANDLES);
        for k in 0..MIN_CLOSED_1H_CANDLES {
            let p = 100.0 + (k as f64) * 0.05;
            bars_1h.push(mk_candle(
                (k as i64) * ONE_HOUR_MS,
                p,
                p + 1.0,
                p - 0.5,
                p + 0.2,
                10.0,
            ));
        }
        let universe_1h = [
            bars_1h.clone(),
            bars_1h.clone(),
            bars_1h.clone(),
            bars_1h.clone(),
            bars_1h.clone(),
        ];
        let sig_1h = compute_b2_01_signals(&universe_1h)?;
        assert_eq!(
            sig_1h.last_open_ts_ms,
            ((MIN_CLOSED_1H_CANDLES - 1) as i64) * ONE_HOUR_MS
        );
        assert!(sig_1h.target_weights.iter().all(|w| *w >= 0.0));

        let bars_4h = aggregate_1h_to_4h(&bars_1h)?;
        assert!(bars_4h.len() >= MIN_CLOSED_4H_CANDLES);
        let universe_4h = [
            bars_4h.clone(),
            bars_4h.clone(),
            bars_4h.clone(),
            bars_4h.clone(),
            bars_4h,
        ];
        let sig_4h = compute_a0_4h_signals(&universe_4h)?;
        assert!(sig_4h.tsmom[0] > 0.0);
        Ok(())
    }
}
