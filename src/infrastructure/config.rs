use std::collections::HashMap;
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::error::InfraError;
use crate::domain::types::{Money1e8, Universe};

pub const LIVE_ACK_MAGIC: &str = "YES_I_ACCEPT_0X_SPOT_RISK";
pub const DEFAULT_BYBIT_BASE_URL: &str = "https://api.bybit.com";
pub const DEFAULT_RECV_WINDOW_MS: u64 = 5_000;
pub const DEFAULT_MAX_CLOCK_SKEW_MS: i64 = 2_000;
pub const DEFAULT_PAPER_INITIAL_CASH_USDT: &str = "1000.00";
pub const DEFAULT_STATE_DIR: &str = "state";
pub const DEFAULT_LOGS_DIR: &str = "logs";

#[derive(Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub bybit_base_url: String,
    pub api_key: String,
    pub api_secret: String,
    pub recv_window_ms: u64,
    pub live_execution_env_flag: bool,
    pub live_ack_valid: bool,
    pub paper_initial_cash_1e8: Money1e8,
    pub telegram_bot_token: String,
    pub telegram_chat_id: String,
    pub state_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub max_clock_skew_ms: i64,
    pub universe: Universe,
}

impl fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppConfig")
            .field("bybit_base_url", &self.bybit_base_url)
            .field("api_key", &redact_secret(&self.api_key))
            .field("api_secret", &redact_secret(&self.api_secret))
            .field("recv_window_ms", &self.recv_window_ms)
            .field("live_execution_env_flag", &self.live_execution_env_flag)
            .field("live_ack_valid", &self.live_ack_valid)
            .field("paper_initial_cash_1e8", &self.paper_initial_cash_1e8)
            .field(
                "telegram_bot_token",
                &redact_secret(&self.telegram_bot_token),
            )
            .field("telegram_chat_id", &self.telegram_chat_id)
            .field("state_dir", &self.state_dir)
            .field("logs_dir", &self.logs_dir)
            .field("max_clock_skew_ms", &self.max_clock_skew_ms)
            .field("universe", &self.universe.as_csv())
            .finish()
    }
}

fn redact_secret(value: &str) -> &'static str {
    if value.is_empty() {
        ""
    } else {
        "[REDACTED]"
    }
}

impl AppConfig {
    pub fn load() -> Result<Self, InfraError> {
        let dotenv = Path::new(".env");
        if dotenv.exists() {
            Self::load_with_dotenv(Some(dotenv))
        } else {
            Self::load_with_dotenv(None)
        }
    }

    pub fn load_with_dotenv(dotenv_path: Option<&Path>) -> Result<Self, InfraError> {
        let mut merged: HashMap<String, String> = HashMap::new();

        if let Some(path) = dotenv_path {
            let content = fs::read_to_string(path)
                .map_err(|e| InfraError::Io(format!("failed to read {}: {e}", path.display())))?;
            let parsed = parse_dotenv(&content)?;
            for (k, v) in parsed {
                merged.insert(k, v);
            }
        }

        for (k, v) in env::vars() {
            merged.insert(k, v);
        }

        Self::from_env_map(&merged)
    }

