use std::fmt;

use crate::domain::error::DomainError;

pub const SCALE_1E8: i128 = 100_000_000;
pub const RATE_SCALE_1E10: i128 = 10_000_000_000;
pub const ONE_HOUR_MS: i64 = 3_600_000;
pub const FOUR_HOURS_MS: i64 = 14_400_000;

pub const STATE_SCHEMA_VERSION: u32 = 2;
pub const MIN_UNIVERSE_ASSETS: usize = 1;
pub const MAX_UNIVERSE_ASSETS: usize = 20;
pub const MIN_SYMBOL_LEN: usize = 6;
pub const MAX_SYMBOL_LEN: usize = 20;
pub const DEFAULT_UNIVERSE_SYMBOLS: [&str; 6] = [
    "BTCUSDT", "ETHUSDT", "SOLUSDT", "LINKUSDT", "AVAXUSDT", "XAUTUSDT",
];
pub const DEFAULT_UNIVERSE_CSV: &str = "BTCUSDT,ETHUSDT,SOLUSDT,LINKUSDT,AVAXUSDT,XAUTUSDT";

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolId(pub usize);

impl SymbolId {
    #[inline]
    pub const fn index(self) -> usize {
        self.0
    }
}

impl fmt::Display for SymbolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetSymbol(String);

impl AssetSymbol {
    pub fn new(raw: &str) -> Result<Self, DomainError> {
        if !(MIN_SYMBOL_LEN..=MAX_SYMBOL_LEN).contains(&raw.len()) {
            return Err(DomainError::InvalidUniverse(format!(
                "symbol '{raw}' length {} out of bounds ({MIN_SYMBOL_LEN}..={MAX_SYMBOL_LEN})",
                raw.len()
            )));
        }
        if !raw
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return Err(DomainError::InvalidUniverse(format!(
                "symbol '{raw}' must be uppercase ASCII alphanumeric"
            )));
        }
        if raw == "USDT" || !raw.ends_with("USDT") {
            return Err(DomainError::InvalidUniverse(format!(
                "symbol '{raw}' must end with 'USDT' and have a non-empty base coin"
            )));
        }
        Ok(Self(raw.to_string()))
    }

    pub fn parse_normalized(raw: &str) -> Result<Self, DomainError> {
        let normalized = raw.trim().to_ascii_uppercase();
        Self::new(&normalized)
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[inline]
    pub fn base_coin(&self) -> &str {
        self.0.strip_suffix("USDT").unwrap_or(self.0.as_str())
    }
}

impl fmt::Display for AssetSymbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Universe {
    symbols: Vec<AssetSymbol>,
}

impl Universe {
    pub fn new(symbols: Vec<AssetSymbol>) -> Result<Self, DomainError> {
        if symbols.len() < MIN_UNIVERSE_ASSETS || symbols.len() > MAX_UNIVERSE_ASSETS {
            return Err(DomainError::InvalidUniverse(format!(
                "universe size {} out of bounds ({MIN_UNIVERSE_ASSETS}..={MAX_UNIVERSE_ASSETS})",
                symbols.len()
            )));
        }
        for i in 0..symbols.len() {
            for j in (i + 1)..symbols.len() {
                if symbols[i] == symbols[j] {
                    return Err(DomainError::InvalidUniverse(format!(
                        "duplicate symbol '{}' in universe",
                        symbols[i]
                    )));
                }
            }
        }
        Ok(Self { symbols })
    }

    pub fn default_universe() -> Result<Self, DomainError> {
        Self::from_strict_strs(&DEFAULT_UNIVERSE_SYMBOLS)
    }

    pub fn from_strict_strs<S: AsRef<str>>(raw: &[S]) -> Result<Self, DomainError> {
        let mut parsed = Vec::with_capacity(raw.len());
        for s in raw {
            parsed.push(AssetSymbol::new(s.as_ref())?);
        }
        Self::new(parsed)
    }

