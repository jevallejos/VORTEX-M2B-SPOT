use crate::domain::accounting::SpotPortfolioLedger;
use crate::domain::error::DomainError;
use crate::domain::types::{
    InstrumentRules, Money1e8, Price1e8, Qty1e8, Symbol, WalletSnapshot, UNIVERSE_LEN,
};

pub const EXTERNAL_CASH_TOLERANCE_1E8: i128 = 100_000; // 0.001 USDT

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseDiscrepancy {
    pub symbol: Symbol,
    pub ledger_qty_1e8: Qty1e8,
    pub wallet_qty_1e8: Qty1e8,
    pub diff_1e8: i128,
    pub base_precision_1e8: Qty1e8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    pub external_cash_delta_1e8: Money1e8,
    pub base_discrepancies: Vec<BaseDiscrepancy>,
}

pub fn reconcile_live_wallet(
    ledger: &mut SpotPortfolioLedger,
    wallet: &WalletSnapshot,
    rules: &[InstrumentRules; UNIVERSE_LEN],
    prices: &[Price1e8; UNIVERSE_LEN],
    had_recent_orders: bool,
) -> Result<ReconcileReport, DomainError> {
    let wallet_usdt = wallet.usdt_free_1e8.checked_add(wallet.usdt_locked_1e8)?;
    let delta = wallet_usdt
        .raw()
        .checked_sub(ledger.cash_1e8().raw())
        .ok_or(DomainError::ArithmeticOverflow)?;

    let mut external_cash_delta_1e8 = Money1e8::ZERO;
    if !had_recent_orders && delta.abs() > EXTERNAL_CASH_TOLERANCE_1E8 {
        if delta > 0 {
            let dep = Money1e8(delta);
            let _ = ledger.record_external_deposit(dep, prices)?;
            external_cash_delta_1e8 = dep;
        } else {
            let wdr_abs = delta.checked_neg().ok_or(DomainError::ArithmeticOverflow)?;
            let _ = ledger.record_external_withdrawal(Money1e8(wdr_abs), prices)?;
            external_cash_delta_1e8 = Money1e8(delta);
        }
    }

    let mut base_discrepancies = Vec::new();
    for (i, sym) in Symbol::ALL.iter().copied().enumerate() {
        let wallet_coin = wallet.coin_free_1e8[i].checked_add(wallet.coin_locked_1e8[i])?;
        let ledger_qty = ledger.position(sym).qty_1e8;
        let diff_1e8 = wallet_coin
            .raw()
            .checked_sub(ledger_qty.raw())
            .ok_or(DomainError::ArithmeticOverflow)?;
        if diff_1e8.abs() > rules[i].base_precision.raw() {
            base_discrepancies.push(BaseDiscrepancy {
                symbol: sym,
                ledger_qty_1e8: ledger_qty,
                wallet_qty_1e8: wallet_coin,
                diff_1e8,
                base_precision_1e8: rules[i].base_precision,
            });
        }
    }

    Ok(ReconcileReport {
        external_cash_delta_1e8,
        base_discrepancies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_rules() -> [InstrumentRules; UNIVERSE_LEN] {
        Symbol::ALL.map(|sym| InstrumentRules {
            symbol: sym,
            base_precision: Qty1e8(100_000), // 0.001
            quote_precision: Money1e8(1_000_000),
            min_order_qty: Qty1e8(100_000),
            min_order_amt: Money1e8(500_000_000),
            tick_size: Price1e8(1_000_000),
            is_trading: true,
        })
    }

    #[test]
    fn reconcile_detects_deposit_and_base_mismatch() -> Result<(), DomainError> {
        let mut ledger = SpotPortfolioLedger::new(Money1e8(100_000_000_000))?;
        let rules = mk_rules();
        let prices = [Price1e8(10_000_000_000); UNIVERSE_LEN];

        let wallet = WalletSnapshot {
            usdt_free_1e8: Money1e8(125_000_000_000), // +250 USDT external deposit
            usdt_locked_1e8: Money1e8::ZERO,
            coin_free_1e8: [
                Qty1e8(500_000), // 0.005 > 0.001 precision -> mismatch!
                Qty1e8::ZERO,
                Qty1e8::ZERO,
                Qty1e8::ZERO,
                Qty1e8::ZERO,
            ],
            coin_locked_1e8: [Qty1e8::ZERO; UNIVERSE_LEN],
            fetched_at_ms: 1_700_000_000_000,
        };

        let report = reconcile_live_wallet(&mut ledger, &wallet, &rules, &prices, false)?;
        assert_eq!(report.external_cash_delta_1e8, Money1e8(25_000_000_000));
        assert_eq!(ledger.cash_1e8(), Money1e8(125_000_000_000));
        assert_eq!(report.base_discrepancies.len(), 1);
        assert_eq!(report.base_discrepancies[0].symbol, Symbol::BtcUsdt);
        Ok(())
    }
}
