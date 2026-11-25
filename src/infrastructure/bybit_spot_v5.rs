use std::fmt;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::domain::error::InfraError;
use crate::domain::ports::{ClockPort, MarketDataPort, SpotExecutionPort};
use crate::domain::types::{
    parse_decimal_1e8, Candle, FillReport, InstrumentRules, Money1e8, OrderRequest, OrderSide,
    Price1e8, Qty1e8, Symbol, TickerSnapshot, WalletSnapshot, UNIVERSE_LEN,
};
use crate::infrastructure::config::AppConfig;

type HmacSha256 = Hmac<Sha256>;

const GET_RETRY_BACKOFF_SECS: [u64; 3] = [1, 2, 4];
const RESULT_PREFIX_MAX_CHARS: usize = 160;
const MAX_FILL_POLL_ATTEMPTS: usize = 6;

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl ClockPort for SystemClock {
    fn now_ms(&self) -> i64 {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        i64::try_from(millis).unwrap_or(i64::MAX)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PaperNoopExecutionPort;

impl SpotExecutionPort for PaperNoopExecutionPort {
    fn fetch_wallet_snapshot(&self) -> Result<WalletSnapshot, InfraError> {
        Err(InfraError::Config(
            "PaperNoopExecutionPort::fetch_wallet_snapshot called in paper mode".to_string(),
        ))
    }

    fn query_order_by_link_id(
        &self,
        _symbol: Symbol,
        _order_link_id: &str,
    ) -> Result<Option<FillReport>, InfraError> {
        Err(InfraError::Config(
            "PaperNoopExecutionPort::query_order_by_link_id called in paper mode".to_string(),
        ))
    }

    fn execute_spot_order(&self, _req: &OrderRequest) -> Result<FillReport, InfraError> {
        Err(InfraError::Config(
            "PaperNoopExecutionPort::execute_spot_order called in paper mode".to_string(),
        ))
    }
}

pub fn sign_bybit_v5(
    timestamp_ms: i64,
    api_key: &str,
    recv_window_ms: u64,
    payload: &str,
    api_secret: &str,
) -> Result<String, InfraError> {
    if api_key.is_empty() || api_secret.is_empty() {
        return Err(InfraError::Config(
            "BYBIT_API_KEY and BYBIT_API_SECRET must be non-empty for private endpoints"
                .to_string(),
        ));
    }
    let sign_input = format!("{timestamp_ms}{api_key}{recv_window_ms}{payload}");
    let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes())
        .map_err(|e| InfraError::Config(format!("invalid HMAC secret: {e}")))?;
    mac.update(sign_input.as_bytes());
    let bytes = mac.finalize().into_bytes();
    Ok(hex::encode(bytes))
}

#[derive(Clone)]
pub struct BybitSpotV5Client {
    agent: ureq::Agent,
    base_url: String,
    api_key: String,
    api_secret: String,
    recv_window_ms: u64,
    clock: SystemClock,
}

impl fmt::Debug for BybitSpotV5Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BybitSpotV5Client")
            .field("base_url", &self.base_url)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    ""
                } else {
                    "[REDACTED]"
                },
            )
            .field(
                "api_secret",
                &if self.api_secret.is_empty() {
                    ""
                } else {
                    "[REDACTED]"
                },
            )
            .field("recv_window_ms", &self.recv_window_ms)
            .finish()
    }
}