    pub fn parse_csv(csv: &str) -> Result<Self, DomainError> {
        let trimmed = csv.trim();
        if trimmed.is_empty() {
            return Err(DomainError::InvalidUniverse(
                "universe symbol list must not be empty".to_string(),
            ));
        }
        let mut parsed = Vec::new();
        for token in trimmed.split(',') {
            let tok = token.trim();
            if tok.is_empty() {
                return Err(DomainError::InvalidUniverse(format!(
                    "empty symbol entry in universe list '{csv}'"
                )));
            }
            parsed.push(AssetSymbol::parse_normalized(tok)?);
        }
        Self::new(parsed)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    #[inline]
    pub fn symbols(&self) -> &[AssetSymbol] {
        &self.symbols
    }

    pub fn as_strings(&self) -> Vec<String> {
        self.symbols
            .iter()
            .map(|s| s.as_str().to_string())
            .collect()
    }

    pub fn as_csv(&self) -> String {
        self.as_strings().join(",")
    }

    #[inline]
    pub fn get(&self, id: SymbolId) -> Option<&AssetSymbol> {
        self.symbols.get(id.index())
    }

    pub fn symbol_id(&self, symbol: &AssetSymbol) -> Option<SymbolId> {
        self.symbols.iter().position(|s| s == symbol).map(SymbolId)
    }

    pub fn symbol_id_by_str(&self, s: &str) -> Option<SymbolId> {
        self.symbols
            .iter()
            .position(|sym| sym.as_str() == s)
            .map(SymbolId)
    }

    pub fn iter_with_ids(&self) -> impl Iterator<Item = (SymbolId, &AssetSymbol)> {
        self.symbols
            .iter()
            .enumerate()
            .map(|(i, s)| (SymbolId(i), s))
    }
}

impl fmt::Display for Universe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_csv())
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Money1e8(pub i128);

impl Money1e8 {
    pub const ZERO: Self = Self(0);

    #[inline]
    pub const fn raw(self) -> i128 {
        self.0
    }

    pub fn checked_add(self, rhs: Self) -> Result<Self, DomainError> {
        self.0
            .checked_add(rhs.0)
            .map(Self)
            .ok_or(DomainError::ArithmeticOverflow)
    }

    pub fn checked_sub(self, rhs: Self) -> Result<Self, DomainError> {
        self.0
            .checked_sub(rhs.0)
            .map(Self)
            .ok_or(DomainError::ArithmeticOverflow)
    }

    pub fn checked_neg(self) -> Result<Self, DomainError> {
        self.0
            .checked_neg()
            .map(Self)
            .ok_or(DomainError::ArithmeticOverflow)
    }

    pub fn from_decimal_str(s: &str) -> Result<Self, DomainError> {
        parse_decimal_1e8(s).map(Self)
    }

    pub fn to_f64(self) -> f64 {
        (self.0 as f64) / (SCALE_1E8 as f64)
    }

    pub fn format_fixed(self, decimals: usize) -> String {
        format_scaled_1e8(self.0, decimals)
    }
}

impl fmt::Display for Money1e8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format_fixed(8))
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Qty1e8(pub i128);

impl Qty1e8 {
    pub const ZERO: Self = Self(0);

    #[inline]
    pub const fn raw(self) -> i128 {
        self.0
    }

    pub fn checked_add(self, rhs: Self) -> Result<Self, DomainError> {
        self.0
            .checked_add(rhs.0)
            .map(Self)
            .ok_or(DomainError::ArithmeticOverflow)
    }

    pub fn checked_sub(self, rhs: Self) -> Result<Self, DomainError> {
        self.0
            .checked_sub(rhs.0)
            .map(Self)
            .ok_or(DomainError::ArithmeticOverflow)
    }

    pub fn from_decimal_str(s: &str) -> Result<Self, DomainError> {
        parse_decimal_1e8(s).map(Self)
    }

    pub fn to_f64(self) -> f64 {
        (self.0 as f64) / (SCALE_1E8 as f64)
    }

