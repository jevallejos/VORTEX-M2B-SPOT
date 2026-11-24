use crate::domain::error::DomainError;
use crate::domain::types::Candle;

pub const C_GK: f64 = 2.0 * std::f64::consts::LN_2 - 1.0;
pub const SIGMA_BAR_FLOOR: f64 = 1e-4;
#[allow(clippy::approx_constant)]
pub const PHI: f64 = 1.618_033_988_749_895;

pub const B2_01_N_EFF: usize = 84;
pub const B2_01_BARS_PER_DAY: f64 = 24.0;
pub const B2_01_LOOKBACKS: [(usize, f64); 3] = [(72, 0.25), (168, 0.35), (504, 0.40)];

pub const A0_4H_N_EFF: usize = 21;
pub const A0_4H_BARS_PER_DAY: f64 = 6.0;
pub const A0_4H_LOOKBACKS: [(usize, f64); 3] = [(18, 0.25), (42, 0.35), (126, 0.40)];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolEstimate {
    pub sigma_bar: f64,
    pub sigma_ann: f64,
}

pub fn garman_klass_ewma_vol(
    candles: &[Candle],
    n_eff: usize,
    bars_per_day: f64,
) -> Result<Vec<VolEstimate>, DomainError> {
    if candles.is_empty() {
        return Err(DomainError::InsufficientHistory {
            required: 1,
            actual: 0,
        });
    }
    if n_eff == 0 {
        return Err(DomainError::InvalidParameter("n_eff must be > 0"));
    }
    if !bars_per_day.is_finite() || bars_per_day <= 0.0 {
        return Err(DomainError::InvalidParameter(
            "bars_per_day must be finite and positive",
        ));
    }

    let alpha = 2.0 / ((n_eff as f64) + 1.0);
    let one_minus_alpha = 1.0 - alpha;
    let ann_factor = (bars_per_day * 365.0).sqrt();

    let mut out = Vec::with_capacity(candles.len());
    let mut ewma = 0.0f64;

    for (idx, c) in candles.iter().enumerate() {
        c.validate()?;

        let hl = (c.high / c.low).ln();
        let co = (c.close / c.open).ln();
        let v_t = (0.5 * hl * hl - C_GK * co * co).max(0.0);
        if !v_t.is_finite() {
            return Err(DomainError::InvalidCandle(
                "non-finite Garman-Klass variance".to_string(),
            ));
        }

        ewma = if idx == 0 {
            v_t
        } else {
            alpha * v_t + one_minus_alpha * ewma
        };

        let sigma_bar = ewma.max(0.0).sqrt().max(SIGMA_BAR_FLOOR);
        let sigma_ann = sigma_bar * ann_factor;
        if !sigma_bar.is_finite() || !sigma_ann.is_finite() {
            return Err(DomainError::InvalidCandle(
                "non-finite volatility estimate".to_string(),
            ));
        }

        out.push(VolEstimate {
            sigma_bar,
            sigma_ann,
        });
    }

    Ok(out)
}

fn validate_lookbacks(lookbacks: &[(usize, f64)]) -> Result<usize, DomainError> {
    if lookbacks.is_empty() {
        return Err(DomainError::InvalidParameter("lookbacks must not be empty"));
    }
    let mut max_k = 0usize;
    for &(k, weight) in lookbacks {
        if k == 0 {
            return Err(DomainError::InvalidParameter("lookback k must be > 0"));
        }
        if !weight.is_finite() || weight < 0.0 {
            return Err(DomainError::InvalidParameter(
                "lookback weight must be finite and non-negative",
            ));
        }
        if k > max_k {
            max_k = k;
        }
    }
    Ok(max_k)
}

pub fn tsmom_multi_horizon_at(
    closes: &[f64],
    sigma_bar: &[f64],
    lookbacks: &[(usize, f64)],
    idx: usize,
) -> Result<f64, DomainError> {
    if closes.len() != sigma_bar.len() {
        return Err(DomainError::InvalidParameter(
            "closes and sigma_bar length mismatch",
        ));
    }
    let max_k = validate_lookbacks(lookbacks)?;

    if idx >= closes.len() {
        return Err(DomainError::InsufficientHistory {
            required: idx.saturating_add(1),
            actual: closes.len(),
        });
    }
    if idx < max_k {
        return Err(DomainError::InsufficientHistory {
            required: max_k + 1,
            actual: idx + 1,
        });
    }

    let c_now = closes[idx];
    let s_now = sigma_bar[idx];
    if !c_now.is_finite() || c_now <= 0.0 {
        return Err(DomainError::InvalidParameter(
            "current close must be finite and positive",
        ));
    }
    if !s_now.is_finite() || s_now <= 0.0 {
        return Err(DomainError::InvalidParameter(
            "current sigma_bar must be finite and positive",
        ));
    }

    let mut signal = 0.0f64;
    for &(k, weight) in lookbacks {
        let c_prev = closes[idx - k];
        if !c_prev.is_finite() || c_prev <= 0.0 {
            return Err(DomainError::InvalidParameter(
                "lookback close must be finite and positive",
            ));
        }
        let z_k = (c_now / c_prev).ln() / (s_now * (k as f64).sqrt());
        if !z_k.is_finite() {
            return Err(DomainError::InvalidParameter("non-finite z-score"));
        }
        signal += weight * (z_k / PHI).tanh();
    }

    Ok(signal)
}