impl BybitSpotV5Client {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        api_secret: impl Into<String>,
        recv_window_ms: u64,
    ) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout_read(Duration::from_secs(10))
            .build();
        Self {
            agent,
            base_url: base_url.into().trim().trim_end_matches('/').to_string(),
            api_key: api_key.into().trim().to_string(),
            api_secret: api_secret.into().trim().to_string(),
            recv_window_ms,
            clock: SystemClock,
        }
    }

    pub fn from_config(cfg: &AppConfig) -> Self {
        Self::new(
            cfg.bybit_base_url.clone(),
            cfg.api_key.clone(),
            cfg.api_secret.clone(),
            cfg.recv_window_ms,
        )
    }

    fn get_public_envelope<T: for<'de> Deserialize<'de>>(
        &self,
        path_and_query: &str,
    ) -> Result<BybitEnvelope<T>, InfraError> {
        let url = format!("{}{}", self.base_url, path_and_query);
        retry_idempotent_get(|| {
            let raw = self.exec_get_once(&url, &[])?;
            parse_bybit_envelope(&raw)
        })
    }

    fn get_private_envelope<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        query: &str,
    ) -> Result<BybitEnvelope<T>, InfraError> {
        let url = if query.is_empty() {
            format!("{}{}", self.base_url, path)
        } else {
            format!("{}{}?{}", self.base_url, path, query)
        };
        retry_idempotent_get(|| {
            let ts_ms = self.clock.now_ms();
            let sign = sign_bybit_v5(
                ts_ms,
                &self.api_key,
                self.recv_window_ms,
                query,
                &self.api_secret,
            )?;
            let ts_str = ts_ms.to_string();
            let rw_str = self.recv_window_ms.to_string();
            let headers = [
                ("X-BAPI-API-KEY", self.api_key.as_str()),
                ("X-BAPI-TIMESTAMP", ts_str.as_str()),
                ("X-BAPI-RECV-WINDOW", rw_str.as_str()),
                ("X-BAPI-SIGN", sign.as_str()),
            ];
            let raw = self.exec_get_once(&url, &headers)?;
            parse_bybit_envelope(&raw)
        })
    }

    fn exec_get_once(&self, url: &str, headers: &[(&str, &str)]) -> Result<String, InfraError> {
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        match req.call() {
            Ok(resp) => resp
                .into_string()
                .map_err(|e| InfraError::Http(format!("GET {url} failed reading body: {e}"))),
            Err(ureq::Error::Status(code, resp)) => {
                let raw_msg = resp.into_string().unwrap_or_default();
                let msg = sanitize_log_prefix(&raw_msg, RESULT_PREFIX_MAX_CHARS);
                Err(InfraError::Http(format!(
                    "GET {url} returned HTTP {code}: {msg}"
                )))
            }
            Err(ureq::Error::Transport(t)) => {
                Err(InfraError::Http(format!("GET {url} transport error: {t}")))
            }
        }
    }

    fn post_private_once(&self, path: &str, body_json: &str) -> Result<String, (bool, InfraError)> {
        let ts_ms = self.clock.now_ms();
        let sign = sign_bybit_v5(
            ts_ms,
            &self.api_key,
            self.recv_window_ms,
            body_json,
            &self.api_secret,
        )
        .map_err(|e| (false, e))?;
        let ts_str = ts_ms.to_string();
        let rw_str = self.recv_window_ms.to_string();
        let url = format!("{}{}", self.base_url, path);

        let resp = self
            .agent
            .post(&url)
            .set("Content-Type", "application/json")
            .set("X-BAPI-API-KEY", self.api_key.as_str())
            .set("X-BAPI-TIMESTAMP", ts_str.as_str())
            .set("X-BAPI-RECV-WINDOW", rw_str.as_str())
            .set("X-BAPI-SIGN", sign.as_str())
            .send_string(body_json);

        match resp {
            Ok(r) => r.into_string().map_err(|e| {
                (
                    true,
                    InfraError::Http(format!("failed reading POST body: {e}")),
                )
            }),
            Err(ureq::Error::Status(code, r)) => {
                let msg = r.into_string().unwrap_or_default();
                let retryable_or_uncertain = (500..=599).contains(&code);
                Err((
                    retryable_or_uncertain,
                    InfraError::Http(format!("POST {url} returned HTTP {code}: {msg}")),
                ))
            }
            Err(ureq::Error::Transport(t)) => Err((
                true,
                InfraError::Http(format!("POST {url} transport error: {t}")),
            )),
        }
    }

    fn query_order_status_internal(
        &self,
        symbol: Symbol,
        order_link_id: &str,
    ) -> Result<OrderLookupOutcome, InfraError> {
        let query = format!(
            "category=spot&symbol={}&orderLinkId={}",
            symbol.as_str(),
            order_link_id
        );
        let rt_env: BybitEnvelope<OrderListResult> =
            self.get_private_envelope("/v5/order/realtime", &query)?;

        let maybe_order = if let Some(first) = rt_env.result.list.into_iter().next() {
            Some(first)
        } else {
            let hist_env: BybitEnvelope<OrderListResult> =
                self.get_private_envelope("/v5/order/history", &query)?;
            hist_env.result.list.into_iter().next()
        };

        let Some(ord) = maybe_order else {
            return Ok(OrderLookupOutcome::NotFound);
        };

        let exec_qty_raw = parse_opt_decimal_1e8(ord.cum_exec_qty.as_deref())?;
        let exec_quote_raw = parse_opt_decimal_1e8(ord.cum_exec_value.as_deref())?;
        let status = ord.order_status.as_str();

        if status == "New" || status == "PartiallyFilled" || status == "Created" {
            return Ok(OrderLookupOutcome::Pending);
        }

        if exec_qty_raw <= 0 {
            return Ok(OrderLookupOutcome::TerminalEmpty(ord.order_status));
        }

        let side = match ord.side.as_str() {
            "Buy" => OrderSide::Buy,
            "Sell" => OrderSide::Sell,
            other => {
                return Err(InfraError::Parse(format!(
                    "unexpected order side '{other}' in order lookup"
                )));
            }
        };

        let mut avg_price_raw = parse_opt_decimal_1e8(ord.avg_price.as_deref())?;
        if avg_price_raw <= 0 && exec_qty_raw > 0 && exec_quote_raw > 0 {
            avg_price_raw =
                (exec_quote_raw.saturating_mul(crate::domain::types::SCALE_1E8)) / exec_qty_raw;
        }
        if avg_price_raw <= 0 {
            return Err(InfraError::Parse(format!(
                "missing positive avgPrice for filled order {}",
                ord.order_id
            )));
        }

        let (fee_1e8, fee_in_base, exec_ts) = self.fetch_order_execution_fee(
            symbol,
            side,
            &ord.order_id,
            ord.cum_exec_fee.as_deref(),
        )?;
        let updated_ts = ord
            .updated_time
            .as_deref()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let filled_at_ms = if exec_ts > 0 {
            exec_ts
        } else if updated_ts > 0 {
            updated_ts
        } else {
            self.clock.now_ms()
        };

        Ok(OrderLookupOutcome::Filled(FillReport {
            symbol,
            side,
            order_id: ord.order_id,
            order_link_id: ord.order_link_id,
            exec_qty_base_1e8: Qty1e8(exec_qty_raw),
            exec_quote_1e8: Money1e8(exec_quote_raw),
            avg_price_1e8: Price1e8(avg_price_raw),
            fee_1e8,
            fee_in_base,
            filled_at_ms,
        }))
    }

    fn fetch_order_execution_fee(
        &self,
        symbol: Symbol,
        side: OrderSide,
        order_id: &str,
        fallback_cum_fee: Option<&str>,
    ) -> Result<(i128, bool, i64), InfraError> {
        let query = format!(
            "category=spot&symbol={}&orderId={}",
            symbol.as_str(),
            order_id
        );
        let env: BybitEnvelope<ExecutionListResult> =
            self.get_private_envelope("/v5/execution/list", &query)?;

        if env.result.list.is_empty() {
            let fee_raw = parse_opt_decimal_1e8(fallback_cum_fee)?;
            return Ok((fee_raw, side == OrderSide::Buy, 0));
        }

        let mut total_fee: i128 = 0;
        let mut detected_base_fee = side == OrderSide::Buy;
        let mut latest_exec_ts: i64 = 0;
        for exec_item in &env.result.list {
            let fee_part = parse_opt_decimal_1e8(exec_item.exec_fee.as_deref())?;
            total_fee = total_fee.checked_add(fee_part).ok_or_else(|| {
                InfraError::Parse("arithmetic overflow summing execution fees".to_string())
            })?;
            if let Some(ccy) = exec_item.fee_currency.as_deref() {
                if !ccy.is_empty() {
                    detected_base_fee = ccy == symbol.base_coin();
                }
            }
            if let Some(ts_s) = exec_item.exec_time.as_deref() {
                if let Ok(ts) = ts_s.parse::<i64>() {
                    if ts > latest_exec_ts {
                        latest_exec_ts = ts;
                    }
                }
            }
        }
        Ok((total_fee, detected_base_fee, latest_exec_ts))
    }
}

