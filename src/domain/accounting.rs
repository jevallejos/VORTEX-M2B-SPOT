use crate::domain::error::DomainError;
use crate::domain::types::{
    mul_div_floor_nonneg, mul_div_round, DomainEvent, Money1e8, PortfolioSnapshot, PositionState,
    Price1e8, Qty1e8, Rate1e10, Symbol, RATE_SCALE_1E10, SCALE_1E8, UNIVERSE_LEN,
};

pub const MAX_HARD_DD_RATE_1E10: i128 = 2_500_000_000; // 25%
pub const DEFAULT_SPOT_FEE_RATE: Rate1e10 = Rate1e10::from_bps(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotPortfolioLedger {
    cash_1e8: Money1e8,
    positions: [PositionState; UNIVERSE_LEN],
    total_deposits_1e8: Money1e8,
    realized_pnl_gross_1e8: Money1e8,
    cum_sell_fees_usdt_1e8: Money1e8,
    cum_buy_fees_base_1e8: [Qty1e8; UNIVERSE_LEN],
    cum_buy_fees_usdt_est_1e8: Money1e8,
    twr_index_1e10: Rate1e10,
    twr_peak_1e10: Rate1e10,
    last_marked_equity_1e8: Money1e8,
    halted: bool,
}

impl SpotPortfolioLedger {
    pub fn new(initial_cash_1e8: Money1e8) -> Result<Self, DomainError> {
        if initial_cash_1e8.0 < 0 {
            return Err(DomainError::NegativeValueNotAllowed("initial_cash_1e8"));
        }

        Ok(Self {
            cash_1e8: initial_cash_1e8,
            positions: [PositionState::default(); UNIVERSE_LEN],
            total_deposits_1e8: initial_cash_1e8,
            realized_pnl_gross_1e8: Money1e8::ZERO,
            cum_sell_fees_usdt_1e8: Money1e8::ZERO,
            cum_buy_fees_base_1e8: [Qty1e8::ZERO; UNIVERSE_LEN],
            cum_buy_fees_usdt_est_1e8: Money1e8::ZERO,
            twr_index_1e10: Rate1e10::ONE,
            twr_peak_1e10: Rate1e10::ONE,
            last_marked_equity_1e8: initial_cash_1e8,
            halted: false,
        })
    }

    pub fn from_snapshot(snap: PortfolioSnapshot) -> Result<Self, DomainError> {
        if snap.cash_1e8.0 < 0 {
            return Err(DomainError::NegativeValueNotAllowed("cash_1e8"));
        }
        if snap.cum_sell_fees_usdt_1e8.0 < 0 {
            return Err(DomainError::NegativeValueNotAllowed(
                "cum_sell_fees_usdt_1e8",
            ));
        }
        if snap.cum_buy_fees_usdt_est_1e8.0 < 0 {
            return Err(DomainError::NegativeValueNotAllowed(
                "cum_buy_fees_usdt_est_1e8",
            ));
        }
        if snap.twr_index_1e10.0 < 0 {
            return Err(DomainError::NegativeValueNotAllowed("twr_index_1e10"));
        }
        if snap.twr_peak_1e10.0 <= 0 {
            return Err(DomainError::InvalidParameter(
                "twr_peak_1e10 must be positive",
            ));
        }
        if snap.last_marked_equity_1e8.0 < 0 {
            return Err(DomainError::NegativeValueNotAllowed(
                "last_marked_equity_1e8",
            ));
        }

        for i in 0..UNIVERSE_LEN {
            if snap.positions[i].qty_1e8.0 < 0 {
                return Err(DomainError::NegativeValueNotAllowed("position qty_1e8"));
            }
            if snap.positions[i].cost_basis_usdt_1e8.0 < 0 {
                return Err(DomainError::NegativeValueNotAllowed(
                    "position cost_basis_usdt_1e8",
                ));
            }
            if snap.cum_buy_fees_base_1e8[i].0 < 0 {
                return Err(DomainError::NegativeValueNotAllowed(
                    "cum_buy_fees_base_1e8",
                ));
            }
        }

        let ledger = Self {
            cash_1e8: snap.cash_1e8,
            positions: snap.positions,
            total_deposits_1e8: snap.total_deposits_1e8,
            realized_pnl_gross_1e8: snap.realized_pnl_gross_1e8,
            cum_sell_fees_usdt_1e8: snap.cum_sell_fees_usdt_1e8,
            cum_buy_fees_base_1e8: snap.cum_buy_fees_base_1e8,
            cum_buy_fees_usdt_est_1e8: snap.cum_buy_fees_usdt_est_1e8,
            twr_index_1e10: snap.twr_index_1e10,
            twr_peak_1e10: snap.twr_peak_1e10,
            last_marked_equity_1e8: snap.last_marked_equity_1e8,
            halted: snap.halted,
        };
        ledger.assert_book_zero_drift()?;
        Ok(ledger)
    }

    pub fn snapshot(&self) -> PortfolioSnapshot {
        PortfolioSnapshot {
            cash_1e8: self.cash_1e8,
            positions: self.positions,
            total_deposits_1e8: self.total_deposits_1e8,
            realized_pnl_gross_1e8: self.realized_pnl_gross_1e8,
            cum_sell_fees_usdt_1e8: self.cum_sell_fees_usdt_1e8,
            cum_buy_fees_base_1e8: self.cum_buy_fees_base_1e8,
            cum_buy_fees_usdt_est_1e8: self.cum_buy_fees_usdt_est_1e8,
            twr_index_1e10: self.twr_index_1e10,
            twr_peak_1e10: self.twr_peak_1e10,
            last_marked_equity_1e8: self.last_marked_equity_1e8,
            halted: self.halted,
        }
    }

    #[inline]
    pub const fn cash_1e8(&self) -> Money1e8 {
        self.cash_1e8
    }

    #[inline]
    pub const fn position(&self, symbol: Symbol) -> PositionState {
        self.positions[symbol.index()]
    }

    #[inline]
    pub const fn positions(&self) -> &[PositionState; UNIVERSE_LEN] {
        &self.positions
    }

    #[inline]
    pub const fn total_deposits_1e8(&self) -> Money1e8 {
        self.total_deposits_1e8
    }

    #[inline]
    pub const fn realized_pnl_gross_1e8(&self) -> Money1e8 {
        self.realized_pnl_gross_1e8
    }

    #[inline]
    pub const fn realized_pnl_net_1e8(&self) -> Money1e8 {
        Money1e8(
            self.realized_pnl_gross_1e8
                .0
                .saturating_sub(self.cum_sell_fees_usdt_1e8.0),
        )
    }

    #[inline]
    pub const fn cum_sell_fees_usdt_1e8(&self) -> Money1e8 {
        self.cum_sell_fees_usdt_1e8
    }

    #[inline]
    pub const fn cum_buy_fees_usdt_est_1e8(&self) -> Money1e8 {
        self.cum_buy_fees_usdt_est_1e8
    }

    #[inline]
    pub const fn twr_index_1e10(&self) -> Rate1e10 {
        self.twr_index_1e10
    }

    #[inline]
    pub const fn twr_peak_1e10(&self) -> Rate1e10 {
        self.twr_peak_1e10
    }

    #[inline]
    pub const fn is_halted(&self) -> bool {
        self.halted
    }

    #[inline]
    pub fn set_halted(&mut self, halted: bool) {
        self.halted = halted;
    }

    fn position_mtm_values_1e8(
        &self,
        prices: &[Price1e8; UNIVERSE_LEN],
    ) -> Result<[i128; UNIVERSE_LEN], DomainError> {
        let mut vals = [0i128; UNIVERSE_LEN];
        for i in 0..UNIVERSE_LEN {
            let p = prices[i].0;
            if p < 0 {
                return Err(DomainError::NegativeValueNotAllowed("price_1e8"));
            }
            if self.positions[i].qty_1e8.0 > 0 && p == 0 {
                return Err(DomainError::InvalidParameter(
                    "price must be positive for open position",
                ));
            }
            vals[i] = mul_div_floor_nonneg(self.positions[i].qty_1e8.0, p, SCALE_1E8)?;
        }
        Ok(vals)
    }

    pub fn unrealized_pnl_1e8(
        &self,
        prices: &[Price1e8; UNIVERSE_LEN],
    ) -> Result<Money1e8, DomainError> {
        let vals = self.position_mtm_values_1e8(prices)?;
        let mut sum = 0i128;
        for (val, pos) in vals.iter().zip(self.positions.iter()) {
            let diff = val
                .checked_sub(pos.cost_basis_usdt_1e8.0)
                .ok_or(DomainError::ArithmeticOverflow)?;
            sum = sum
                .checked_add(diff)
                .ok_or(DomainError::ArithmeticOverflow)?;
        }
        Ok(Money1e8(sum))
    }

    pub fn equity_1e8(&self, prices: &[Price1e8; UNIVERSE_LEN]) -> Result<Money1e8, DomainError> {
        let vals = self.position_mtm_values_1e8(prices)?;
        let mut sum = self.cash_1e8.0;
        for v in vals {
            sum = sum.checked_add(v).ok_or(DomainError::ArithmeticOverflow)?;
        }
        Ok(Money1e8(sum))
    }

    pub fn current_weights(
        &self,
        prices: &[Price1e8; UNIVERSE_LEN],
    ) -> Result<[f64; UNIVERSE_LEN], DomainError> {
        let vals = self.position_mtm_values_1e8(prices)?;
        let mut equity = self.cash_1e8.0;
        for v in vals {
            equity = equity
                .checked_add(v)
                .ok_or(DomainError::ArithmeticOverflow)?;
        }

        if equity <= 0 {
            return Ok([0.0; UNIVERSE_LEN]);
        }

        let eq_f64 = equity as f64;
        let mut weights = [0.0f64; UNIVERSE_LEN];
        for i in 0..UNIVERSE_LEN {
            weights[i] = (vals[i] as f64) / eq_f64;
        }
        Ok(weights)
    }

    pub fn current_drawdown_1e10(&self) -> Result<i128, DomainError> {
        if self.twr_peak_1e10.0 <= 0 {
            return Err(DomainError::DivisionByZero);
        }
        if self.twr_index_1e10.0 >= self.twr_peak_1e10.0 {
            return Ok(0);
        }
        let drop = self
            .twr_peak_1e10
            .0
            .checked_sub(self.twr_index_1e10.0)
            .ok_or(DomainError::ArithmeticOverflow)?;
        mul_div_floor_nonneg(drop, RATE_SCALE_1E10, self.twr_peak_1e10.0)
    }

    pub fn drift_book_1e8(&self) -> Result<i128, DomainError> {
        let mut total_cost = 0i128;
        for pos in &self.positions {
            total_cost = total_cost
                .checked_add(pos.cost_basis_usdt_1e8.0)
                .ok_or(DomainError::ArithmeticOverflow)?;
        }
        let lhs = self
            .cash_1e8
            .0
            .checked_add(total_cost)
            .ok_or(DomainError::ArithmeticOverflow)?;

        let rhs = self
            .total_deposits_1e8
            .0
            .checked_add(self.realized_pnl_gross_1e8.0)
            .and_then(|v| v.checked_sub(self.cum_sell_fees_usdt_1e8.0))
            .ok_or(DomainError::ArithmeticOverflow)?;

        lhs.checked_sub(rhs).ok_or(DomainError::ArithmeticOverflow)
    }

    pub fn drift_mtm_1e8(&self, prices: &[Price1e8; UNIVERSE_LEN]) -> Result<i128, DomainError> {
        let equity = self.equity_1e8(prices)?.0;
        let unrealized = self.unrealized_pnl_1e8(prices)?.0;
        let realized_net = self
            .realized_pnl_gross_1e8
            .0
            .checked_sub(self.cum_sell_fees_usdt_1e8.0)
            .ok_or(DomainError::ArithmeticOverflow)?;
        let rhs = self
            .total_deposits_1e8
            .0
            .checked_add(realized_net)
            .and_then(|v| v.checked_add(unrealized))
            .ok_or(DomainError::ArithmeticOverflow)?;

        equity
            .checked_sub(rhs)
            .ok_or(DomainError::ArithmeticOverflow)
    }

    fn assert_book_zero_drift(&self) -> Result<(), DomainError> {
        let drift = self.drift_book_1e8()?;
        if drift != 0 {
            return Err(DomainError::AccountingDriftNonZero { drift_1e8: drift });
        }
        Ok(())
    }

    pub fn assert_zero_drift(&self, prices: &[Price1e8; UNIVERSE_LEN]) -> Result<(), DomainError> {
        self.assert_book_zero_drift()?;
        let mtm_drift = self.drift_mtm_1e8(prices)?;
        if mtm_drift != 0 {
            return Err(DomainError::AccountingDriftNonZero {
                drift_1e8: mtm_drift,
            });
        }
        Ok(())
    }

    pub fn apply_buy_fill(
        &mut self,
        symbol: Symbol,
        quote_spent_1e8: Money1e8,
        gross_base_qty_1e8: Qty1e8,
        fee_base_reported_1e8: Qty1e8,
        base_precision: Qty1e8,
    ) -> Result<(), DomainError> {
        if quote_spent_1e8.0 < 0 {
            return Err(DomainError::NegativeValueNotAllowed("quote_spent_1e8"));
        }
        if quote_spent_1e8.0 == 0 {
            return Err(DomainError::InvalidParameter(
                "quote_spent_1e8 must be positive",
            ));
        }
        if gross_base_qty_1e8.0 < 0 {
            return Err(DomainError::NegativeValueNotAllowed("gross_base_qty_1e8"));
        }
        if gross_base_qty_1e8.0 == 0 {
            return Err(DomainError::InvalidParameter(
                "gross_base_qty_1e8 must be positive",
            ));
        }
        if fee_base_reported_1e8.0 < 0 {
            return Err(DomainError::NegativeValueNotAllowed(
                "fee_base_reported_1e8",
            ));
        }
        if fee_base_reported_1e8 > gross_base_qty_1e8 {
            return Err(DomainError::InvalidParameter(
                "buy fee cannot exceed gross base qty",
            ));
        }
        if self.cash_1e8 < quote_spent_1e8 {
            return Err(DomainError::InsufficientCash {
                required: quote_spent_1e8.0,
                available: self.cash_1e8.0,
            });
        }

        let net_qty = gross_base_qty_1e8
            .checked_sub(fee_base_reported_1e8)?
            .floor_to_step(base_precision)?;
        let fee_usdt_est = Money1e8(mul_div_round(
            quote_spent_1e8.0,
            fee_base_reported_1e8.0,
            gross_base_qty_1e8.0,
        )?);

        let idx = symbol.index();
        let new_cash = self.cash_1e8.checked_sub(quote_spent_1e8)?;
        let new_qty = self.positions[idx].qty_1e8.checked_add(net_qty)?;
        let new_cost = self.positions[idx]
            .cost_basis_usdt_1e8
            .checked_add(quote_spent_1e8)?;
        let new_cum_fee_base =
            self.cum_buy_fees_base_1e8[idx].checked_add(fee_base_reported_1e8)?;
        let new_cum_fee_usdt_est = self.cum_buy_fees_usdt_est_1e8.checked_add(fee_usdt_est)?;

        self.cash_1e8 = new_cash;
        self.positions[idx].qty_1e8 = new_qty;
        self.positions[idx].cost_basis_usdt_1e8 = new_cost;
        self.cum_buy_fees_base_1e8[idx] = new_cum_fee_base;
        self.cum_buy_fees_usdt_est_1e8 = new_cum_fee_usdt_est;

        self.assert_book_zero_drift()
    }

    pub fn apply_sell_fill(
        &mut self,
        symbol: Symbol,
        sell_qty_1e8: Qty1e8,
        gross_quote_1e8: Money1e8,
        fee_usdt_1e8: Money1e8,
    ) -> Result<(), DomainError> {
        if sell_qty_1e8.0 < 0 {
            return Err(DomainError::NegativeValueNotAllowed("sell_qty_1e8"));
        }
        if sell_qty_1e8.0 == 0 {
            return Err(DomainError::InvalidParameter(
                "sell_qty_1e8 must be positive",
            ));
        }
        if gross_quote_1e8.0 < 0 {
            return Err(DomainError::NegativeValueNotAllowed("gross_quote_1e8"));
        }
        if fee_usdt_1e8.0 < 0 {
            return Err(DomainError::NegativeValueNotAllowed("fee_usdt_1e8"));
        }

        let idx = symbol.index();
        let pos = self.positions[idx];
        if sell_qty_1e8 > pos.qty_1e8 {
            return Err(DomainError::InsufficientPosition {
                symbol: symbol.to_string(),
                required: sell_qty_1e8.0,
                available: pos.qty_1e8.0,
            });
        }

        let released_cost = if sell_qty_1e8 == pos.qty_1e8 {
            pos.cost_basis_usdt_1e8
        } else {
            Money1e8(mul_div_round(
                pos.cost_basis_usdt_1e8.0,
                sell_qty_1e8.0,
                pos.qty_1e8.0,
            )?)
        };

        let new_qty = pos.qty_1e8.checked_sub(sell_qty_1e8)?;
        let new_cost = pos.cost_basis_usdt_1e8.checked_sub(released_cost)?;
        let net_proceeds = gross_quote_1e8.checked_sub(fee_usdt_1e8)?;
        let new_cash = self.cash_1e8.checked_add(net_proceeds)?;
        if new_cash.0 < 0 {
            return Err(DomainError::InsufficientCash {
                required: fee_usdt_1e8.0,
                available: self.cash_1e8.0.saturating_add(gross_quote_1e8.0),
            });
        }

        let trade_pnl_gross = gross_quote_1e8.checked_sub(released_cost)?;
        let new_realized_gross = self.realized_pnl_gross_1e8.checked_add(trade_pnl_gross)?;
        let new_cum_sell_fees = self.cum_sell_fees_usdt_1e8.checked_add(fee_usdt_1e8)?;

        self.positions[idx].qty_1e8 = new_qty;
        self.positions[idx].cost_basis_usdt_1e8 = new_cost;
        self.cash_1e8 = new_cash;
        self.realized_pnl_gross_1e8 = new_realized_gross;
        self.cum_sell_fees_usdt_1e8 = new_cum_sell_fees;

        self.assert_book_zero_drift()
    }

    pub fn record_external_deposit(
        &mut self,
        amount_1e8: Money1e8,
        prices: &[Price1e8; UNIVERSE_LEN],
    ) -> Result<Vec<DomainEvent>, DomainError> {
        if amount_1e8.0 < 0 {
            return Err(DomainError::NegativeValueNotAllowed("deposit amount_1e8"));
        }
        if amount_1e8.0 == 0 {
            return Err(DomainError::InvalidParameter(
                "deposit amount_1e8 must be positive",
            ));
        }

        let mut events = Vec::new();
        if let Some(halt_event) = self.mark_to_market(prices)? {
            events.push(halt_event);
        }

        let new_cash = self.cash_1e8.checked_add(amount_1e8)?;
        let new_deposits = self.total_deposits_1e8.checked_add(amount_1e8)?;
        let new_last_eq = self.last_marked_equity_1e8.checked_add(amount_1e8)?;

        self.cash_1e8 = new_cash;
        self.total_deposits_1e8 = new_deposits;
        self.last_marked_equity_1e8 = new_last_eq;

        self.assert_zero_drift(prices)?;
        events.push(DomainEvent::ExternalDepositRecorded {
            delta_1e8: amount_1e8,
            new_total_deposits_1e8: new_deposits,
        });
        Ok(events)
    }

    pub fn record_external_withdrawal(
        &mut self,
        amount_1e8: Money1e8,
        prices: &[Price1e8; UNIVERSE_LEN],
    ) -> Result<Option<DomainEvent>, DomainError> {
        if amount_1e8.0 < 0 {
            return Err(DomainError::NegativeValueNotAllowed(
                "withdrawal amount_1e8",
            ));
        }
        if amount_1e8.0 == 0 {
            return Err(DomainError::InvalidParameter(
                "withdrawal amount_1e8 must be positive",
            ));
        }
        if self.cash_1e8 < amount_1e8 {
            return Err(DomainError::InsufficientCash {
                required: amount_1e8.0,
                available: self.cash_1e8.0,
            });
        }

        let halt_event = self.mark_to_market(prices)?;

        let new_cash = self.cash_1e8.checked_sub(amount_1e8)?;
        let new_deposits = self.total_deposits_1e8.checked_sub(amount_1e8)?;
        let new_last_eq = self
            .last_marked_equity_1e8
            .checked_sub(amount_1e8)?
            .0
            .max(0);

        self.cash_1e8 = new_cash;
        self.total_deposits_1e8 = new_deposits;
        self.last_marked_equity_1e8 = Money1e8(new_last_eq);

        self.assert_zero_drift(prices)?;
        Ok(halt_event)
    }

    pub fn mark_to_market(
        &mut self,
        prices: &[Price1e8; UNIVERSE_LEN],
    ) -> Result<Option<DomainEvent>, DomainError> {
        self.assert_zero_drift(prices)?;
        let equity = self.equity_1e8(prices)?;

        if self.last_marked_equity_1e8.0 > 0 {
            let updated_twr = mul_div_round(
                self.twr_index_1e10.0,
                equity.0,
                self.last_marked_equity_1e8.0,
            )?;
            self.twr_index_1e10 = Rate1e10(updated_twr.max(0));
            if self.twr_index_1e10 > self.twr_peak_1e10 {
                self.twr_peak_1e10 = self.twr_index_1e10;
            }
        }
        self.last_marked_equity_1e8 = equity;

        let dd = self.current_drawdown_1e10()?;
        if dd > MAX_HARD_DD_RATE_1E10 {
            self.halted = true;
            return Ok(Some(DomainEvent::HardDrawdownHalt {
                drawdown_1e10: dd,
                twr_index_1e10: self.twr_index_1e10.0,
                twr_peak_1e10: self.twr_peak_1e10.0,
                equity_1e8: equity,
            }));
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_prices(v: i128) -> [Price1e8; UNIVERSE_LEN] {
        [Price1e8(v); UNIVERSE_LEN]
    }

    #[test]
    fn buy_sell_fee_asymmetry_and_zero_drift() -> Result<(), DomainError> {
        let initial = Money1e8(100_000_000_000); // 1000 USDT
        let mut ledger = SpotPortfolioLedger::new(initial)?;
        let prices = unit_prices(10_000_000_000); // 100 USDT

        // Buy 500 USDT worth -> 5.00 gross base, 0.005 base fee, step 0.001 => net 4.995
        ledger.apply_buy_fill(
            Symbol::BtcUsdt,
            Money1e8(50_000_000_000),
            Qty1e8(500_000_000),
            Qty1e8(500_000),
            Qty1e8(100_000),
        )?;

        assert_eq!(ledger.cash_1e8(), Money1e8(50_000_000_000));
        assert_eq!(
            ledger.position(Symbol::BtcUsdt).qty_1e8,
            Qty1e8(499_500_000)
        );
        assert_eq!(
            ledger.position(Symbol::BtcUsdt).cost_basis_usdt_1e8,
            Money1e8(50_000_000_000)
        );
        assert_eq!(ledger.cum_buy_fees_usdt_est_1e8(), Money1e8(50_000_000));
        assert_eq!(ledger.drift_book_1e8()?, 0);
        ledger.assert_zero_drift(&prices)?;

        // Partial sell 2.00 BTC at 120 USDT => gross 240, fee 0.24 USDT
        let sell_prices = [
            Price1e8(12_000_000_000),
            Price1e8(10_000_000_000),
            Price1e8(10_000_000_000),
            Price1e8(10_000_000_000),
            Price1e8(10_000_000_000),
        ];
        ledger.apply_sell_fill(
            Symbol::BtcUsdt,
            Qty1e8(200_000_000),
            Money1e8(24_000_000_000),
            Money1e8(24_000_000),
        )?;

        assert_eq!(ledger.drift_book_1e8()?, 0);
        assert_eq!(ledger.drift_mtm_1e8(&sell_prices)?, 0);
        ledger.assert_zero_drift(&sell_prices)?;

        // Full sell of remaining BTC
        let rem = ledger.position(Symbol::BtcUsdt).qty_1e8;
        ledger.apply_sell_fill(
            Symbol::BtcUsdt,
            rem,
            Money1e8(35_940_000_000),
            Money1e8(35_940_000),
        )?;
        assert_eq!(ledger.position(Symbol::BtcUsdt).qty_1e8, Qty1e8::ZERO);
        assert_eq!(
            ledger.position(Symbol::BtcUsdt).cost_basis_usdt_1e8,
            Money1e8::ZERO
        );
        ledger.assert_zero_drift(&sell_prices)?;
        Ok(())
    }

    #[test]
    fn deposit_and_withdrawal_preserve_twr_and_drift() -> Result<(), DomainError> {
        let mut ledger = SpotPortfolioLedger::new(Money1e8(100_000_000_000))?; // 1000
        let prices = unit_prices(10_000_000_000); // 100

        // Buy 500 USDT of ETH (5.0 ETH, 0 fee for clean MTM math)
        ledger.apply_buy_fill(
            Symbol::EthUsdt,
            Money1e8(50_000_000_000),
            Qty1e8(500_000_000),
            Qty1e8::ZERO,
            Qty1e8(100_000),
        )?;
        ledger.assert_zero_drift(&prices)?;

        // Price doubles to 200 -> equity = 500 cash + 1000 ETH = 1500 -> TWR = 1.5
        let up_prices = unit_prices(20_000_000_000);
        let ev = ledger.mark_to_market(&up_prices)?;
        assert!(ev.is_none());
        assert_eq!(ledger.twr_index_1e10(), Rate1e10(15_000_000_000));
        assert_eq!(ledger.twr_peak_1e10(), Rate1e10(15_000_000_000));

        // External deposit of +500 USDT must NOT distort TWR!
        let dep_events = ledger.record_external_deposit(Money1e8(50_000_000_000), &up_prices)?;
        assert_eq!(dep_events.len(), 1);
        assert_eq!(ledger.twr_index_1e10(), Rate1e10(15_000_000_000));
        assert_eq!(ledger.equity_1e8(&up_prices)?, Money1e8(200_000_000_000));
        ledger.assert_zero_drift(&up_prices)?;

        // External withdrawal of 250 USDT must NOT distort TWR!
        let w_ev = ledger.record_external_withdrawal(Money1e8(25_000_000_000), &up_prices)?;
        assert!(w_ev.is_none());
        assert_eq!(ledger.twr_index_1e10(), Rate1e10(15_000_000_000));
        assert_eq!(ledger.equity_1e8(&up_prices)?, Money1e8(175_000_000_000));
        ledger.assert_zero_drift(&up_prices)?;
        Ok(())
    }

    #[test]
    fn hard_drawdown_halt_triggers_above_25_percent() -> Result<(), DomainError> {
        let mut ledger = SpotPortfolioLedger::new(Money1e8(100_000_000_000))?; // 1000
        ledger.apply_buy_fill(
            Symbol::SolUsdt,
            Money1e8(100_000_000_000),
            Qty1e8(1_000_000_000),
            Qty1e8::ZERO,
            Qty1e8(100_000),
        )?;

        // Drop 25.00% -> 75 -> NOT > 25%, no halt
        let p_25 = unit_prices(7_500_000_000);
        assert!(ledger.mark_to_market(&p_25)?.is_none());
        assert!(!ledger.is_halted());

        // Drop to 74 -> 26% DD -> triggers HardDrawdownHalt
        let p_26 = unit_prices(7_400_000_000);
        let ev = ledger.mark_to_market(&p_26)?;
        assert!(matches!(ev, Some(DomainEvent::HardDrawdownHalt { .. })));
        assert!(ledger.is_halted());
        Ok(())
    }
}
