# VORTEX-M2B-SPOT v1.0.0

Deterministic `1h` Spot `0x` TSMOM Vol-Targeted execution engine (`B2-01`) with a parallel `4h` shadow telemetry book (`A0_4H`) for Bybit Spot V5 (`UNIFIED` account, `LONG_FLAT`, zero leverage, zero short selling).

## Architecture

- **Hexagonal Clean DDD**:
  - `src/domain/`: 100% pure `std` (zero external crates). Fixed-point `i128` (`Money1e8`, `Qty1e8`, `Price1e8`, `Rate1e10`) with **`Drift == 0`** conservation invariant, Garman-Klass EWMA volatility, multi-horizon TSMOM (`3d / 7d / 21d`), `SELL_FIRST_BUY_SECOND` rebalance planner, anti-dust liquidation, and `0.3%` buy liquidity reserve.
  - `src/application/`: `RunHourlyCycleUseCase` (strict closed-hour idempotency, exclusive `run.lock`, `DRAIN_MODE.lock`, `HALT_LOCK`, double safety lock for `--live-spot`, parallel `A0_4H` shadow engine) and live wallet reconciliation.
  - `src/infrastructure/`: Minimal synchronous runtime (`ureq` with `rustls` TLS, `serde`/`serde_json`, `hmac`/`sha2`/`hex`; no `tokio`), Bybit Spot V5 REST client with deterministic `orderLinkId`, atomic state persistence (`.tmp -> sync_all -> rename`), FNV-1a 64-bit sealed append-only JSONL audit ledger, and best-effort Telegram alerts.

## Universe & Parameters (Frozen)

- **Pairs**: `BTCUSDT`, `ETHUSDT`, `SOLUSDT`, `LINKUSDT`, `AVAXUSDT`
- **Primary Engine (`B2-01`, `1h`)**: $N_{\text{eff}} = 84$, lookbacks $(72, 0.25), (168, 0.35), (504, 0.40)$, $\sigma_{\text{target}} = 0.50$, $W_{\max} = 0.40$, $L_{\max} = 1.00$, deadband $\delta = 0.20 \implies 0.08$.
- **Shadow Engine (`A0_4H`, `4h` UTC)**: $N_{\text{eff}} = 21$, lookbacks $(18, 0.25), (42, 0.35), (126, 0.40)$, paper-only telemetry.

## CLI Usage

```bash
# Default safe paper simulation (zero private order calls)
vortex-m2b-spot run --paper

# Inspect current primary & shadow 4h state (read-only)
vortex-m2b-spot status

# Verify Bybit public connectivity, clock skew < 2000ms, instruments & state/ permissions
vortex-m2b-spot health

# Live Spot 0x execution (requires Double Safety Lock in .env and no DRAIN/HALT locks)
vortex-m2b-spot run --live-spot
```

## Documentation

See [`DOCS/INDEX.md`](DOCS/INDEX.md), [`DOCS/PHASE_1_ARCHITECTURE_AND_VERIFICATION_REPORT.md`](DOCS/PHASE_1_ARCHITECTURE_AND_VERIFICATION_REPORT.md), and [`DOCS/RUNBOOK_TOKIO_PAPER.md`](DOCS/RUNBOOK_TOKIO_PAPER.md).