enum OrderLookupOutcome {
    NotFound,
    Pending,
    Filled(FillReport),
    TerminalEmpty(String),
}

impl MarketDataPort for BybitSpotV5Client {
    fn fetch_server_time_ms(&self) -> Result<i64, InfraError> {
        let env: BybitEnvelope<MarketTimeResult> = self.get_public_envelope("/v5/market/time")?;

        if let Some(nano_str) = env.result.time_nano.as_deref() {
            if let Ok(nano) = nano_str.trim().parse::<i128>() {
                if nano > 0 {
                    let ms = i64::try_from(nano / 1_000_000)
                        .map_err(|_| InfraError::Parse("timeNano overflow".to_string()))?;
                    return Ok(ms);
                }
            }
        }
        if let Some(sec_str) = env.result.time_second.as_deref() {
            if let Ok(sec) = sec_str.trim().parse::<i64>() {
                if sec > 0 {
                    return sec
                        .checked_mul(1_000)
                        .ok_or_else(|| InfraError::Parse("timeSecond overflow".to_string()));
                }
            }
        }
        if let Some(t) = env.time {
            if t > 0 {
                return Ok(t);
            }
        }
        Err(InfraError::Parse(
            "missing server timestamp in /v5/market/time".to_string(),
        ))
    }

    fn fetch_instruments_info(
        &self,
        symbols: &[Symbol; UNIVERSE_LEN],
    ) -> Result<[InstrumentRules; UNIVERSE_LEN], InfraError> {
        let mut out: Vec<InstrumentRules> = Vec::with_capacity(UNIVERSE_LEN);
        for &sym in symbols {
            let path = format!(
                "/v5/market/instruments-info?category=spot&symbol={}",
                sym.as_str()
            );
            let env: BybitEnvelope<InstrumentsInfoResult> = self.get_public_envelope(&path)?;
            let item = env
                .result
                .list
                .into_iter()
                .find(|it| it.symbol == sym.as_str())
                .ok_or_else(|| {
                    InfraError::Parse(format!("missing instrument info for {}", sym.as_str()))
                })?;

            let base_precision = Qty1e8::from_decimal_str(&item.lot_size_filter.base_precision)
                .map_err(|e| {
                    InfraError::Parse(format!("basePrecision for {}: {e}", sym.as_str()))
                })?;
            let quote_precision = Money1e8::from_decimal_str(&item.lot_size_filter.quote_precision)
                .map_err(|e| {
                    InfraError::Parse(format!("quotePrecision for {}: {e}", sym.as_str()))
                })?;
            let min_order_qty = Qty1e8::from_decimal_str(&item.lot_size_filter.min_order_qty)
                .map_err(|e| InfraError::Parse(format!("minOrderQty for {}: {e}", sym.as_str())))?;
            let min_order_amt = Money1e8::from_decimal_str(&item.lot_size_filter.min_order_amt)
                .map_err(|e| InfraError::Parse(format!("minOrderAmt for {}: {e}", sym.as_str())))?;
            let tick_size = Price1e8::from_decimal_str(&item.price_filter.tick_size)
                .map_err(|e| InfraError::Parse(format!("tickSize for {}: {e}", sym.as_str())))?;

            if base_precision.raw() <= 0 || quote_precision.raw() <= 0 || tick_size.raw() <= 0 {
                return Err(InfraError::Parse(format!(
                    "non-positive precision/tickSize for {}",
                    sym.as_str()
                )));
            }

            out.push(InstrumentRules {
                symbol: sym,
                base_precision,
                quote_precision,
                min_order_qty,
                min_order_amt,
                tick_size,
                is_trading: item.status == "Trading",
            });
        }

        out.try_into()
            .map_err(|_| InfraError::Parse("unexpected instrument rules count".to_string()))
    }