    pub fn from_env_map(map: &HashMap<String, String>) -> Result<Self, InfraError> {
        let bybit_base_url = non_empty_or(map.get("BYBIT_BASE_URL"), DEFAULT_BYBIT_BASE_URL)
            .trim_end_matches('/')
            .to_string();
        if !bybit_base_url.starts_with("https://") && !bybit_base_url.starts_with("http://") {
            return Err(InfraError::Config(format!(
                "BYBIT_BASE_URL must start with http:// or https://, got {bybit_base_url}"
            )));
        }

        let api_key = map
            .get("BYBIT_API_KEY")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let api_secret = map
            .get("BYBIT_API_SECRET")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let recv_window_ms = match map.get("BYBIT_RECV_WINDOW_MS").map(|s| s.trim()) {
            Some(s) if !s.is_empty() => s.parse::<u64>().map_err(|e| {
                InfraError::Config(format!("invalid BYBIT_RECV_WINDOW_MS '{s}': {e}"))
            })?,
            _ => DEFAULT_RECV_WINDOW_MS,
        };
        if recv_window_ms == 0 {
            return Err(InfraError::Config(
                "BYBIT_RECV_WINDOW_MS must be greater than 0".to_string(),
            ));
        }

        let live_execution_env_flag = match map.get("BYBIT_SPOT_LIVE_EXECUTION").map(|s| s.trim()) {
            Some(s) if !s.is_empty() => parse_bool_flag("BYBIT_SPOT_LIVE_EXECUTION", s)?,
            _ => false,
        };

        let live_ack_valid = map
            .get("BYBIT_SPOT_ACK_LIVE")
            .map(|s| s.trim() == LIVE_ACK_MAGIC)
            .unwrap_or(false);

        let cash_str = non_empty_or(
            map.get("PAPER_INITIAL_CASH_USDT"),
            DEFAULT_PAPER_INITIAL_CASH_USDT,
        );
        let paper_initial_cash_1e8 = Money1e8::from_decimal_str(&cash_str).map_err(|e| {
            InfraError::Config(format!("invalid PAPER_INITIAL_CASH_USDT '{cash_str}': {e}"))
        })?;
        if paper_initial_cash_1e8.raw() <= 0 {
            return Err(InfraError::Config(
                "PAPER_INITIAL_CASH_USDT must be strictly positive".to_string(),
            ));
        }

        let telegram_bot_token = non_empty_opt(map.get("TELEGRAM_BOT_TOKEN"))
            .or_else(|| non_empty_opt(map.get("TELEGRAM_TOKEN")))
            .unwrap_or_default();

        let telegram_chat_id = map
            .get("TELEGRAM_CHAT_ID")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let state_dir = PathBuf::from(non_empty_or(map.get("VORTEX_STATE_DIR"), DEFAULT_STATE_DIR));
        let logs_dir = PathBuf::from(non_empty_or(map.get("VORTEX_LOGS_DIR"), DEFAULT_LOGS_DIR));

        let max_clock_skew_ms = match map.get("MAX_CLOCK_SKEW_MS").map(|s| s.trim()) {
            Some(s) if !s.is_empty() => s
                .parse::<i64>()
                .map_err(|e| InfraError::Config(format!("invalid MAX_CLOCK_SKEW_MS '{s}': {e}")))?,
            _ => DEFAULT_MAX_CLOCK_SKEW_MS,
        };
        if max_clock_skew_ms <= 0 {
            return Err(InfraError::Config(
                "MAX_CLOCK_SKEW_MS must be strictly positive".to_string(),
            ));
        }

        let universe = match non_empty_opt(map.get("M2B_UNIVERSE_SYMBOLS")) {
            Some(raw) => Universe::parse_csv(&raw).map_err(|e| {
                InfraError::Config(format!("invalid M2B_UNIVERSE_SYMBOLS '{raw}': {e}"))
            })?,
            None => Universe::default_universe()
                .map_err(|e| InfraError::Config(format!("invalid default universe: {e}")))?,
        };

        Ok(Self {
            bybit_base_url,
            api_key,
            api_secret,
            recv_window_ms,
            live_execution_env_flag,
            live_ack_valid,
            paper_initial_cash_1e8,
            telegram_bot_token,
            telegram_chat_id,
            state_dir,
            logs_dir,
            max_clock_skew_ms,
            universe,
        })
    }
}

