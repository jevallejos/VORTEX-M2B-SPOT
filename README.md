# VORTEX-M2B-SPOT v1.1.1

Deterministic `1h` Spot `0x` TSMOM Vol-Targeted execution engine (`B2-01`) with a parallel `4h` shadow telemetry book (`A0_4H`) for Bybit Spot V5 (`UNIFIED` account, `LONG_FLAT`, zero leverage, zero short selling), supporting a configurable Spot `*USDT` universe (default 6 pairs including `XAUTUSDT`) and Fail-Closed `schema_version = 2` persistence.

## Architecture

- **Hexagonal Clean DDD**:
  - `src/domain/`: 100% pure `std` (zero external crates). Validated dynamic `Universe` (`1..=20` Spot `*USDT` pairs), fixed-point `i128` (`Money1e8`, `Qty1e8`, `Price1e8`, `Rate1e10`) with **`Drift == 0`** conservation invariant, Garman-Klass EWMA volatility, multi-horizon TSMOM (`3d / 7d / 21d`), `SELL_FIRST_BUY_SECOND` rebalance planner, anti-dust liquidation, and `0.3%` buy liquidity reserve.
  - `src/application/`: `RunHourlyCycleUseCase` (strict closed-hour idempotency, exclusive `run.lock`, `DRAIN_MODE.lock` blocking new buys while allowing exit sells, `HALT_LOCK`, double safety lock for `--live-spot`, parallel `A0_4H` shadow engine) and live wallet reconciliation across dynamic base coins (e.g. `XAUTUSDT -> XAUT`).
  - `src/infrastructure/`: Minimal synchronous runtime (`ureq` with `rustls` TLS, `serde`/`serde_json`, `hmac`/`sha2`/`hex`; no `tokio`), Bybit Spot V5 REST client with two-stage `retCode` envelope parsing, transient GET retries with fresh HMAC signing, zero blind POST retries, deterministic `orderLinkId`, atomic Fail-Closed `schema_version = 2` state persistence (`.tmp -> sync_all -> rename`), FNV-1a 64-bit sealed append-only JSONL audit ledger, and best-effort Telegram alerts.

## Universe & Parameters (Frozen)

- **Configurable Universe (`M2B_UNIVERSE_SYMBOLS`)**: Optional comma-separated list (`1..=20` unique uppercase ASCII `*USDT` symbols, length `6..=20`, Fail-Closed startup validation).
- **Default Pairs (`6`)**: `BTCUSDT`, `ETHUSDT`, `SOLUSDT`, `LINKUSDT`, `AVAXUSDT`, `XAUTUSDT`
- **State Persistence (`schema_version = 2`)**: `primary_state.json` and `m2b_shadow_4h_state.json` store `"schema_version": 2` and `"symbols": [...]`, aborting Fail-Closed on version mismatch (`v1 -> v2`), symbol/order mismatch, or corrupt vector lengths.
- **Primary Engine (`B2-01`, `1h`)**: $N_{\text{eff}} = 84$, lookbacks $(72, 0.25), (168, 0.35), (504, 0.40)$, $\sigma_{\text{target}} = 0.50$ with frozen numerator $\sigma_{\text{target}} / 4 = 0.125$ (independent of $N$), vol floor $\max(\sigma_{\text{ann}}, 0.15)$, $W_{\max} = 0.40$, $L_{\max} = 1.00$, deadband $\delta = 0.20 \implies 0.08$.
- **Liquidation & Compatibility (`v1.1.1`)**: Full liquidation triggers whenever `target_w < 0.015` (`LIQUIDATE_TARGET_THRESH`) for any open position (`current_w > 0.0` / `qty > 0`), gated strictly by the exchange instrument minimums (`min_order_amt` and `min_order_qty`) with silent dust skip (`0` churn, `Drift == 0`) and zero-migration hot compatibility for `v1.1.0` `schema_version = 2` state files.
- **Shadow Engine (`A0_4H`, `4h` UTC)**: $N_{\text{eff}} = 21$, lookbacks $(18, 0.25), (42, 0.35), (126, 0.40)$, paper-only telemetry.

## CLI Usage

```bash
# Default safe paper simulation (zero private order calls)
vortex-m2b-spot run --paper

# Inspect current primary & shadow 4h state (read-only)
vortex-m2b-spot status

# Verify Bybit public connectivity, clock skew < 2000ms, configured instruments & state/ permissions
vortex-m2b-spot health

# Live Spot 0x execution (requires Double Safety Lock in .env and no HALT_LOCK)
vortex-m2b-spot run --live-spot
```

## Documentation

See [`DOCS/INDEX.md`](DOCS/INDEX.md), [`DOCS/PHASE_1_ARCHITECTURE_AND_VERIFICATION_REPORT.md`](DOCS/PHASE_1_ARCHITECTURE_AND_VERIFICATION_REPORT.md), [`DOCS/PHASE_2_V110_DYNAMIC_UNIVERSE_XAUT_REPORT.md`](DOCS/PHASE_2_V110_DYNAMIC_UNIVERSE_XAUT_REPORT.md), [`DOCS/PHASE_3_V111_LIQUIDATION_SLIT_FIX_REPORT.md`](DOCS/PHASE_3_V111_LIQUIDATION_SLIT_FIX_REPORT.md), and [`DOCS/RUNBOOK_TOKIO_PAPER.md`](DOCS/RUNBOOK_TOKIO_PAPER.md).