    fn fetch_klines_1h(&self, symbol: Symbol, limit: usize) -> Result<Vec<Candle>, InfraError> {
        let capped_limit = limit.clamp(1, 1000);
        let path = format!(
            "/v5/market/kline?category=spot&symbol={}&interval=60&limit={}",
            symbol.as_str(),
            capped_limit
        );
        let env: BybitEnvelope<KlineResult> = self.get_public_envelope(&path)?;

        let mut candles = Vec::with_capacity(env.result.list.len());
        for row in env.result.list {
            if row.len() < 7 {
                return Err(InfraError::Parse(format!(
                    "malformed kline row with len {} for {}",
                    row.len(),
                    symbol.as_str()
                )));
            }
            let open_ts_ms = row[0].trim().parse::<i64>().map_err(|e| {
                InfraError::Parse(format!("invalid kline open_ts_ms '{}': {e}", row[0]))
            })?;
            let open = parse_f64_strict(&row[1], "open")?;
            let high = parse_f64_strict(&row[2], "high")?;
            let low = parse_f64_strict(&row[3], "low")?;
            let close = parse_f64_strict(&row[4], "close")?;
            let volume = parse_f64_strict(&row[5], "volume")?;
            let turnover = parse_f64_strict(&row[6], "turnover")?;

            let c = Candle {
                open_ts_ms,
                open,
                high,
                low,
                close,
                volume,
                turnover,
            };
            c.validate().map_err(|e| {
                InfraError::Parse(format!("invalid candle for {}: {e}", symbol.as_str()))
            })?;
            candles.push(c);
        }

        candles.sort_by_key(|c| c.open_ts_ms);
        candles.dedup_by_key(|c| c.open_ts_ms);
        Ok(candles)
    }

    fn fetch_tickers(
        &self,
        symbols: &[Symbol; UNIVERSE_LEN],
    ) -> Result<[TickerSnapshot; UNIVERSE_LEN], InfraError> {
        let mut out: Vec<TickerSnapshot> = Vec::with_capacity(UNIVERSE_LEN);
        for &sym in symbols {
            let path = format!("/v5/market/tickers?category=spot&symbol={}", sym.as_str());
            let env: BybitEnvelope<TickersResult> = self.get_public_envelope(&path)?;
            let item = env
                .result
                .list
                .into_iter()
                .find(|it| it.symbol == sym.as_str())
                .ok_or_else(|| InfraError::Parse(format!("missing ticker for {}", sym.as_str())))?;

            let last = Price1e8::from_decimal_str(&item.last_price)
                .map_err(|e| InfraError::Parse(format!("lastPrice for {}: {e}", sym.as_str())))?;
            let bid1 = if item.bid1_price.trim().is_empty() {
                last
            } else {
                Price1e8::from_decimal_str(&item.bid1_price).map_err(|e| {
                    InfraError::Parse(format!("bid1Price for {}: {e}", sym.as_str()))
                })?
            };
            let ask1 = if item.ask1_price.trim().is_empty() {
                last
            } else {
                Price1e8::from_decimal_str(&item.ask1_price).map_err(|e| {
                    InfraError::Parse(format!("ask1Price for {}: {e}", sym.as_str()))
                })?
            };

            if last.raw() <= 0 || bid1.raw() <= 0 || ask1.raw() <= 0 {
                return Err(InfraError::Parse(format!(
                    "non-positive ticker price for {}",
                    sym.as_str()
                )));
            }

            out.push(TickerSnapshot {
                symbol: sym,
                bid1,
                ask1,
                last,
            });
        }

        out.try_into()
            .map_err(|_| InfraError::Parse("unexpected ticker count".to_string()))
    }
}

impl SpotExecutionPort for BybitSpotV5Client {
    fn fetch_wallet_snapshot(&self) -> Result<WalletSnapshot, InfraError> {
        let env: BybitEnvelope<WalletBalanceResult> =
            self.get_private_envelope("/v5/account/wallet-balance", "accountType=UNIFIED")?;

        let account = env.result.list.into_iter().next().ok_or_else(|| {
            InfraError::Parse("missing UNIFIED account entry in wallet-balance".to_string())
        })?;

        let mut usdt_free_1e8 = Money1e8::ZERO;
        let mut usdt_locked_1e8 = Money1e8::ZERO;
        let mut coin_free_1e8 = [Qty1e8::ZERO; UNIVERSE_LEN];
        let mut coin_locked_1e8 = [Qty1e8::ZERO; UNIVERSE_LEN];

        for c in account.coin {
            let total_raw = parse_opt_decimal_1e8(c.wallet_balance.as_deref())?;
            let locked_raw = parse_opt_decimal_1e8(c.locked.as_deref())?;
            let free_raw = total_raw.checked_sub(locked_raw).ok_or_else(|| {
                InfraError::Parse(format!("wallet arithmetic underflow for coin {}", c.coin))
            })?;
            if free_raw < 0 || locked_raw < 0 {
                return Err(InfraError::Parse(format!(
                    "negative wallet balance for coin {}: total={total_raw}, locked={locked_raw}",
                    c.coin
                )));
            }

            if c.coin == "USDT" {
                usdt_free_1e8 = Money1e8(free_raw);
                usdt_locked_1e8 = Money1e8(locked_raw);
            } else {
                for sym in Symbol::ALL {
                    if c.coin == sym.base_coin() {
                        let idx = sym.index();
                        coin_free_1e8[idx] = Qty1e8(free_raw);
                        coin_locked_1e8[idx] = Qty1e8(locked_raw);
                    }
                }
            }
        }

        Ok(WalletSnapshot {
            usdt_free_1e8,
            usdt_locked_1e8,
            coin_free_1e8,
            coin_locked_1e8,
            fetched_at_ms: self.clock.now_ms(),
        })
    }