fn non_empty_opt(val: Option<&String>) -> Option<String> {
    val.map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn non_empty_or(val: Option<&String>, default: &str) -> String {
    non_empty_opt(val).unwrap_or_else(|| default.to_string())
}

fn parse_bool_flag(name: &str, raw: &str) -> Result<bool, InfraError> {
    if raw.eq_ignore_ascii_case("true") || raw == "1" || raw.eq_ignore_ascii_case("yes") {
        Ok(true)
    } else if raw.eq_ignore_ascii_case("false") || raw == "0" || raw.eq_ignore_ascii_case("no") {
        Ok(false)
    } else {
        Err(InfraError::Config(format!(
            "invalid boolean value for {name}: '{raw}'"
        )))
    }
}

pub fn parse_dotenv(content: &str) -> Result<HashMap<String, String>, InfraError> {
    let mut out = HashMap::new();
    for (idx, raw_line) in content.lines().enumerate() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let line = trimmed
            .strip_prefix("export ")
            .map(str::trim_start)
            .unwrap_or(trimmed);

        let Some((raw_key, raw_val)) = line.split_once('=') else {
            return Err(InfraError::Config(format!(
                "malformed .env line {}: missing '='",
                idx + 1
            )));
        };

        let key = raw_key.trim();
        if key.is_empty() {
            return Err(InfraError::Config(format!(
                "malformed .env line {}: empty key",
                idx + 1
            )));
        }

        let val_trimmed = raw_val.trim();
        let is_double_quoted =
            val_trimmed.len() >= 2 && val_trimmed.starts_with('"') && val_trimmed.ends_with('"');
        let is_single_quoted =
            val_trimmed.len() >= 2 && val_trimmed.starts_with('\'') && val_trimmed.ends_with('\'');
        let value = if is_double_quoted || is_single_quoted {
            val_trimmed[1..val_trimmed.len() - 1].to_string()
        } else {
            strip_inline_comment(val_trimmed).trim().to_string()
        };

        out.insert(key.to_string(), value);
    }
    Ok(out)
}

fn strip_inline_comment(s: &str) -> &str {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            return &s[..i];
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::types::DEFAULT_UNIVERSE_CSV;

    #[test]
    fn test_dotenv_parser_and_token_fallback() {
        let raw = "\
# comment
BYBIT_BASE_URL=https://api.bybit.com
TELEGRAM_TOKEN=\"legacy_tok\"
PAPER_INITIAL_CASH_USDT=2500.50 # inline comment
BYBIT_SPOT_ACK_LIVE='YES_I_ACCEPT_0X_SPOT_RISK'
";
        let map = parse_dotenv(raw).ok().unwrap_or_default();
        let cfg = AppConfig::from_env_map(&map);
        assert!(cfg.is_ok());
        if let Ok(c) = cfg {
            assert_eq!(c.telegram_bot_token, "legacy_tok");
            assert_eq!(c.paper_initial_cash_1e8, Money1e8(250_050_000_000));
            assert!(c.live_ack_valid);
            assert_eq!(c.universe.as_csv(), DEFAULT_UNIVERSE_CSV);
            assert_eq!(c.universe.len(), 6);
            let dbg = format!("{c:?}");
            assert!(!dbg.contains("legacy_tok"));
            assert!(dbg.contains("[REDACTED]"));
            assert!(dbg.contains("XAUTUSDT"));
        }
    }

    #[test]
    fn test_universe_env_parsing_and_fail_closed_validation() {
        let mut map = HashMap::new();
        map.insert(
            "M2B_UNIVERSE_SYMBOLS".to_string(),
            " btcusdt , xautusdt ".to_string(),
        );
        let cfg = AppConfig::from_env_map(&map);
        assert!(cfg.is_ok());
        if let Ok(c) = cfg {
            assert_eq!(c.universe.as_csv(), "BTCUSDT,XAUTUSDT");
            assert_eq!(c.universe.len(), 2);
        }

        let mut empty_val = HashMap::new();
        empty_val.insert("M2B_UNIVERSE_SYMBOLS".to_string(), "   ".to_string());
        let cfg_default = AppConfig::from_env_map(&empty_val);
        assert!(cfg_default.is_ok());
        if let Ok(c) = cfg_default {
            assert_eq!(c.universe.as_csv(), DEFAULT_UNIVERSE_CSV);
        }

        let mut dup = HashMap::new();
        dup.insert(
            "M2B_UNIVERSE_SYMBOLS".to_string(),
            "BTCUSDT,btcusdt".to_string(),
        );
        assert!(matches!(
            AppConfig::from_env_map(&dup),
            Err(InfraError::Config(_))
        ));
    }
}