pub fn tsmom_multi_horizon(
    closes: &[f64],
    sigma_bar: &[f64],
    lookbacks: &[(usize, f64)],
) -> Result<Vec<Option<f64>>, DomainError> {
    if closes.len() != sigma_bar.len() {
        return Err(DomainError::InvalidParameter(
            "closes and sigma_bar length mismatch",
        ));
    }
    let max_k = validate_lookbacks(lookbacks)?;

    let mut out = Vec::with_capacity(closes.len());
    for idx in 0..closes.len() {
        if idx < max_k {
            out.push(None);
        } else {
            let val = tsmom_multi_horizon_at(closes, sigma_bar, lookbacks, idx)?;
            out.push(Some(val));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_candle(ts: i64, open: f64, high: f64, low: f64, close: f64) -> Candle {
        Candle {
            open_ts_ms: ts,
            open,
            high,
            low,
            close,
            volume: 10.0,
            turnover: 1000.0,
        }
    }

    #[test]
    fn gk_ewma_rejects_invalid_inputs() {
        assert_eq!(
            garman_klass_ewma_vol(&[], B2_01_N_EFF, B2_01_BARS_PER_DAY),
            Err(DomainError::InsufficientHistory {
                required: 1,
                actual: 0
            })
        );
        let c = [mk_candle(0, 100.0, 105.0, 95.0, 102.0)];
        assert_eq!(
            garman_klass_ewma_vol(&c, 0, 24.0),
            Err(DomainError::InvalidParameter("n_eff must be > 0"))
        );
        assert_eq!(
            garman_klass_ewma_vol(&c, 10, 0.0),
            Err(DomainError::InvalidParameter(
                "bars_per_day must be finite and positive"
            ))
        );
    }

    #[test]
    fn gk_ewma_respects_floor_and_causal_update() -> Result<(), DomainError> {
        let flat = [
            mk_candle(0, 100.0, 100.0, 100.0, 100.0),
            mk_candle(3_600_000, 100.0, 102.0, 99.0, 101.0),
        ];
        let vols = garman_klass_ewma_vol(&flat, B2_01_N_EFF, B2_01_BARS_PER_DAY)?;
        assert_eq!(vols.len(), 2);
        assert!((vols[0].sigma_bar - SIGMA_BAR_FLOOR).abs() < 1e-12);
        assert!(vols[1].sigma_bar > SIGMA_BAR_FLOOR);
        let expected_ann = vols[1].sigma_bar * (24.0 * 365.0f64).sqrt();
        assert!((vols[1].sigma_ann - expected_ann).abs() < 1e-12);
        Ok(())
    }

    #[test]
    fn tsmom_multi_horizon_works_and_checks_history() -> Result<(), DomainError> {
        let closes = vec![100.0, 102.0, 105.0, 110.0, 115.0];
        let sigmas = vec![0.02; 5];
        let lookbacks = [(1, 0.4), (3, 0.6)];

        let err = tsmom_multi_horizon_at(&closes, &sigmas, &lookbacks, 2);
        assert_eq!(
            err,
            Err(DomainError::InsufficientHistory {
                required: 4,
                actual: 3
            })
        );

        let val = tsmom_multi_horizon_at(&closes, &sigmas, &lookbacks, 4)?;
        let z1 = (115.0f64 / 110.0).ln() / (0.02 * 1.0f64.sqrt());
        let z3 = (115.0f64 / 102.0).ln() / (0.02 * 3.0f64.sqrt());
        let expected = 0.4 * (z1 / PHI).tanh() + 0.6 * (z3 / PHI).tanh();
        assert!((val - expected).abs() < 1e-12);

        let series = tsmom_multi_horizon(&closes, &sigmas, &lookbacks)?;
        assert_eq!(series.len(), 5);
        assert!(series[0].is_none());
        assert!(series[2].is_none());
        assert!(series[3].is_some());
        assert_eq!(series[4], Some(val));
        Ok(())
    }
}