    fn query_order_by_link_id(
        &self,
        symbol: Symbol,
        order_link_id: &str,
    ) -> Result<Option<FillReport>, InfraError> {
        for attempt in 1..=3 {
            match self.query_order_status_internal(symbol, order_link_id)? {
                OrderLookupOutcome::Filled(fill) => return Ok(Some(fill)),
                OrderLookupOutcome::NotFound | OrderLookupOutcome::TerminalEmpty(_) => {
                    return Ok(None);
                }
                OrderLookupOutcome::Pending => {
                    if attempt == 3 {
                        return Err(InfraError::ExchangeApi {
                            code: -1,
                            msg: format!("order {order_link_id} still pending after polling"),
                        });
                    }
                    thread::sleep(Duration::from_millis(250));
                }
            }
        }
        Ok(None)
    }

    fn execute_spot_order(&self, req: &OrderRequest) -> Result<FillReport, InfraError> {
        if req.order_link_id.trim().is_empty() {
            return Err(InfraError::Config(
                "order_link_id must be non-empty".to_string(),
            ));
        }

        let (market_unit, qty_str) = match req.side {
            OrderSide::Buy => {
                if req.quote_spend_1e8.raw() <= 0 {
                    return Err(InfraError::Config(
                        "buy quote_spend_1e8 must be positive".to_string(),
                    ));
                }
                ("quoteCoin", req.quote_spend_1e8.format_fixed(2))
            }
            OrderSide::Sell => {
                let floored = req
                    .base_qty_1e8
                    .floor_to_step(req.base_precision)
                    .map_err(|e| InfraError::Config(format!("invalid sell qty: {e}")))?;
                if floored.raw() <= 0 {
                    return Err(InfraError::Config(
                        "sell base_qty_1e8 floored to zero".to_string(),
                    ));
                }
                ("baseCoin", floored.format_by_step(req.base_precision))
            }
        };

        let create_req = CreateOrderBody {
            category: "spot",
            symbol: req.symbol.as_str(),
            side: req.side.as_str(),
            order_type: "Market",
            qty: &qty_str,
            market_unit,
            order_link_id: &req.order_link_id,
        };
        let payload = serde_json::to_string(&create_req).map_err(|e| {
            InfraError::Parse(format!("failed to serialize order/create payload: {e}"))
        })?;

        match self.post_private_once("/v5/order/create", &payload) {
            Ok(raw) => {
                let env: BybitEnvelope<CreateOrderResult> = parse_bybit_envelope(&raw)?;
                if env
                    .result
                    .order_id
                    .as_deref()
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(false)
                {
                    return Err(InfraError::Parse(
                        "empty orderId in /v5/order/create response".to_string(),
                    ));
                }
            }
            Err((uncertain_network_or_5xx, orig_err)) => {
                if uncertain_network_or_5xx {
                    if let Ok(Some(fill)) =
                        self.query_order_by_link_id(req.symbol, &req.order_link_id)
                    {
                        return Ok(fill);
                    }
                }
                return Err(orig_err);
            }
        }

        for attempt in 1..=MAX_FILL_POLL_ATTEMPTS {
            match self.query_order_status_internal(req.symbol, &req.order_link_id)? {
                OrderLookupOutcome::Filled(fill) => return Ok(fill),
                OrderLookupOutcome::TerminalEmpty(status) => {
                    return Err(InfraError::ExchangeApi {
                        code: -1,
                        msg: format!(
                            "spot order {} terminated with status '{status}' and 0 fill",
                            req.order_link_id
                        ),
                    });
                }
                OrderLookupOutcome::NotFound | OrderLookupOutcome::Pending => {
                    if attempt < MAX_FILL_POLL_ATTEMPTS {
                        let delay_ms = 150u64.saturating_mul(attempt as u64);
                        thread::sleep(Duration::from_millis(delay_ms));
                    }
                }
            }
        }

        Err(InfraError::ExchangeApi {
            code: -1,
            msg: format!(
                "timed out waiting for fill report on order_link_id={}",
                req.order_link_id
            ),
        })
    }
}

pub fn is_retryable_infra_error(err: &InfraError) -> bool {
    match err {
        InfraError::ExchangeApi { code, .. } => {
            matches!(*code, -1 | 10000 | 10006 | 10016 | 10018)
        }
        InfraError::Http(msg) => {
            if let Some(status) = extract_http_status_code(msg) {
                return status == 408 || status == 429 || (500..=599).contains(&status);
            }
            let lower = msg.to_ascii_lowercase();
            [
                "transport",
                "timeout",
                "timed out",
                "failed reading body",
                "connection",
                "network",
                "dns",
                "broken pipe",
                "unexpected eof",
                "temporarily unavailable",
            ]
            .iter()
            .any(|pat| lower.contains(pat))
        }
        InfraError::Parse(_)
        | InfraError::Io(_)
        | InfraError::LockBusy(_)
        | InfraError::Config(_)
        | InfraError::Notifier(_)
        | InfraError::ClockSkew { .. } => false,
    }
}

fn extract_http_status_code(msg: &str) -> Option<u16> {
    let lower = msg.to_ascii_lowercase();
    for marker in [
        "http ",
        "http/1.1 ",
        "http/1.0 ",
        "http/2 ",
        "status ",
        "status=",
        "status:",
    ] {
        let mut search_from = 0usize;
        while let Some(slice) = lower.get(search_from..) {
            let Some(rel_idx) = slice.find(marker) else {
                break;
            };
            let after_marker = search_from + rel_idx + marker.len();
            if let Some(rest) = lower.get(after_marker..) {
                let trimmed = rest.trim_start();
                let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
                if digits.len() == 3 {
                    if let Ok(code) = digits.parse::<u16>() {
                        if (100..=599).contains(&code) {
                            return Some(code);
                        }
                    }
                }
            }
            search_from = after_marker;
        }
    }
    None
}