    pub fn floor_to_step(self, step: Self) -> Result<Self, DomainError> {
        if step.0 <= 0 {
            return Err(DomainError::InvalidInstrumentRules(
                "qty step must be positive".to_string(),
            ));
        }
        if self.0 < 0 {
            return Err(DomainError::NegativeValueNotAllowed("qty floor_to_step"));
        }
        Ok(Self((self.0 / step.0) * step.0))
    }

    pub fn format_by_step(self, step: Self) -> String {
        let mut s = step.0;
        if s <= 0 {
            return format_scaled_1e8(self.0, 8);
        }
        let mut decimals = 8usize;
        while decimals > 0 && s % 10 == 0 {
            s /= 10;
            decimals -= 1;
        }
        format_scaled_1e8(self.0, decimals)
    }

    pub fn format_fixed(self, decimals: usize) -> String {
        format_scaled_1e8(self.0, decimals)
    }
}

impl fmt::Display for Qty1e8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format_fixed(8))
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Price1e8(pub i128);

impl Price1e8 {
    pub const ZERO: Self = Self(0);

    #[inline]
    pub const fn raw(self) -> i128 {
        self.0
    }

    pub fn from_decimal_str(s: &str) -> Result<Self, DomainError> {
        parse_decimal_1e8(s).map(Self)
    }

    pub fn from_f64_checked(v: f64) -> Result<Self, DomainError> {
        if !v.is_finite() || v <= 0.0 {
            return Err(DomainError::InvalidParameter(
                "price must be finite and positive",
            ));
        }
        let scaled = (v * (SCALE_1E8 as f64)).round();
        if !scaled.is_finite() || scaled <= 0.0 || scaled > (i128::MAX as f64) {
            return Err(DomainError::ArithmeticOverflow);
        }
        Ok(Self(scaled as i128))
    }

    pub fn to_f64(self) -> f64 {
        (self.0 as f64) / (SCALE_1E8 as f64)
    }

    pub fn format_fixed(self, decimals: usize) -> String {
        format_scaled_1e8(self.0, decimals)
    }
}

impl fmt::Display for Price1e8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format_fixed(8))
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Rate1e10(pub i128);

impl Rate1e10 {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(RATE_SCALE_1E10);

    #[inline]
    pub const fn from_bps(bps: i128) -> Self {
        Self(bps * 1_000_000)
    }

    #[inline]
    pub const fn raw(self) -> i128 {
        self.0
    }

    pub fn to_f64(self) -> f64 {
        (self.0 as f64) / (RATE_SCALE_1E10 as f64)
    }
}

pub fn mul_div_round(a: i128, b: i128, denom: i128) -> Result<i128, DomainError> {
    if denom == 0 {
        return Err(DomainError::DivisionByZero);
    }
    if denom < 0 {
        return Err(DomainError::NegativeValueNotAllowed(
            "mul_div_round denominator",
        ));
    }
    let prod = a.checked_mul(b).ok_or(DomainError::ArithmeticOverflow)?;
    let half = denom / 2;
    let adjusted = if prod >= 0 {
        prod.checked_add(half)
            .ok_or(DomainError::ArithmeticOverflow)?
    } else {
        prod.checked_sub(half)
            .ok_or(DomainError::ArithmeticOverflow)?
    };
    Ok(adjusted / denom)
}

pub fn mul_div_floor_nonneg(a: i128, b: i128, denom: i128) -> Result<i128, DomainError> {
    if denom == 0 {
        return Err(DomainError::DivisionByZero);
    }
    if a < 0 || b < 0 || denom < 0 {
        return Err(DomainError::NegativeValueNotAllowed("mul_div_floor_nonneg"));
    }
    let prod = a.checked_mul(b).ok_or(DomainError::ArithmeticOverflow)?;
    Ok(prod / denom)
}

