use crate::domain::error::InfraError;
use crate::domain::types::{
    AuditEntry, Candle, EnginePersistedState, FillReport, InstrumentRules, OrderRequest,
    Shadow4hPersistedState, Symbol, TickerSnapshot, WalletSnapshot, UNIVERSE_LEN,
};

pub trait ClockPort: Send + Sync {
    fn now_ms(&self) -> i64;
}

pub trait MarketDataPort: Send + Sync {
    fn fetch_server_time_ms(&self) -> Result<i64, InfraError>;
    fn fetch_instruments_info(
        &self,
        symbols: &[Symbol; UNIVERSE_LEN],
    ) -> Result<[InstrumentRules; UNIVERSE_LEN], InfraError>;
    fn fetch_klines_1h(&self, symbol: Symbol, limit: usize) -> Result<Vec<Candle>, InfraError>;
    fn fetch_tickers(
        &self,
        symbols: &[Symbol; UNIVERSE_LEN],
    ) -> Result<[TickerSnapshot; UNIVERSE_LEN], InfraError>;
}

pub trait SpotExecutionPort: Send + Sync {
    fn fetch_wallet_snapshot(&self) -> Result<WalletSnapshot, InfraError>;
    fn query_order_by_link_id(
        &self,
        symbol: Symbol,
        order_link_id: &str,
    ) -> Result<Option<FillReport>, InfraError>;
    fn execute_spot_order(&self, req: &OrderRequest) -> Result<FillReport, InfraError>;
}

pub trait RunLockGuard: Send {}

pub trait StateStorePort: Send + Sync {
    fn acquire_run_lock(&self) -> Result<Box<dyn RunLockGuard>, InfraError>;
    fn has_drain_lock(&self) -> Result<bool, InfraError>;
    fn has_halt_lock(&self) -> Result<bool, InfraError>;
    fn write_halt_lock(&self, reason: &str) -> Result<(), InfraError>;
    fn load_primary_state(&self) -> Result<Option<EnginePersistedState>, InfraError>;
    fn save_primary_state(&self, state: &EnginePersistedState) -> Result<(), InfraError>;
    fn load_shadow_4h_state(&self) -> Result<Option<Shadow4hPersistedState>, InfraError>;
    fn save_shadow_4h_state(&self, state: &Shadow4hPersistedState) -> Result<(), InfraError>;
    fn append_audit_entry(&self, entry: &AuditEntry) -> Result<(), InfraError>;
}

pub trait NotifierPort: Send + Sync {
    fn send_message(&self, text: &str) -> Result<(), InfraError>;
}