fn sanitize_log_prefix(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut count = 0usize;
    let mut prev_ws = true;
    let mut truncated = false;

    for ch in input.chars() {
        let normalized = if ch.is_control() || ch.is_whitespace() {
            if prev_ws {
                continue;
            }
            prev_ws = true;
            ' '
        } else {
            prev_ws = false;
            ch
        };

        if count == max_chars {
            truncated = true;
            break;
        }
        out.push(normalized);
        count += 1;
    }

    if truncated {
        if max_chars >= 3 {
            let mut removed = 0usize;
            while removed < 3 && out.pop().is_some() {
                removed += 1;
            }
            while out.ends_with(' ') {
                out.pop();
            }
            out.push_str("...");
        }
    } else {
        while out.ends_with(' ') {
            out.pop();
        }
    }

    out
}

fn retry_idempotent_get<T, F>(op: F) -> Result<T, InfraError>
where
    F: FnMut() -> Result<T, InfraError>,
{
    retry_idempotent_get_with_sleep(op, thread::sleep)
}

fn retry_idempotent_get_with_sleep<T, F, S>(mut op: F, mut sleep_fn: S) -> Result<T, InfraError>
where
    F: FnMut() -> Result<T, InfraError>,
    S: FnMut(Duration),
{
    let mut backoff_iter = GET_RETRY_BACKOFF_SECS.iter().copied();
    loop {
        match op() {
            Ok(value) => return Ok(value),
            Err(err) => {
                if !is_retryable_infra_error(&err) {
                    return Err(err);
                }
                match backoff_iter.next() {
                    Some(secs) => sleep_fn(Duration::from_secs(secs)),
                    None => return Err(err),
                }
            }
        }
    }
}

fn parse_bybit_envelope<T: for<'de> Deserialize<'de>>(
    raw: &str,
) -> Result<BybitEnvelope<T>, InfraError> {
    let raw_env: RawBybitEnvelope = serde_json::from_str(raw).map_err(|e| {
        InfraError::Parse(format!(
            "failed to parse Bybit V5 envelope: {e}; raw_prefix={}",
            sanitize_log_prefix(raw, RESULT_PREFIX_MAX_CHARS)
        ))
    })?;

    if raw_env.ret_code != 0 {
        return Err(InfraError::ExchangeApi {
            code: raw_env.ret_code,
            msg: raw_env.ret_msg,
        });
    }

    let result_value = raw_env.result.ok_or_else(|| {
        InfraError::Parse("missing or null 'result' in Bybit V5 envelope (retCode=0)".to_string())
    })?;

    let result: T = serde_json::from_value(result_value.clone()).map_err(|e| {
        InfraError::Parse(format!(
            "failed to parse Bybit V5 result: {e}; result_prefix={}",
            sanitize_log_prefix(&result_value.to_string(), RESULT_PREFIX_MAX_CHARS)
        ))
    })?;

    Ok(BybitEnvelope {
        result,
        time: raw_env.time,
    })
}

fn parse_opt_decimal_1e8(raw: Option<&str>) -> Result<i128, InfraError> {
    let Some(s) = raw else {
        return Ok(0);
    };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Ok(0);
    }
    parse_decimal_1e8(trimmed).map_err(|e| InfraError::Parse(format!("decimal '{trimmed}': {e}")))
}

fn parse_f64_strict(s: &str, field: &str) -> Result<f64, InfraError> {
    let val = s
        .trim()
        .parse::<f64>()
        .map_err(|e| InfraError::Parse(format!("invalid {field} '{s}': {e}")))?;
    if !val.is_finite() {
        return Err(InfraError::Parse(format!("non-finite {field} '{s}'")));
    }
    Ok(val)
}

#[derive(Deserialize)]
struct RawBybitEnvelope {
    #[serde(rename = "retCode")]
    ret_code: i64,
    #[serde(rename = "retMsg", default)]
    ret_msg: String,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    time: Option<i64>,
}

struct BybitEnvelope<T> {
    result: T,
    time: Option<i64>,
}

#[derive(Deserialize)]
struct MarketTimeResult {
    #[serde(rename = "timeSecond")]
    time_second: Option<String>,
    #[serde(rename = "timeNano")]
    time_nano: Option<String>,
}

#[derive(Deserialize)]
struct InstrumentsInfoResult {
    list: Vec<InstrumentItem>,
}

#[derive(Deserialize)]
struct InstrumentItem {
    symbol: String,
    status: String,
    #[serde(rename = "lotSizeFilter")]
    lot_size_filter: LotSizeFilter,
    #[serde(rename = "priceFilter")]
    price_filter: PriceFilter,
}

#[derive(Deserialize)]
struct LotSizeFilter {
    #[serde(rename = "basePrecision")]
    base_precision: String,
    #[serde(rename = "quotePrecision")]
    quote_precision: String,
    #[serde(rename = "minOrderQty")]
    min_order_qty: String,
    #[serde(rename = "minOrderAmt")]
    min_order_amt: String,
}

#[derive(Deserialize)]
struct PriceFilter {
    #[serde(rename = "tickSize")]
    tick_size: String,
}

#[derive(Deserialize)]
struct KlineResult {
    list: Vec<Vec<String>>,
}

#[derive(Deserialize)]
struct TickersResult {
    list: Vec<TickerItem>,
}

