use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    UnknownSymbol(String),
    InvalidDecimal(String),
    ArithmeticOverflow,
    DivisionByZero,
    NegativeValueNotAllowed(&'static str),
    InvalidCandle(String),
    InsufficientHistory {
        required: usize,
        actual: usize,
    },
    InvalidParameter(&'static str),
    InsufficientCash {
        required: i128,
        available: i128,
    },
    InsufficientPosition {
        symbol: String,
        required: i128,
        available: i128,
    },
    AccountingDriftNonZero {
        drift_1e8: i128,
    },
    InvalidInstrumentRules(String),
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSymbol(s) => write!(f, "unknown symbol: {s}"),
            Self::InvalidDecimal(s) => write!(f, "invalid decimal string: {s}"),
            Self::ArithmeticOverflow => write!(f, "checked i128 arithmetic overflow"),
            Self::DivisionByZero => write!(f, "division by zero"),
            Self::NegativeValueNotAllowed(ctx) => write!(f, "negative value not allowed: {ctx}"),
            Self::InvalidCandle(msg) => write!(f, "invalid candle: {msg}"),
            Self::InsufficientHistory { required, actual } => {
                write!(
                    f,
                    "insufficient history: required {required}, actual {actual}"
                )
            }
            Self::InvalidParameter(msg) => write!(f, "invalid parameter: {msg}"),
            Self::InsufficientCash {
                required,
                available,
            } => {
                write!(
                    f,
                    "insufficient cash: required {required}, available {available}"
                )
            }
            Self::InsufficientPosition {
                symbol,
                required,
                available,
            } => {
                write!(
                    f,
                    "insufficient position for {symbol}: required {required}, available {available}"
                )
            }
            Self::AccountingDriftNonZero { drift_1e8 } => {
                write!(f, "accounting invariant violated: drift_1e8 = {drift_1e8}")
            }
            Self::InvalidInstrumentRules(msg) => write!(f, "invalid instrument rules: {msg}"),
        }
    }
}

impl std::error::Error for DomainError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfraError {
    Http(String),
    ExchangeApi { code: i64, msg: String },
    Parse(String),
    Io(String),
    LockBusy(String),
    Config(String),
    Notifier(String),
    ClockSkew { skew_ms: i64, max_allowed_ms: i64 },
}

impl fmt::Display for InfraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(msg) => write!(f, "http error: {msg}"),
            Self::ExchangeApi { code, msg } => write!(f, "exchange api error ({code}): {msg}"),
            Self::Parse(msg) => write!(f, "parse error: {msg}"),
            Self::Io(msg) => write!(f, "io error: {msg}"),
            Self::LockBusy(msg) => write!(f, "lock busy: {msg}"),
            Self::Config(msg) => write!(f, "config error: {msg}"),
            Self::Notifier(msg) => write!(f, "notifier error: {msg}"),
            Self::ClockSkew {
                skew_ms,
                max_allowed_ms,
            } => {
                write!(
                    f,
                    "clock skew {skew_ms}ms exceeds max allowed {max_allowed_ms}ms"
                )
            }
        }
    }
}

impl std::error::Error for InfraError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppError {
    Domain(DomainError),
    Infra(InfraError),
    LiveSafetyLockDenied(String),
    HaltLockActive(String),
    ReconciliationMismatch(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Domain(e) => write!(f, "domain error: {e}"),
            Self::Infra(e) => write!(f, "infrastructure error: {e}"),
            Self::LiveSafetyLockDenied(msg) => write!(f, "live safety lock denied: {msg}"),
            Self::HaltLockActive(msg) => write!(f, "halt lock active: {msg}"),
            Self::ReconciliationMismatch(msg) => write!(f, "reconciliation mismatch: {msg}"),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Domain(e) => Some(e),
            Self::Infra(e) => Some(e),
            Self::LiveSafetyLockDenied(_)
            | Self::HaltLockActive(_)
            | Self::ReconciliationMismatch(_) => None,
        }
    }
}

impl From<DomainError> for AppError {
    fn from(value: DomainError) -> Self {
        Self::Domain(value)
    }
}

impl From<InfraError> for AppError {
    fn from(value: InfraError) -> Self {
        Self::Infra(value)
    }
}
