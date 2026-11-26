use crate::domain::accounting::SpotPortfolioLedger;
use crate::domain::error::DomainError;
use crate::domain::types::{
    AssetSymbol, InstrumentRules, Money1e8, Price1e8, Qty1e8, Universe, WalletSnapshot,
};

pub const EXTERNAL_CASH_TOLERANCE_1E8: i128 = 100_000; // 0.001 USDT

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseDiscrepancy {
    pub symbol: AssetSymbol,
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

fn check_len(field: &'static str, expected: usize, actual: usize) -> Result<(), DomainError> {
    if actual != expected {
        return Err(DomainError::CorruptStateLength {
            field,
            expected,
            actual,
        });
    }
    Ok(())
}

pub fn reconcile_live_wallet(
    universe: &Universe,
    ledger: &mut SpotPortfolioLedger,
    wallet: &WalletSnapshot,
    rules: &[InstrumentRules],
    prices: &[Price1e8],
    had_recent_orders: bool,
) -> Result<ReconcileReport, DomainError> {
    let n = universe.len();
    check_len("ledger.num_assets", n, ledger.num_assets())?;
    check_len("wallet.coin_free_1e8", n, wallet.coin_free_1e8.len())?;
    check_len("wallet.coin_locked_1e8", n, wallet.coin_locked_1e8.len())?;
    check_len("rules", n, rules.len())?;
    check_len("prices_1e8", n, prices.len())?;

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
    for (id, sym) in universe.iter_with_ids() {
        let i = id.index();
        if rules[i].symbol != *sym {
            return Err(DomainError::InvalidInstrumentRules(format!(
                "reconcile rules[{i}] symbol '{}' does not match universe symbol '{sym}'",
                rules[i].symbol
            )));
        }
        let wallet_coin = wallet.coin_free_1e8[i].checked_add(wallet.coin_locked_1e8[i])?;
        let ledger_qty = ledger.position(id)?.qty_1e8;
        let diff_1e8 = wallet_coin
            .raw()
            .checked_sub(ledger_qty.raw())
            .ok_or(DomainError::ArithmeticOverflow)?;
        if diff_1e8.abs() > rules[i].base_precision.raw() {
            base_discrepancies.push(BaseDiscrepancy {
                symbol: sym.clone(),
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

    fn mk_rules(universe: &Universe) -> Vec<InstrumentRules> {
        universe
            .symbols()
            .iter()
            .map(|sym| InstrumentRules {
                symbol: sym.clone(),
                base_precision: Qty1e8(100_000), // 0.001
                quote_precision: Money1e8(1_000_000),
                min_order_qty: Qty1e8(100_000),
                min_order_amt: Money1e8(500_000_000),
                tick_size: Price1e8(1_000_000),
                is_trading: true,
            })
            .collect()
    }

    #[test]
    fn reconcile_detects_deposit_and_base_mismatch() -> Result<(), DomainError> {
        let universe = Universe::default_universe()?;
        let n = universe.len();
        let mut ledger = SpotPortfolioLedger::new(Money1e8(100_000_000_000), n)?;
        let rules = mk_rules(&universe);
        let prices = vec![Price1e8(10_000_000_000); n];

        let mut coin_free = vec![Qty1e8::ZERO; n];
        coin_free[0] = Qty1e8(500_000); // BTC mismatch
        coin_free[5] = Qty1e8(300_000); // XAUT mismatch

        let wallet = WalletSnapshot {
            usdt_free_1e8: Money1e8(125_000_000_000), // +250 USDT external deposit
            usdt_locked_1e8: Money1e8::ZERO,
            coin_free_1e8: coin_free,
            coin_locked_1e8: vec![Qty1e8::ZERO; n],
            fetched_at_ms: 1_700_000_000_000,
        };

        let report =
            reconcile_live_wallet(&universe, &mut ledger, &wallet, &rules, &prices, false)?;
        assert_eq!(report.external_cash_delta_1e8, Money1e8(25_000_000_000));
        assert_eq!(ledger.cash_1e8(), Money1e8(125_000_000_000));
        assert_eq!(report.base_discrepancies.len(), 2);
        assert_eq!(report.base_discrepancies[0].symbol.as_str(), "BTCUSDT");
        assert_eq!(report.base_discrepancies[1].symbol.as_str(), "XAUTUSDT");
        Ok(())
    }
}