#[derive(Deserialize)]
struct TickerItem {
    symbol: String,
    #[serde(rename = "bid1Price")]
    bid1_price: String,
    #[serde(rename = "ask1Price")]
    ask1_price: String,
    #[serde(rename = "lastPrice")]
    last_price: String,
}

#[derive(Deserialize)]
struct WalletBalanceResult {
    list: Vec<WalletAccountItem>,
}

#[derive(Deserialize)]
struct WalletAccountItem {
    coin: Vec<WalletCoinItem>,
}

#[derive(Deserialize)]
struct WalletCoinItem {
    coin: String,
    #[serde(rename = "walletBalance")]
    wallet_balance: Option<String>,
    locked: Option<String>,
}

#[derive(Serialize)]
struct CreateOrderBody<'a> {
    category: &'a str,
    symbol: &'a str,
    side: &'a str,
    #[serde(rename = "orderType")]
    order_type: &'a str,
    qty: &'a str,
    #[serde(rename = "marketUnit")]
    market_unit: &'a str,
    #[serde(rename = "orderLinkId")]
    order_link_id: &'a str,
}

#[derive(Deserialize)]
struct CreateOrderResult {
    #[serde(rename = "orderId")]
    order_id: Option<String>,
}

#[derive(Deserialize)]
struct OrderListResult {
    list: Vec<OrderItem>,
}

#[derive(Deserialize)]
struct OrderItem {
    #[serde(rename = "orderId")]
    order_id: String,
    #[serde(rename = "orderLinkId")]
    order_link_id: String,
    side: String,
    #[serde(rename = "orderStatus")]
    order_status: String,
    #[serde(rename = "cumExecQty")]
    cum_exec_qty: Option<String>,
    #[serde(rename = "cumExecValue")]
    cum_exec_value: Option<String>,
    #[serde(rename = "avgPrice")]
    avg_price: Option<String>,
    #[serde(rename = "cumExecFee")]
    cum_exec_fee: Option<String>,
    #[serde(rename = "updatedTime")]
    updated_time: Option<String>,
}

#[derive(Deserialize)]
struct ExecutionListResult {
    list: Vec<ExecutionItem>,
}