pub fn parse_decimal_1e8(s: &str) -> Result<i128, DomainError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(DomainError::InvalidDecimal(s.to_string()));
    }

    let (neg, rest) = if let Some(r) = trimmed.strip_prefix('-') {
        (true, r)
    } else if let Some(r) = trimmed.strip_prefix('+') {
        (false, r)
    } else {
        (false, trimmed)
    };

    if rest.is_empty() {
        return Err(DomainError::InvalidDecimal(s.to_string()));
    }

    let (mantissa, exp_shift) = match rest.split_once(['e', 'E']) {
        Some((m, e)) => {
            if m.is_empty() || e.is_empty() {
                return Err(DomainError::InvalidDecimal(s.to_string()));
            }
            let exp: i32 = e
                .parse()
                .map_err(|_| DomainError::InvalidDecimal(s.to_string()))?;
            (m, exp)
        }
        None => (rest, 0i32),
    };

    let (int_part, frac_part) = match mantissa.split_once('.') {
        Some((i, f)) => {
            if f.contains('.') {
                return Err(DomainError::InvalidDecimal(s.to_string()));
            }
            (i, f)
        }
        None => (mantissa, ""),
    };

    if int_part.is_empty() && frac_part.is_empty() {
        return Err(DomainError::InvalidDecimal(s.to_string()));
    }
    if !int_part.bytes().all(|b| b.is_ascii_digit())
        || !frac_part.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(DomainError::InvalidDecimal(s.to_string()));
    }

    let mut digits = String::with_capacity(int_part.len() + frac_part.len());
    digits.push_str(int_part);
    digits.push_str(frac_part);

    let frac_len_i32 =
        i32::try_from(frac_part.len()).map_err(|_| DomainError::ArithmeticOverflow)?;
    let total_pow10 = 8i32
        .checked_add(exp_shift)
        .and_then(|v| v.checked_sub(frac_len_i32))
        .ok_or(DomainError::ArithmeticOverflow)?;

    let mut acc: i128 = 0;
    for b in digits.bytes() {
        let d = i128::from(b - b'0');
        acc = acc
            .checked_mul(10)
            .and_then(|v| v.checked_add(d))
            .ok_or(DomainError::ArithmeticOverflow)?;
    }

    if total_pow10 >= 0 {
        for _ in 0..total_pow10 {
            acc = acc.checked_mul(10).ok_or(DomainError::ArithmeticOverflow)?;
        }
    } else {
        for _ in 0..(-total_pow10) {
            acc /= 10;
        }
    }

    if neg {
        acc.checked_neg().ok_or(DomainError::ArithmeticOverflow)
    } else {
        Ok(acc)
    }
}

pub fn format_scaled_1e8(val: i128, decimals: usize) -> String {
    let neg = val < 0;
    let abs = val.unsigned_abs();
    let int_part = abs / 100_000_000u128;
    let frac_part = abs % 100_000_000u128;
    let sign = if neg && val != 0 { "-" } else { "" };

    if decimals == 0 {
        return format!("{sign}{int_part}");
    }

    let capped = decimals.min(8);
    let full_frac = format!("{frac_part:08}");
    let slice = &full_frac[..capped];
    format!("{sign}{int_part}.{slice}")
}