#[derive(Deserialize)]
struct ExecutionItem {
    #[serde(rename = "execFee")]
    exec_fee: Option<String>,
    #[serde(rename = "feeCurrency")]
    fee_currency: Option<String>,
    #[serde(rename = "execTime")]
    exec_time: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_bybit_v5_deterministic() {
        let sig = sign_bybit_v5(
            1658385579423,
            "XXXXXXXXXX",
            5000,
            "accountType=UNIFIED",
            "SSSSSSSSSS",
        );
        assert!(sig.is_ok());
        if let Ok(hex_sig) = sig {
            assert_eq!(hex_sig.len(), 64);
            assert!(hex_sig.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn test_paper_noop_execution_port_errors() {
        let noop = PaperNoopExecutionPort;
        assert!(noop.fetch_wallet_snapshot().is_err());
        assert!(noop.query_order_by_link_id(Symbol::BtcUsdt, "x").is_err());
    }

    #[test]
    fn parse_envelope_returns_exchange_api_error_when_result_is_empty_object() {
        let raw = r#"{"retCode":10016,"retMsg":"Service temporarily unavailable","result":{},"retExtInfo":{},"time":1795618808000}"#;
        let res = parse_bybit_envelope::<KlineResult>(raw);
        assert!(res.is_err());
        if let Err(ref err) = res {
            assert_eq!(
                err,
                &InfraError::ExchangeApi {
                    code: 10016,
                    msg: "Service temporarily unavailable".to_string(),
                }
            );
            assert!(is_retryable_infra_error(err));
        }
    }

    #[test]
    fn parse_envelope_succeeds_on_ret_code_zero_and_valid_result() {
        let raw = r#"{"retCode":0,"retMsg":"OK","result":{"symbol":"BTCUSDT","category":"spot","list":[["1795615200000","95000.0","95500.0","94800.0","95250.5","123.45","11750000.0"]]},"retExtInfo":{},"time":1795618808000}"#;
        let res = parse_bybit_envelope::<KlineResult>(raw);
        assert!(res.is_ok());
        if let Ok(env) = res {
            assert_eq!(env.time, Some(1795618808000));
            assert_eq!(env.result.list.len(), 1);
            assert_eq!(env.result.list[0][0], "1795615200000");
            assert_eq!(env.result.list[0][4], "95250.5");
        }
    }

    #[test]
    fn retryable_error_classifier_covers_transient_codes_and_rejects_fatal_errors() {
        for code in [-1, 10000, 10006, 10016, 10018] {
            let err = InfraError::ExchangeApi {
                code,
                msg: "transient".to_string(),
            };
            assert!(
                is_retryable_infra_error(&err),
                "expected ExchangeApi({code}) to be retryable"
            );
        }

        for http_err in [
            InfraError::Http(
                "GET https://api.bybit.com/v5/market/kline returned HTTP 408: timeout".to_string(),
            ),
            InfraError::Http(
                "GET https://api.bybit.com/v5/market/kline returned HTTP 429: rate limit"
                    .to_string(),
            ),
            InfraError::Http(
                "GET https://api.bybit.com/v5/market/kline returned HTTP 500: internal".to_string(),
            ),
            InfraError::Http(
                "GET https://api.bybit.com/v5/market/kline returned HTTP 502: bad gateway"
                    .to_string(),
            ),
            InfraError::Http(
                "GET https://api.bybit.com/v5/market/kline returned HTTP 503: unavailable"
                    .to_string(),
            ),
            InfraError::Http(
                "GET https://api.bybit.com/v5/market/kline returned HTTP 504: gateway timeout"
                    .to_string(),
            ),
            InfraError::Http(
                "GET https://api.bybit.com/v5/market/kline transport error: io timeout".to_string(),
            ),
            InfraError::Http(
                "GET https://api.bybit.com/v5/market/kline failed reading body: unexpected eof"
                    .to_string(),
            ),
        ] {
            assert!(
                is_retryable_infra_error(&http_err),
                "expected {http_err} to be retryable"
            );
        }

        for fatal_code in [10001, 10003, 10004] {
            let err = InfraError::ExchangeApi {
                code: fatal_code,
                msg: "fatal".to_string(),
            };
            assert!(
                !is_retryable_infra_error(&err),
                "expected ExchangeApi({fatal_code}) NOT to be retryable"
            );
        }

        for non_retryable in [
            InfraError::Parse("missing field `list`".to_string()),
            InfraError::Http("GET https://api.bybit.com/v5/market/kline returned HTTP 400: invalid param 503 transport".to_string()),
            InfraError::Http("GET https://api.bybit.com/v5/market/kline returned HTTP 401: unauthorized".to_string()),
            InfraError::Http("GET https://api.bybit.com/v5/market/kline returned HTTP 403: forbidden".to_string()),
            InfraError::Http("GET https://api.bybit.com/v5/market/kline returned HTTP 404: not found".to_string()),
            InfraError::Config("missing key".to_string()),
            InfraError::ClockSkew {
                skew_ms: 3000,
                max_allowed_ms: 2000,
            },
        ] {
            assert!(
                !is_retryable_infra_error(&non_retryable),
                "expected {non_retryable} NOT to be retryable"
            );
        }
    }

    #[test]
    fn parse_envelope_ret_code_zero_malformed_result_includes_bounded_sanitized_prefix() {
        let padding = "x".repeat(260);
        let raw = format!(
            r#"{{"retCode":0,"retMsg":"OK","result":{{"unexpected":"line1\nline2\t{padding}"}},"time":1795618808000}}"#
        );
        let res = parse_bybit_envelope::<KlineResult>(&raw);
        assert!(res.is_err());
        if let Err(err) = res {
            assert!(
                matches!(&err, InfraError::Parse(_)),
                "expected InfraError::Parse, got {err}"
            );
            if let InfraError::Parse(msg) = err {
                assert!(msg.contains("missing field"), "msg={msg}");
                assert!(!msg.contains('\n'), "msg should have no LF: {msg}");
                assert!(!msg.contains('\r'), "msg should have no CR: {msg}");
                let marker = "result_prefix=";
                let marker_pos = msg.find(marker);
                assert!(marker_pos.is_some(), "missing result_prefix in {msg}");
                if let Some(pos) = marker_pos {
                    let prefix = &msg[pos + marker.len()..];
                    assert!(
                        prefix.chars().count() <= RESULT_PREFIX_MAX_CHARS,
                        "prefix too long: {}",
                        prefix.chars().count()
                    );
                    assert!(prefix.ends_with("..."), "expected ellipsis in {prefix}");
                }
            }
        }
    }

    #[test]
    fn retry_idempotent_get_uses_1s_2s_4s_backoff_and_fails_fast_on_non_retryable() {
        let mut attempts = 0usize;
        let mut sleeps: Vec<Duration> = Vec::new();
        let recovered = retry_idempotent_get_with_sleep(
            || {
                attempts += 1;
                if attempts < 4 {
                    Err(InfraError::ExchangeApi {
                        code: 10016,
                        msg: "busy".to_string(),
                    })
                } else {
                    Ok(42usize)
                }
            },
            |d| sleeps.push(d),
        );
        assert_eq!(recovered, Ok(42usize));
        assert_eq!(attempts, 4);
        assert_eq!(
            sleeps,
            vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
            ]
        );

        let mut exhausted_attempts = 0usize;
        let mut exhausted_sleeps: Vec<Duration> = Vec::new();
        let exhausted: Result<(), InfraError> = retry_idempotent_get_with_sleep(
            || {
                exhausted_attempts += 1;
                Err(InfraError::ExchangeApi {
                    code: 10006,
                    msg: "rate limit".to_string(),
                })
            },
            |d| exhausted_sleeps.push(d),
        );
        assert_eq!(
            exhausted,
            Err(InfraError::ExchangeApi {
                code: 10006,
                msg: "rate limit".to_string(),
            })
        );
        assert_eq!(exhausted_attempts, 4);
        assert_eq!(
            exhausted_sleeps,
            vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
            ]
        );

        let mut fatal_attempts = 0usize;
        let mut fatal_sleeps: Vec<Duration> = Vec::new();
        let fatal: Result<(), InfraError> = retry_idempotent_get_with_sleep(
            || {
                fatal_attempts += 1;
                Err(InfraError::ExchangeApi {
                    code: 10001,
                    msg: "invalid parameter".to_string(),
                })
            },
            |d| fatal_sleeps.push(d),
        );
        assert!(fatal.is_err());
        assert_eq!(fatal_attempts, 1);
        assert!(fatal_sleeps.is_empty());

        let mut parse_attempts = 0usize;
        let mut parse_sleeps: Vec<Duration> = Vec::new();
        let parse_err: Result<(), InfraError> = retry_idempotent_get_with_sleep(
            || {
                parse_attempts += 1;
                Err(InfraError::Parse("bad json".to_string()))
            },
            |d| parse_sleeps.push(d),
        );
        assert!(parse_err.is_err());
        assert_eq!(parse_attempts, 1);
        assert!(parse_sleeps.is_empty());
    }
}