pub fn fnv1a64_hex(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

pub fn make_order_link_id(closed_hour_ts_ms: i64, symbol: &AssetSymbol, side: OrderSide) -> String {
    let full = format!(
        "m2b_{}_{}_{}",
        closed_hour_ts_ms,
        symbol.as_str(),
        side.as_lower()
    );
    if full.len() <= 36 {
        full
    } else {
        let digest = fnv1a64_hex(symbol.as_str().as_bytes());
        let short_hash = digest.get(..8).unwrap_or(&digest);
        let side_code = match side {
            OrderSide::Buy => "b",
            OrderSide::Sell => "s",
        };
        format!("m2b_{closed_hour_ts_ms}_{short_hash}_{side_code}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candle {
    pub open_ts_ms: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub turnover: f64,
}

impl Candle {
    #[inline]
    pub const fn close_ts_ms(self, interval_ms: i64) -> i64 {
        self.open_ts_ms + interval_ms
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.open_ts_ms < 0 {
            return Err(DomainError::InvalidCandle(
                "negative open_ts_ms".to_string(),
            ));
        }
        let prices = [self.open, self.high, self.low, self.close];
        if prices.iter().any(|p| !p.is_finite() || *p <= 0.0) {
            return Err(DomainError::InvalidCandle(
                "non-finite or non-positive OHLC".to_string(),
            ));
        }
        if !self.volume.is_finite() || self.volume < 0.0 {
            return Err(DomainError::InvalidCandle("invalid volume".to_string()));
        }
        if !self.turnover.is_finite() || self.turnover < 0.0 {
            return Err(DomainError::InvalidCandle("invalid turnover".to_string()));
        }
        if self.high < self.low {
            return Err(DomainError::InvalidCandle("high < low".to_string()));
        }
        if self.high < self.open || self.high < self.close {
            return Err(DomainError::InvalidCandle(
                "high below open or close".to_string(),
            ));
        }
        if self.low > self.open || self.low > self.close {
            return Err(DomainError::InvalidCandle(
                "low above open or close".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentRules {
    pub symbol: AssetSymbol,
    pub base_precision: Qty1e8,
    pub quote_precision: Money1e8,
    pub min_order_qty: Qty1e8,
    pub min_order_amt: Money1e8,
    pub tick_size: Price1e8,
    pub is_trading: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickerSnapshot {
    pub symbol: AssetSymbol,
    pub bid1: Price1e8,
    pub ask1: Price1e8,
    pub last: Price1e8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderSide {
    Buy,
    Sell,
}

impl OrderSide {
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "Buy",
            Self::Sell => "Sell",
        }
    }

    #[inline]
    pub const fn as_lower(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }
}

impl fmt::Display for OrderSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Paper,
    LiveSpot,
}

impl ExecutionMode {
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Paper => "paper",
            Self::LiveSpot => "live-spot",
        }
    }
}

impl fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedOrder {
    pub symbol_id: SymbolId,
    pub symbol: AssetSymbol,
    pub side: OrderSide,
    pub quote_spend_1e8: Money1e8,
    pub base_qty_1e8: Qty1e8,
    pub reference_price_1e8: Price1e8,
    pub current_weight: f64,
    pub target_weight: f64,
    pub full_liquidation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderRequest {
    pub symbol: AssetSymbol,
    pub side: OrderSide,
    pub quote_spend_1e8: Money1e8,
    pub base_qty_1e8: Qty1e8,
    pub base_precision: Qty1e8,
    pub order_link_id: String,
    pub reference_price_1e8: Price1e8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FillReport {
    pub symbol: AssetSymbol,
    pub side: OrderSide,
    pub order_id: String,
    pub order_link_id: String,
    pub exec_qty_base_1e8: Qty1e8,
    pub exec_quote_1e8: Money1e8,
    pub avg_price_1e8: Price1e8,
    pub fee_1e8: i128,
    pub fee_in_base: bool,
    pub filled_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletSnapshot {
    pub usdt_free_1e8: Money1e8,
    pub usdt_locked_1e8: Money1e8,
    pub coin_free_1e8: Vec<Qty1e8>,
    pub coin_locked_1e8: Vec<Qty1e8>,
    pub fetched_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEvent {
    HardDrawdownHalt {
        drawdown_1e10: i128,
        twr_index_1e10: i128,
        twr_peak_1e10: i128,
        equity_1e8: Money1e8,
    },
    ExternalDepositRecorded {
        delta_1e8: Money1e8,
        new_total_deposits_1e8: Money1e8,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PositionState {
    pub qty_1e8: Qty1e8,
    pub cost_basis_usdt_1e8: Money1e8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioSnapshot {
    pub cash_1e8: Money1e8,
    pub positions: Vec<PositionState>,
    pub total_deposits_1e8: Money1e8,
    pub realized_pnl_gross_1e8: Money1e8,
    pub cum_sell_fees_usdt_1e8: Money1e8,
    pub cum_buy_fees_base_1e8: Vec<Qty1e8>,
    pub cum_buy_fees_usdt_est_1e8: Money1e8,
    pub twr_index_1e10: Rate1e10,
    pub twr_peak_1e10: Rate1e10,
    pub last_marked_equity_1e8: Money1e8,
    pub halted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnginePersistedState {
    pub schema_version: u32,
    pub symbols: Vec<AssetSymbol>,
    pub mode: ExecutionMode,
    pub last_processed_hour_ts_ms: i64,
    pub last_daily_summary_day_utc: i64,
    pub last_order_ts_ms: i64,
    pub portfolio: PortfolioSnapshot,
    pub last_target_weights: Vec<f64>,
    pub last_current_weights: Vec<f64>,
    pub last_sigma_ann: Vec<f64>,
    pub last_tsmom: Vec<f64>,
    pub last_prices_1e8: Vec<Price1e8>,
    pub last_drift_1e8: i128,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shadow4hPersistedState {
    pub schema_version: u32,
    pub symbols: Vec<AssetSymbol>,
    pub last_processed_4h_close_ts_ms: i64,
    pub portfolio: PortfolioSnapshot,
    pub last_target_weights: Vec<f64>,
    pub last_current_weights: Vec<f64>,
    pub last_sigma_ann: Vec<f64>,
    pub last_tsmom: Vec<f64>,
    pub last_prices_1e8: Vec<Price1e8>,
    pub last_drift_1e8: i128,
}

fn validate_persisted_state_fields(
    schema_version: u32,
    symbols: &[AssetSymbol],
    expected: &Universe,
    checks: [(&'static str, usize); 7],
) -> Result<(), DomainError> {
    if schema_version != STATE_SCHEMA_VERSION {
        return Err(DomainError::SchemaVersionMismatch {
            expected: STATE_SCHEMA_VERSION,
            actual: schema_version,
        });
    }
    if symbols != expected.symbols() {
        return Err(DomainError::UniverseMismatch {
            expected: expected.as_strings(),
            actual: symbols.iter().map(|s| s.as_str().to_string()).collect(),
        });
    }

    let n = expected.len();
    for (field, actual) in checks {
        if actual != n {
            return Err(DomainError::CorruptStateLength {
                field,
                expected: n,
                actual,
            });
        }
    }
    Ok(())
}

impl EnginePersistedState {
    pub fn validate_against(&self, expected: &Universe) -> Result<(), DomainError> {
        validate_persisted_state_fields(
            self.schema_version,
            &self.symbols,
            expected,
            [
                ("portfolio.positions", self.portfolio.positions.len()),
                (
                    "portfolio.cum_buy_fees_base_1e8",
                    self.portfolio.cum_buy_fees_base_1e8.len(),
                ),
                ("last_target_weights", self.last_target_weights.len()),
                ("last_current_weights", self.last_current_weights.len()),
                ("last_sigma_ann", self.last_sigma_ann.len()),
                ("last_tsmom", self.last_tsmom.len()),
                ("last_prices_1e8", self.last_prices_1e8.len()),
            ],
        )
    }
}

impl Shadow4hPersistedState {
    pub fn validate_against(&self, expected: &Universe) -> Result<(), DomainError> {
        validate_persisted_state_fields(
            self.schema_version,
            &self.symbols,
            expected,
            [
                ("portfolio.positions", self.portfolio.positions.len()),
                (
                    "portfolio.cum_buy_fees_base_1e8",
                    self.portfolio.cum_buy_fees_base_1e8.len(),
                ),
                ("last_target_weights", self.last_target_weights.len()),
                ("last_current_weights", self.last_current_weights.len()),
                ("last_sigma_ann", self.last_sigma_ann.len()),
                ("last_tsmom", self.last_tsmom.len()),
                ("last_prices_1e8", self.last_prices_1e8.len()),
            ],
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub ts_ms: i64,
    pub engine: String,
    pub event_type: String,
    pub mode: String,
    pub hour_ts_ms: i64,
    pub symbol: String,
    pub side: String,
    pub order_link_id: String,
    pub qty_1e8: i128,
    pub quote_1e8: i128,
    pub price_1e8: i128,
    pub fee_1e8: i128,
    pub fee_in_base: bool,
    pub equity_1e8: i128,
    pub drift_1e8: i128,
    pub detail: String,
}
