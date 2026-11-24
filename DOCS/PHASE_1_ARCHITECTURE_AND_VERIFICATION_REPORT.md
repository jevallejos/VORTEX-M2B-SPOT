# VORTEX-M2B-SPOT — Reporte de Arquitectura y Verificación de Fase 1

**Proyecto:** `VORTEX-M2B-SPOT` (`v1.0.0`)  
**Estrategia primaria:** `B2-01` (`1h` Spot `0x` TSMOM Vol-Targeted, `LONG_FLAT`)  
**Telemetría paralela:** `A0_4H` (`4h` UTC Shadow Book en `--paper`)  
**Venue objetivo:** Bybit Spot V5 (`UNIFIED`, sin apalancamiento, sin cortos)  
**Repositorio origen:** `https://github.com/jevallejos/VORTEX-M2B-SPOT.git`

---

## Sección 1: Veredicto Ejecutivo e Inventario Exacto

### 1.1 Veredicto Ejecutivo

**VEREDICTO:** **APROBADO PARA VALIDACIÓN `--paper` DE 30 DÍAS EN TOKIO** (`NO PROMOVIDO AÚN A --live-spot`).

El motor `vortex-m2b-spot` v1.0.0 cumple integralmente las invariantes matemáticas, arquitectónicas, contables y de seguridad operacional exigidas en Fase 1:

1. **Aislamiento de dominio puro:** `src/domain/` utiliza exclusivamente la biblioteca estándar (`std`), con **0 crates externos** y **0 usos** de `unsafe`, `.unwrap()`, `.expect()`, `panic!()`, `todo!()` o `unimplemented!()`.
2. **Integridad física y causalidad (`B2-01` + `A0_4H`):** Garman-Klass EWMA causal ($N_{\text{eff}} = 84$ en `1h` / $21$ en `4h`), TSMOM multi-horizonte ($\varphi = 1.618033988749895$), filtrado estricto de velas cerradas (`close_ts_ms <= server_time_ms`) y agregador exacto `1h -> 4h` verificados bit-a-bit sin sesgo look-ahead.
3. **Conservación contable exacta (`Drift == 0`):** Aritmética entera escalada `i128` (`1e8` para USDT/cantidades/precios y `1e10` para tasas/TWR), asimetría Spot V5 (`Buy` comisión en base truncada a `base_precision`; `Sell` comisión en USDT), neutralidad TWR ante depósitos/retiros externos y corte duro de drawdown (`> 25%` TWR).
4. **Verificación integral:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets` (`35/35` pruebas aprobadas: `17` unitarias en `src/lib.rs`, `6` E2E con mocks, `8` de física de dominio y `4` de red-team hygiene) y `cargo build --release` pasaron con código de salida `0`. La prueba en vivo de `.\target\release\vortex-m2b-spot.exe health` contra `https://api.bybit.com` validó conectividad, reloj (`skew = 227 ms <= 2000 ms`) y los 5 instrumentos Spot en estado `Trading` (tras observar inicialmente un skew transitorio del host local de `4149 ms > 2000 ms`, confirmando en práctica el bloqueo fail-closed de reloj).

La habilitación de `--live-spot` permanece bloqueada por diseño (**Double Safety Lock**) hasta superar los criterios cuantitativos y operacionales de la ventana `--paper` de 30 días en Tokio (Sección 6).

---

### 1.2 Inventario Exacto de Archivos del Repositorio (Línea Base Verificada)

Inventario completo observado en disco para los **31 archivos** de código fuente, pruebas, CI, despliegue y configuración raíz (previo a la emisión de los 3 documentos en `DOCS/`):

| Ruta Exacta | Capa Arquitectónica | Líneas | Bytes |
|---|---|---:|---:|
| `.env.example` | `Root/CI` | 12 | 286 |
| `.gitattributes` | `Root/CI` | 7 | 132 |
| `.github/workflows/ci.yml` | `Root/CI` | 35 | 689 |
| `.gitignore` | `Root/CI` | 9 | 75 |
| `Cargo.lock` | `Root/CI` | 777 | 20,119 |
| `Cargo.toml` | `Root/CI` | 33 | 682 |
| `README.md` | `Docs` | 36 | 2,329 |
| `deploy/install_tokio.sh` | `Deploy` | 46 | 1,781 |
| `deploy/vortex-m2b-spot.service` | `Deploy` | 23 | 572 |
| `deploy/vortex-m2b-spot.timer` | `Deploy` | 12 | 230 |
| `deploy/vspot` | `Deploy` | 79 | 2,095 |
| `src/lib.rs` | `Root/CI` | 3 | 61 |
| `src/main.rs` | `Infrastructure` | 312 | 11,984 |
| `src/domain/mod.rs` | `Domain` | 6 | 103 |
| `src/domain/accounting.rs` | `Domain` | 712 | 24,525 |
| `src/domain/error.rs` | `Domain` | 156 | 5,007 |
| `src/domain/indicators.rs` | `Domain` | 269 | 8,148 |
| `src/domain/ports.rs` | `Domain` | 50 | 2,042 |
| `src/domain/strategy.rs` | `Domain` | 663 | 22,905 |
| `src/domain/types.rs` | `Domain` | 668 | 17,986 |
| `src/application/mod.rs` | `Application` | 2 | 34 |
| `src/application/cycle.rs` | `Application` | 1,111 | 41,122 |
| `src/application/reconcile.rs` | `Application` | 118 | 4,187 |
| `src/infrastructure/mod.rs` | `Infrastructure` | 4 | 78 |
| `src/infrastructure/bybit_spot_v5.rs` | `Infrastructure` | 1,011 | 34,089 |
| `src/infrastructure/config.rs` | `Infrastructure` | 288 | 9,781 |
| `src/infrastructure/state_store.rs` | `Infrastructure` | 694 | 23,690 |
| `src/infrastructure/telegram.rs` | `Infrastructure` | 94 | 2,800 |
| `tests/application_e2e_mocks.rs` | `Tests` | 814 | 27,036 |
| `tests/domain_physics.rs` | `Tests` | 704 | 25,467 |
| `tests/red_team_hygiene.rs` | `Tests` | 264 | 7,645 |
| **Total Línea Base (31 archivos)** | — | **9,012** | **297,680** |

> **Nota de Documentación (`DOCS/`):** Además de los 31 archivos de línea base anteriores, en esta fase se entregan bajo capa `Docs` los 3 documentos gobernados: `DOCS/INDEX.md`, `DOCS/PHASE_1_ARCHITECTURE_AND_VERIFICATION_REPORT.md` y `DOCS/RUNBOOK_TOKIO_PAPER.md` (verificados en UTF-8 sin BOM, LF puro y cubiertos por `tests/red_team_hygiene.rs:34`).

---

## Sección 2: Auditoría Matemática y Física (`Fórmula LaTeX -> archivo:línea`)

### 2.1 Universo, Análisis Dimensional y Tipos Numéricos

- **Universo fijo (`UNIVERSE_LEN = 5`):** `BTCUSDT`, `ETHUSDT`, `SOLUSDT`, `LINKUSDT`, `AVAXUSDT` definidos mediante el enum ordenado `Symbol` (`src/domain/types.rs:9-66`).
- **Aritmética de punto fijo (`i128`):**
  - `Money1e8`, `Qty1e8`, `Price1e8`: escala `SCALE_1E8 = 100_000_000` ($10^8$, resolución $10^{-8}$ USDT o moneda base) en `src/domain/types.rs:5,74-238`.
  - `Rate1e10`: escala `RATE_SCALE_1E10 = 10_000_000_000` ($10^{10}$, donde `1.0` = $10^{10}$ y `from_bps(bps) = bps * 1_000_000`) en `src/domain/types.rs:6,240-261`.
  - Multiplicación-división con redondeo simétrico al entero más cercano (`mul_div_round`, `src/domain/types.rs:263-282`) y truncado al piso para valores no negativos (`mul_div_floor_nonneg`, `src/domain/types.rs:284-293`), ambos usando `checked_mul` sobre `i128`.
  - Cuantización por paso de lote (`Qty1e8::floor_to_step`, `src/domain/types.rs:160-170`):
    $$\operatorname{floor\_to\_step}(q, \Delta q) = \left\lfloor \frac{q}{\Delta q} \right\rfloor \cdot \Delta q \qquad (\Delta q > 0,\; q \ge 0)$$
- **Aislamiento de coma flotante (`f64`):** Exclusivamente utilizado para indicadores causales, señales TSMOM y pesos objetivo (`VolEstimate` en `src/domain/indicators.rs:17-21`, `SignalOutput` en `src/domain/strategy.rs:24-33`). Cada vela de entrada se valida en `Candle::validate` (`src/domain/types.rs:434-466`) exigiendo:
  $$t_{\text{open}} \ge 0, \quad O, H, L, C \in (0, +\infty) \cap \mathbb{R}, \quad V, T \in [0, +\infty) \cap \mathbb{R}, \quad L \le \min(O, C) \le \max(O, C) \le H$$

### 2.2 Volatilidad Causal Garman-Klass EWMA

Para cada vela cerrada $t \in \{0, \dots, M-1\}$:

1. **Constante de Garman-Klass y piso por barra** (`src/domain/indicators.rs:4-5`):
   $$c_{\text{GK}} = 2 \ln(2) - 1, \qquad \sigma_{\text{floor}} = 10^{-4}$$
2. **Varianza instantánea por barra con clamp no negativo** (`src/domain/indicators.rs:53-55`):
   $$v_t = \max\!\left(0,\;\; \frac{1}{2}\ln^2\!\left(\frac{H_t}{L_t}\right) - c_{\text{GK}}\,\ln^2\!\left(\frac{C_t}{O_t}\right)\right)$$
   Se verifica finitud de $v_t$ (`src/domain/indicators.rs:56-60`).
3. **Actualización causal EWMA** (`src/domain/indicators.rs:43-44,62-66`):
   $$\alpha = \frac{2}{N_{\text{eff}} + 1}, \qquad \tilde{\sigma}_t^2 = \begin{cases} v_0 & t = 0 \\ \alpha v_t + (1 - \alpha)\tilde{\sigma}_{t-1}^2 & t \ge 1 \end{cases}$$
   Con parámetros congelados:
   - `B2-01` (`1h`): `B2_01_N_EFF = 84` (`src/domain/indicators.rs:9`), $\alpha_{1\text{h}} = \frac{2}{85}$.
   - `A0_4H` (`4h`): `A0_4H_N_EFF = 21` (`src/domain/indicators.rs:13`), $\alpha_{4\text{h}} = \frac{2}{22}$.
4. **Desviación por barra con piso y anualización** (`src/domain/indicators.rs:10,14,45,68-69`):
   $$\sigma_{\text{bar}, t} = \max\!\left(\sqrt{\max(0, \tilde{\sigma}_t^2)},\; 10^{-4}\right), \qquad \sigma_{\text{ann}, t} = \sigma_{\text{bar}, t} \sqrt{B_{\text{day}} \cdot 365}$$
   - `B2-01` (`B2_01_BARS_PER_DAY = 24.0`): factor anualizador $\sqrt{24 \times 365} = \sqrt{8760}$.
   - `A0_4H` (`A0_4H_BARS_PER_DAY = 6.0`): factor anualizador $\sqrt{6 \times 365} = \sqrt{2190}$.

### 2.3 Señal TSMOM Multi-Horizonte

Definida en `tsmom_multi_horizon_at` (`src/domain/indicators.rs:106-161`) con $\varphi = 1.618033988749895$ (`PHI`, `src/domain/indicators.rs:7`):

$$z_{t,k} = \frac{\ln(C_t / C_{t-k})}{\sigma_{\text{bar}, t}\sqrt{k}}, \qquad S_t = \sum_{(k, w_k) \in \mathcal{L}} w_k \tanh\!\left(\frac{z_{t,k}}{\varphi}\right)$$

- **Conjunto de horizontes `B2-01` (`1h`)** (`src/domain/indicators.rs:11`, requiere índice $t \ge 504$, i.e. $\ge 505$ velas en `src/domain/strategy.rs:259`, y el ciclo operacional exige `MIN_CLOSED_1H_CANDLES = 600` en `src/domain/strategy.rs:21`):
  $$\mathcal{L}_{1\text{h}} = \{(72,\, 0.25),\; (168,\, 0.35),\; (504,\, 0.40)\}$$
- **Conjunto de horizontes `A0_4H` (`4h`)** (`src/domain/indicators.rs:15`, requiere índice $t \ge 126$, i.e. $\ge 127$ velas en `src/domain/strategy.rs:272`, y el ciclo sombra exige `MIN_CLOSED_4H_CANDLES = 147` en `src/domain/strategy.rs:22`):
  $$\mathcal{L}_{4\text{h}} = \{(18,\, 0.25),\; (42,\, 0.35),\; (126,\, 0.40)\}$$

Dado que $\sum w_k = 1.00$ y $\tanh(\cdot) \in (-1, 1)$, la señal compuesta cumple $S_t \in (-1, 1)$ de forma acotada.

### 2.4 Pesos Vol-Targeted (`LONG_FLAT`)

Implementado en `compute_vol_targeted_weights` (`src/domain/strategy.rs:160-191`) con constantes congeladas (`src/domain/strategy.rs:12-15`):

$$\sigma_{\text{target}} = 0.50, \quad W_{\max} = 0.40, \quad L_{\max} = 1.00, \quad \sigma_{\text{vol\_floor}} = 0.15$$

Para cada símbolo $i \in \{0, \dots, 4\}$:

$$w_i^{\text{raw}} = \operatorname{clamp}\!\left(\frac{\sigma_{\text{target}} / 4}{\max(\sigma_{\text{ann}, i},\; 0.15)} \cdot S_i,\;\; 0,\;\; W_{\max}\right)$$

Escalado por tope de exposición agregada Spot $0\times$ ($L_{\max} = 1.00$):

$$s = \begin{cases} \min\!\left(1,\;\; \dfrac{L_{\max}}{\sum_{j=0}^{4} w_j^{\text{raw}}}\right) & \text{si } \sum_{j=0}^{4} w_j^{\text{raw}} > 0 \\ 1 & \text{en otro caso} \end{cases}, \qquad w_i^{\text{target}} = w_i^{\text{raw}} \cdot s$$

Se garantiza $w_i^{\text{target}} \in [0, 0.40]$ (cero cortos) y $\sum_{i=0}^{4} w_i^{\text{target}} \le 1.00$ (cero apalancamiento).

### 2.5 Deadband, Anti-Dust, `SELL_FIRST_BUY_SECOND` y Reserva `0.3%`

1. **Predicado de rebalanceo y umbral de liquidación** (`src/domain/strategy.rs:16-19,280-284`):
   - `DEADBAND_DELTA = 0.20`, `DEADBAND = 0.08` ($= 0.20 \times W_{\max}$), `LIQUIDATE_TARGET_THRESH = 0.015`, `LIQUIDATE_CURRENT_THRESH = 0.02`:
     $$\text{should\_rebalance}(w_i^{\text{target}}, w_i^{\text{cur}}) \iff |w_i^{\text{target}} - w_i^{\text{cur}}| \ge 0.08 \;\;\lor\;\; \left(w_i^{\text{target}} < 0.015 \;\land\; w_i^{\text{cur}} \ge 0.02\right)$$
   - Si `raw_target_w < 0.015`, el objetivo efectivo se colapsa a $0.0$ (`src/domain/strategy.rs:361-365`) y se planifica liquidación total si la cantidad redondeada cumple `sellable >= min_order_qty` y nocional $\ge$ `min_order_amt` (`src/domain/strategy.rs:371-387`).
2. **Escalamiento Anti-Dust en ventas parciales** (`src/domain/strategy.rs:395-422`):
   Al calcular una venta parcial `sell_qty`, se evalúa el remanente `rem_qty = pos_qty - sell_qty` y su valor `rem_val`. Si `rem_qty > 0` y (`rem_val < min_order_amt` o `rem_qty < min_order_qty`), la orden escala automáticamente a **liquidación total** (`sell_qty = floor_to_step(pos_qty, base_precision)`, `full_liquidation = true`).
3. **Ordenamiento estricto `SELL_FIRST_BUY_SECOND` y Reserva de Buy `0.3%`** (`src/domain/strategy.rs:20,433-491`):
   - Todas las órdenes `Sell` preceden estrictamente a toda orden `Buy`.
   - Flujo proyectado de efectivo post-ventas:
     $$\text{cash}_{\text{proj}} = \text{cash}_0 + \sum_{s \in \text{Sells}} \max\!\left(0,\;\; Q^{\text{gross}}_s - \operatorname{round}\!\left(\frac{Q^{\text{gross}}_s \cdot r_{\text{fee}}}{10^{10}}\right)\right)$$
   - Presupuesto máximo de compra con reserva de deslizamiento `BUY_SLIP_RESERVE_BPS = 30` ($0.30\%$):
     $$\text{budget} = \left\lfloor \frac{\text{cash}_{\text{proj}} \cdot (10000 - 30)}{10000} \right\rfloor$$
   - Si la suma deseada de compras $\sum D_j > \text{budget}$, cada compra se prorratea a $\left\lfloor \frac{D_j \cdot \text{budget}}{\sum D_m} \right\rfloor$, omitiendo órdenes por debajo de `min_order_amt` o `min_order_qty`.

### 2.6 Filtro Estricto de Velas Cerradas y Agregador `1h -> 4h` UTC

- **Filtro estricto `1h`** (`filter_strictly_closed_1h`, `src/domain/strategy.rs:69-90`): conserva solo velas con $t_{\text{open}} + 3\,600\,000 \le t_{\text{server}}$; ordena ascendentemente y deduplica por `open_ts_ms` (prevalece el último registro).
- **Agregador `1h -> 4h`** (`aggregate_1h_to_4h`, `src/domain/strategy.rs:92-153`): agrupa por cubeta UTC $t_0 = \lfloor t_{\text{open}} / 14\,400\,000 \rfloor \cdot 14\,400\,000$. Solo emite una vela `4h` cuando están presentes los 4 slots exactos $\{t_0, t_0+1\text{h}, t_0+2\text{h}, t_0+3\text{h}\}$ (descartando cubetas incompletas):
  $$O_{4\text{h}} = O_0, \quad H_{4\text{h}} = \max_{k=0..3} H_k, \quad L_{4\text{h}} = \min_{k=0..3} L_k, \quad C_{4\text{h}} = C_3, \quad V_{4\text{h}} = \sum_{k=0}^{3} V_k, \quad T_{4\text{h}} = \sum_{k=0}^{3} T_k$$
- **Compuerta de cierre `4h` UTC** (`is_4h_utc_close`, `src/domain/strategy.rs:155-158`): `(last_closed_1h_open_ts_ms + ONE_HOUR_MS).rem_euclid(FOUR_HOURS_MS) == 0`.

---

## Sección 3: Arquitectura Hexagonal Clean DDD

### 3.1 Prueba de Regla de Dependencia Hacia Adentro

- **`src/domain/`** (`mod.rs`, `types.rs`, `error.rs`, `ports.rs`, `indicators.rs`, `strategy.rs`, `accounting.rs`): importa **únicamente `std`** (`std::fmt`, `std::error::Error`, `std::f64::consts::LN_2`). Cero referencias a `ureq`, `serde`, `serde_json`, `hmac`, `sha2`, `hex`, `tokio` o E/S de sistema de archivos. Verificado automáticamente por `tests/red_team_hygiene.rs:87-130`.
- **`src/application/`** (`cycle.rs`, `reconcile.rs`): depende exclusivamente de `crate::domain::*` y consume infraestructura solo a través de trait objects `&dyn *Port` inyectados en `RunHourlyCycleUseCase::new` (`src/application/cycle.rs:62-76`).
- **`src/infrastructure/`** (`bybit_spot_v5.rs`, `config.rs`, `state_store.rs`, `telegram.rs`) y **`src/main.rs`**: implementan los traits de `src/domain/ports.rs` usando runtime síncrono mínimo (`ureq` con `rustls`, `serde_json`, `hmac`, `sha2`, `hex`, sin `tokio`).

### 3.2 Matriz Puerto / Adaptador

| Puerto (`src/domain/ports.rs`) | Responsabilidad | Adaptador `--paper` (`src/main.rs:41-49`) | Adaptador `--live-spot` (`src/main.rs:41-49`) |
|---|---|---|---|
| `ClockPort` (`:7-9`) | Tiempo UTC local (`now_ms`) | `SystemClock` (`bybit_spot_v5.rs:23-33`) | `SystemClock` (`bybit_spot_v5.rs:23-33`) |
| `MarketDataPort` (`:11-22`) | `/v5/market/time`, `instruments-info`, `kline`, `tickers` (solo lectura pública) | `BybitSpotV5Client` (`bybit_spot_v5.rs:429-620`) | `BybitSpotV5Client` (`bybit_spot_v5.rs:429-620`) |
| `SpotExecutionPort` (`:24-32`) | Wallet UNIFIED, consulta `orderLinkId`, envío `/v5/order/create` | `PaperNoopExecutionPort` (falla cerrado si se invoca, `bybit_spot_v5.rs:36-60`) | `BybitSpotV5Client` (`bybit_spot_v5.rs:622-796`) |
| `StateStorePort` + `RunLockGuard` (`:34-46`) | `run.lock`, `DRAIN_MODE.lock`, `HALT_LOCK`, estados JSON atómicos y ledger JSONL | `FsStateStore` (`state_store.rs:23-242`) | `FsStateStore` (`state_store.rs:23-242`) |
| `NotifierPort` (`:48-50`) | Alertas operacionales best-effort | `TelegramNotifier` (`telegram.rs:10-94`) | `TelegramNotifier` (`telegram.rs:10-94`) |

### 3.3 Máquina de Estados Finitos (FSM) del Ciclo Horario (`RunHourlyCycleUseCase::run`)

Secuencia determinista en `src/application/cycle.rs:79-462`:

1. **`ACQUIRE_RUN_LOCK` (`:80`)**: `FsStateStore::acquire_run_lock` crea `state/run.lock` con `OpenOptions::create_new(true)` (`O_CREAT | O_EXCL`, `state_store.rs:95-124`). Si ya existe retorna `InfraError::LockBusy`; el guard RAII `FsRunLockGuard::drop` elimina el archivo al salir de ámbito.
2. **`LIVE_SAFETY_PREGATE` (`:82-92`)**: si `mode == LiveSpot`, verifica el Double Safety Lock (`BYBIT_SPOT_LIVE_EXECUTION=true` y `BYBIT_SPOT_ACK_LIVE` exacto) y la ausencia de `state/HALT_LOCK` (`AppError::HaltLockActive`). No aborta por `state/DRAIN_MODE.lock`, permitiendo drenaje ordenado mediante ventas de salida.
3. **`CLOCK_SKEW_GUARD` (`:94-103`)**: consulta `/v5/market/time` y `clock.now_ms()`; si `|local - server| > max_clock_skew_ms` (default `2000 ms`) aborta con `InfraError::ClockSkew`.
4. **`INGEST_AND_ALIGN` (`:105-128`)**: descarga `instruments-info`, `tickers` (`last > 0`) y `700` velas `1h` por símbolo; filtra estrictamente velas cerradas (`filter_strictly_closed_1h`) y alinea los 5 pares al último timestamp común (`align_candles_to_common_latest`, mínimo `600` velas cerradas `1h`).
5. **`IDEMPOTENCY_GATE` (`:130-135`)**: si `closed_hour_ts_ms <= prev_state.last_processed_hour_ts_ms`, retorna `CycleOutcome::AlreadyProcessed` sin emitir órdenes, ledger ni alertas.
6. **`LOAD_OR_BOOTSTRAP_LEDGER` (`:137-158`)**: carga `state/primary_state.json` (validando `assert_book_zero_drift`) o inicializa libro nuevo (`Paper`: `paper_initial_cash_1e8`; `LiveSpot`: `init_live_ledger_from_wallet` desde `UNIFIED` wallet).
7. **`LOCKFILE_EVAL` (`:160-161`)**: evalúa `DRAIN_MODE.lock` (`drain_mode`) y `HALT_LOCK` (`halt_lock`).
8. **`LIVE_WALLET_RECONCILE` (`:163-244`, solo `LiveSpot`)**: llama `reconcile_live_wallet` (`reconcile.rs:24-73`). Si `!had_recent_orders` (`RECENT_ORDER_WINDOW_MS = 120_000`) y `|wallet_usdt - ledger_cash| > 0.001 USDT`, registra depósito/retiro externo neutral al TWR y audita `EXTERNAL_DEPOSIT`/`EXTERNAL_WITHDRAWAL`. Si algún coin base difiere `> base_precision`, escribe `HALT_LOCK`, audita `RECONCILIATION_MISMATCH` y aborta.
9. **`PRE_TRADE_MTM_AND_DD_GATE` (`:246-260`)**: ejecuta `ledger.mark_to_market(&prices)`. Si drawdown TWR `> 25%`, ejecuta `handle_hard_drawdown_halt` (`:464-507`): `halted = true`, escribe `state/HALT_LOCK`, sella evento `HARD_DRAWDOWN_HALT` y notifica.
10. **`SIGNAL_AND_PLAN` (`:262-275`)**: `compute_b2_01_signals` + `plan_rebalance_orders` con `allow_buys = !drain_mode && !halt_lock && !ledger.is_halted()`. Tanto en `--paper` como en `--live-spot`, bajo `DRAIN_MODE.lock` se bloquean estrictamente todas las compras (`Buy`) y se permiten ventas de salida/reducción (`Sell`).
11. **`EXECUTE_ORDERS` (`:277-371`)**:
    - `Paper`: `simulate_paper_orders` (`:731-842`), slippage `2 bps` desfavorable (`+2 bps` Buy / `-2 bps` Sell), comisión `10 bps` con asimetría Spot V5, cero llamadas privadas.
    - `LiveSpot`: por cada orden, verifica `query_order_by_link_id` antes de emitir `/v5/order/create`, limita el gasto `Buy` al `9970/10000` del efectivo en libro (`:307-320`), aplica la guarda defensiva `guard_live_order_request` (`:331-336,844-856`) para rechazar cualquier `OrderSide::Buy` si `drain_mode || halt_lock || ledger.is_halted()`, ejecuta `execute_spot_order`, aplica `apply_live_fill` y sella `ORDER_FILL` en el ledger.
12. **`POST_TRADE_MTM_AND_PERSIST` (`:373-415`)**: marca a mercado si hubo fills, exige `ledger.assert_zero_drift(&prices)?` y persiste atómicamente `state/primary_state.json`.
13. **`SHADOW_4H_GATE` (`:417-424, 509-623`)**: si `is_4h_utc_close(common_last_open_ts)` y `closed_hour_ts_ms > last_processed_4h_close_ts_ms`, agrega `1h -> 4h`, valida `>= 147` velas `4h`, calcula `compute_a0_4h_signals`, simula libro sombra `A0_4H` en `Paper`, verifica `assert_zero_drift`, persiste `state/m2b_shadow_4h_state.json` y registra `SHADOW_4H_CYCLE`.
14. **`BEST_EFFORT_NOTIFICATIONS` (`:426-451`)**: envía resumen de órdenes (incluyendo `drain_mode=true/false`) y resumen diario UTC vía `NotifierPort` descartando errores de red (`let _ = ...`) para nunca abortar persistencia.

### 3.4 Double Safety Lock para `--live-spot` y Semántica Unificada de `DRAIN_MODE.lock`

La ejecución real de órdenes Spot en Bybit V5 exige la conjunción simultánea de **4 guardas fail-closed obligatorias** (`src/main.rs:31`, `src/infrastructure/config.rs:10,124-132`, `src/application/cycle.rs:82-92,209-243,464-507`):

1. Bandera explícita de CLI: `vortex-m2b-spot run --live-spot` (`run` sin argumentos o con `--paper` ejecuta `ExecutionMode::Paper`).
2. Variable de entorno `BYBIT_SPOT_LIVE_EXECUTION=true`.
3. Confirmación literal exacta `BYBIT_SPOT_ACK_LIVE=YES_I_ACCEPT_0X_SPOT_RISK`.
4. Ausencia de `state/HALT_LOCK` (`AppError::HaltLockActive`), junto con parada dura automática ante cualquier `RECONCILIATION_MISMATCH` (`base_discrepancies`) o `HARD_DRAWDOWN_HALT` (`> 25%` TWR).

**Comportamiento unificado de `state/DRAIN_MODE.lock` (`--paper` y `--live-spot`):**
- La presencia de `state/DRAIN_MODE.lock` **no aborta** el ciclo horario en `--live-spot`; unifica su semántica con `--paper` para permitir un drenaje ordenado de cartera:
  - **Bloqueo estricto de compras:** `allow_buys` se fuerza a `false` en `plan_rebalance_orders` (`src/application/cycle.rs:264`) y una guarda defensiva adicional (`guard_live_order_request`, `:331-336,844-856`) rechaza cualquier `OrderRequest` con `OrderSide::Buy` justo antes de `SpotExecutionPort::execute_spot_order`.
  - **Ejecución normal de ventas de salida/reducción:** las órdenes `OrderSide::Sell` generadas por el planificador (reducción de peso o liquidación total por `TSMOM` negativo) se ejecutan normalmente en `--paper` y `--live-spot`, preservando idempotencia `orderLinkId`, conciliación de wallet, `Drift == 0` y trazabilidad explícita `drain_mode=true/false` tanto en `CYCLE_EXECUTED` (`src/main.rs:78-87`) como en las alertas Telegram (`src/application/cycle.rs:426-437`).

---

## Sección 4: Contabilidad `i128` (`Drift == 0`), Asimetría Spot V5, Depósitos `$50/15d` e Idempotencia `orderLinkId`

### 4.1 Invariantes de Conservación (`drift_book_1e8 == 0` y `drift_mtm_1e8 == 0`)

Implementadas en `SpotPortfolioLedger` (`src/domain/accounting.rs:272-332`):

- **Conservación en libros (`drift_book_1e8`, `src/domain/accounting.rs:272-293`):**
  $$\Delta_{\text{book}} = \left(C_{\text{USDT}} + \sum_{i=0}^{4} B_i\right) - \left(D_{\text{net}} + P_{\text{realized, gross}} - F_{\text{sell, USDT}}\right) = 0$$
  donde $C_{\text{USDT}}$ es `cash_1e8`, $B_i$ es `positions[i].cost_basis_usdt_1e8`, $D_{\text{net}}$ es `total_deposits_1e8`, $P_{\text{realized, gross}}$ es `realized_pnl_gross_1e8` y $F_{\text{sell, USDT}}$ es `cum_sell_fees_usdt_1e8`.
- **Conservación Mark-to-Market (`drift_mtm_1e8`, `src/domain/accounting.rs:295-313`):**
  $$E(p) = C_{\text{USDT}} + \sum_{i=0}^{4} \left\lfloor \frac{q_i \cdot p_i}{10^8} \right\rfloor, \qquad U(p) = \sum_{i=0}^{4} \left(\left\lfloor \frac{q_i \cdot p_i}{10^8} \right\rfloor - B_i\right)$$
  $$\Delta_{\text{mtm}}(p) = E(p) - \left(D_{\text{net}} + (P_{\text{realized, gross}} - F_{\text{sell, USDT}}) + U(p)\right) = 0$$

`assert_book_zero_drift` y `assert_zero_drift` se ejecutan tras toda mutación contable (`from_snapshot`, `apply_buy_fill`, `apply_sell_fill`, `record_external_deposit`, `record_external_withdrawal`, `mark_to_market` y fin de ciclo horario).

### 4.2 Asimetría de Comisiones en Bybit Spot V5

1. **Compra Spot (`apply_buy_fill`, `src/domain/accounting.rs:334-401`; `marketUnit="quoteCoin"`, `bybit_spot_v5.rs:711`):**
   - Bybit Spot V5 cobra la comisión de compra en la **moneda base** (verificado en `/v5/execution/list` comparando `feeCurrency == symbol.base_coin()`, `bybit_spot_v5.rs:405-409`).
   - El efectivo USDT decrece exactamente en `quote_spent_1e8`, y el `cost_basis_usdt_1e8` de la posición aumenta en `quote_spent_1e8` completo (`accounting.rs:385-389`).
   - La cantidad neta acreditada se trunca a la precisión de lote del exchange:
     $$q_{\text{net}} = \operatorname{floor\_to\_step}\!\left(q_{\text{gross}} - f_{\text{base}},\; \Delta q_{\text{base}}\right)$$
     (`accounting.rs:375-377`).
   - Se acumulan `cum_buy_fees_base_1e8[i]` y el estimado informativo `cum_buy_fees_usdt_est_1e8 = round(quote_spent * f_base / q_gross)` sin alterar la igualdad $\Delta_{\text{book}} = 0$ (la comisión queda implícita como mayor costo unitario por moneda neta).
2. **Venta Spot (`apply_sell_fill`, `src/domain/accounting.rs:403-467`; `marketUnit="baseCoin"`, `bybit_spot_v5.rs:723`):**
   - Bybit cobra la comisión de venta en **USDT**.
   - Se libera el costo base proporcional (`pos.cost_basis_usdt_1e8` exacto si es venta total, o `mul_div_round(cost_basis, sell_qty, pos.qty)` si es parcial, `accounting.rs:435-443`).
   - `cash_1e8 += gross_quote - fee_usdt`; `realized_pnl_gross_1e8 += gross_quote - released_cost`; `cum_sell_fees_usdt_1e8 += fee_usdt`.

### 4.3 Neutralidad TWR ante Depósitos Externos (`$50 / 15d`) y Corte por Drawdown (`> 25%`)

- **Depósitos/retiros externos (`record_external_deposit` / `record_external_withdrawal`, `src/domain/accounting.rs:469-542`):**
  Para soportar aportes periódicos (ej. `$50 USDT` cada `15 días`) sin distorsionar el rendimiento ni el PnL de trading:
  1. Primero se ejecuta `self.mark_to_market(prices)?` **antes** de sumar el depósito, fijando el rendimiento hasta ese instante en `twr_index_1e10`.
  2. Luego se suma `amount_1e8` simultáneamente a `cash_1e8`, `total_deposits_1e8` y `last_marked_equity_1e8` (`accounting.rs:488-494`).
  3. En la siguiente marca a mercado:
     $$\text{TWR}_{t+1} = \operatorname{round}\!\left(\text{TWR}_t \cdot \frac{E_{t+1}}{\text{last\_marked\_equity}_t}\right)$$
     Como `last_marked_equity` incorpora exactamente $+D$, la inyección de capital produce $\Delta \text{TWR} = 0$, $\Delta \text{PnL}_{\text{trading}} = 0$ y $\Delta_{\text{book}} = \Delta_{\text{mtm}} = 0$ (verificado por `tests/domain_physics.rs:567-625`).
- **Hard Drawdown Halt (`> 25%`, `src/domain/accounting.rs:7,257-270,564-573`):**
  $$\text{DD}_{1\text{e}10} = \left\lfloor \frac{(\text{TWR}_{\text{peak}} - \text{TWR}_{\text{index}}) \cdot 10^{10}}{\text{TWR}_{\text{peak}}} \right\rfloor$$
  Si $\text{DD}_{1\text{e}10} > 2\,500\,000\,000$ (`MAX_HARD_DD_RATE_1E10`, estrictamente mayor que `25.00%`), el ledger fija `halted = true`, emite `DomainEvent::HardDrawdownHalt`, escribe `state/HALT_LOCK` y sella `HARD_DRAWDOWN_HALT`.

### 4.4 Idempotencia `orderLinkId`, Recuperación de Fallos, Persistencia Atómica y Ledger FNV-1a

- **`orderLinkId` determinista (`make_order_link_id`, `src/domain/types.rs:408-415`):**
  Formato `m2b_{closed_hour_ts_ms}_{SYMBOL}_{side}` (ej. `m2b_1700000000000_BTCUSDT_buy`, 29–30 caracteres, dentro del límite de 36 caracteres de Bybit V5).
- **Recuperación ante fallos de red/reintentos (`src/application/cycle.rs:304-337`, `src/infrastructure/bybit_spot_v5.rs:285-375,740-795`):**
  1. Antes de enviar `POST /v5/order/create`, consulta `query_order_by_link_id` (`/v5/order/realtime` -> `/v5/order/history`). Si ya existe fill, reutiliza el `FillReport` sin reenviar orden.
  2. Si `POST /v5/order/create` falla con error de transporte o HTTP `5xx` (estado incierto), vuelve a consultar `query_order_by_link_id` antes de propagar error.
  3. Tras creación exitosa, sondea hasta `MAX_FILL_POLL_ATTEMPTS = 6` y obtiene comisiones exactas desde `/v5/execution/list`.
- **Persistencia atómica (`atomic_write_bytes`, `src/infrastructure/state_store.rs:259-316`):**
  Escribe `{archivo}.tmp.{pid}`, ejecuta `file.sync_all()` y realiza `fs::rename` atómico sobre `state/primary_state.json`, `state/m2b_shadow_4h_state.json` y `state/HALT_LOCK`.
- **Audit ledger sellado FNV-1a 64-bit (`src/domain/types.rs:399-406`, `src/infrastructure/state_store.rs:199-257`):**
  Cada entrada `AuditEntryPayloadDto` se serializa canónicamente y se sella con FNV-1a 64-bit (offset `0xcbf29ce484222325`, primo `0x0100000001b3`) en el campo `fnv1a64_hex` antes de hacer `append` + `sync_all()` en `logs/m2b_audit_ledger.jsonl`. `verify_audit_ledger_line` detecta cualquier alteración de 1 bit.

---

## Sección 5: Evidencia Bruta de Verificación

Salidas literales obtenidas directamente en PowerShell dentro de `D:\Desarrollo\VORTEX-M2B-SPOT`:

### 5.1 Formato, Clippy y Suite Completa de Pruebas (`fmt`, `clippy`, `test`)

```text
PS> cargo fmt --check; echo "FMT_EXIT=$LASTEXITCODE"
FMT_EXIT=0

PS> cargo clippy --all-targets -- -D warnings; echo "CLIPPY_EXIT=$LASTEXITCODE"
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.34s
CLIPPY_EXIT=0

PS> cargo test --all-targets; echo "TEST_EXIT=$LASTEXITCODE"
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.28s
     Running unittests src\lib.rs (target\debug\deps\vortex_m2b_spot-c5232136fe237f23.exe)

running 17 tests
test application::cycle::tests::live_spot_double_safety_lock_denies_without_ack ... ok
test application::reconcile::tests::reconcile_detects_deposit_and_base_mismatch ... ok
test domain::accounting::tests::deposit_and_withdrawal_preserve_twr_and_drift ... ok
test domain::accounting::tests::buy_sell_fee_asymmetry_and_zero_drift ... ok
test domain::accounting::tests::hard_drawdown_halt_triggers_above_25_percent ... ok
test domain::indicators::tests::gk_ewma_rejects_invalid_inputs ... ok
test domain::indicators::tests::gk_ewma_respects_floor_and_causal_update ... ok
test domain::indicators::tests::tsmom_multi_horizon_works_and_checks_history ... ok
test domain::strategy::tests::filter_and_aggregate_1h_to_4h_works ... ok
test domain::strategy::tests::plan_rebalance_sell_first_anti_dust_and_buy_reserve ... ok
test domain::strategy::tests::vol_targeted_weights_respect_caps_and_l_max ... ok
test infrastructure::bybit_spot_v5::tests::test_paper_noop_execution_port_errors ... ok
test infrastructure::bybit_spot_v5::tests::test_sign_bybit_v5_deterministic ... ok
test infrastructure::config::tests::test_dotenv_parser_and_token_fallback ... ok
test domain::strategy::tests::signals_b2_01_and_a0_4h_smoke ... ok
test application::cycle::tests::paper_cycle_executes_idempotently_and_updates_shadow_4h ... ok
test infrastructure::state_store::tests::test_audit_seal_and_verify ... ok

test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running unittests src\main.rs (target\debug\deps\vortex_m2b_spot-61747baddcac5d48.exe)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests\application_e2e_mocks.rs (target\debug\deps\application_e2e_mocks-5a0e2b5bf175164d.exe)

running 6 tests
test bybit_v5_hmac_sha256_known_test_vectors ... ok
test live_spot_double_safety_lock_and_lockfile_rejections ... ok
test multi_hour_crossing_4h_utc_boundary_and_idempotency ... ok
test telegram_notifier_failure_does_not_abort_cycle_or_persistence ... ok
test fs_state_store_atomic_persistence_audit_seal_and_run_lock_contention ... ok
test drain_mode_in_paper_blocks_new_buys_and_allows_exit_sells ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s

     Running tests\domain_physics.rs (target\debug\deps\domain_physics-fa9ee5d11e2561bc.exe)

running 8 tests
test anti_dust_escalation_and_below_5_usdt_buy_skipped ... ok
test strict_sell_first_buy_second_with_zero_initial_cash_and_reserve ... ok
test garman_klass_ewma_closed_form_floor_nan_and_annualization ... ok
test external_deposit_plus_50_usdt_is_neutral_to_twr_and_trading_pnl ... ok
test hard_drawdown_halt_strictly_above_25_percent ... ok
test spot_v5_asymmetric_fee_accounting_and_zero_drift ... ok
test tsmom_1h_72_168_504_closed_form_and_1h_to_4h_aggregator ... ok
test zero_lookahead_b2_01_bit_identical_after_future_perturbation ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running tests\red_team_hygiene.rs (target\debug\deps\red_team_hygiene-bc4f2c254d6e5e27.exe)

running 4 tests
test src_has_zero_panic_unwrap_expect_todo_unimplemented_or_unsafe ... ok
test domain_has_zero_external_crate_imports ... ok
test gitignore_env_example_and_secret_hygiene ... ok
test governed_files_are_utf8_no_bom_pure_lf_and_zero_banned_strings ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

TEST_EXIT=0
```

### 5.2 Compilación Release y Health Check en Vivo contra Bybit V5

```text
PS> cargo build --release; echo "BUILD_EXIT=$LASTEXITCODE"
    Finished `release` profile [optimized] target(s) in 0.20s
BUILD_EXIT=0
```

Observación inicial transitoria de guarda de reloj (el reloj local Windows presentaba desfase transitorio antes de sincronizar NTP, demostrando el rechazo fail-closed cuando `skew > 2000ms`):
```text
PS> .\target\release\vortex-m2b-spot.exe health; echo "HEALTH_EXIT=$LASTEXITCODE"
ERROR: clock skew 4149ms exceeds max_allowed 2000ms
HEALTH_EXIT=1
```

Ejecución subsiguiente tras convergencia NTP local (con limpieza inmediata de `state/` y `logs/` temporales):
```text
PS> .\target\release\vortex-m2b-spot.exe health; echo "HEALTH_EXIT=$LASTEXITCODE"
=== VORTEX-M2B-SPOT HEALTH OK ===
Bybit Base URL   : https://api.bybit.com
Server Time (ms) : 1795490203344
Clock Skew (ms)  : 227 (limit 2000)
State Dir        : state (rw)
Logs Dir         : logs (rw)
Instrument BTCUSDT  | status=Trading | base_step=0.000001 | min_qty=0.000001 | min_amt=5.00
Instrument ETHUSDT  | status=Trading | base_step=0.00001 | min_qty=0.00001 | min_amt=5.00
Instrument SOLUSDT  | status=Trading | base_step=0.0001 | min_qty=0.0001 | min_amt=5.00
Instrument LINKUSDT | status=Trading | base_step=0.001 | min_qty=0.001 | min_amt=5.00
Instrument AVAXUSDT | status=Trading | base_step=0.001 | min_qty=0.001 | min_amt=5.00
HEALTH_EXIT=0
```

### 5.3 Escaneo de Higiene, Codificación y Estado Git

```text
PS> # Verificacion de patrones prohibidos en src/ y crates externos en src/domain/
src_forbidden_hits=0
domain_external_crate_hits=0

PS> git status -sb
## No commits yet on main
?? .env.example
?? .gitattributes
?? .github/
?? .gitignore
?? Cargo.lock
?? Cargo.toml
?? README.md
?? deploy/
?? src/
?? tests/

PS> git remote -v
origin	https://github.com/jevallejos/VORTEX-M2B-SPOT.git (fetch)
origin	https://github.com/jevallejos/VORTEX-M2B-SPOT.git (push)
```

Todos los archivos de línea base (`31/31`) fueron verificados en PowerShell con `cr=0` (LF puro) y `bom=False` (UTF-8 sin BOM), y el conjunto completo incluyendo `DOCS/` pasa `cargo test --test red_team_hygiene`.

---

## Sección 6: Riesgos Residuales y Criterios de Promoción (Ventana `--paper` de 30 días en Tokio)

### 6.1 Riesgos Residuales Identificados

| Riesgo | Descripción Técnica | Mitigación Implementada / Operacional |
|---|---|---|
| **Varianza de slippage e impacto de mercado** | `--paper` modela un slippage fijo de `2 bps` (`PAPER_SLIPPAGE_BPS=2`), mientras que órdenes `Market` reales en libros Spot pueden sufrir mayor spread en picos de volatilidad. | Reserva de `30 bps` (`BUY_SLIP_RESERVE_BPS=30`) en el planificador y tope `9970/10000` en `--live-spot` previenen sobregiros de efectivo. |
| **Deriva de reloj del host (`ClockSkew`)** | Ya observada en pruebas locales (`4149 ms` transitorios). Un host con NTP desincronizado abortará ciclos horarios fail-closed. | Exigir `chrony` / `systemd-timesyncd` activo en Tokio y monitorear `vspot health` / alertas en journal. |
| **Mantenimiento / latencia / rate limits Bybit V5** | Cada ciclo horario realiza 1 consulta de tiempo, 5 `instruments-info`, 5 `tickers` y 5 `kline` (`limit=700`). Caídas > 3 reintentos (`MAX_GET_ATTEMPTS=3`) o demoras de fill > `MAX_FILL_POLL_ATTEMPTS=6` abortan el ciclo. | `align_candles_to_common_latest` garantiza coherencia temporal; `orderLinkId` previene duplicación si se reintenta en el siguiente disparo o manualmente. |
| **Cola de drawdown TWR (`> 25%`) y recuperación `HALT_LOCK`** | Una caída `> 25%` TWR activa `HALT_LOCK` permanente hasta intervención manual, impidiendo compras ante rebotes hasta auditoría del operador. | Comportamiento intencional fail-closed; procedimiento documentado en `DOCS/RUNBOOK_TOKIO_PAPER.md` Sección 7.2. |
| **Tracking error y arrastre de comisiones `B2-01` (1h) vs `A0_4H` (4h)** | La frecuencia `1h` puede rotar más que `4h` en mercados laterales, acumulando `10 bps` por lado más slippage. | Libro sombra paralelo `A0_4H` en `state/m2b_shadow_4h_state.json` permite comparación directa bajo exactamente los mismos precios y comisiones durante 30 días. |
| **Polvo residual (`< $5 USDT`)** | Con cuentas pequeñas (`$100`–`$1000`), órdenes calculadas debajo de `min_order_amt = 5.00 USDT` se omiten limpiamente, pudiendo introducir ligera desviación frente al peso teórico. | Escalamiento anti-dust en ventas (`strategy.rs:401-408`) evita quedar atrapado con residuos invendibles. |

### 6.2 Criterios Cuantitativos y Operacionales de Promoción (`--paper` -> `--live-spot`)

La promoción a `--live-spot` **solo** se autorizará si al término de los 30 días continuos en Tokio (`~720` barras `1h` y `~180` cierres `4h` UTC) se cumplen **todos** los umbrales siguientes:

1. **Confiabilidad operacional (`>= 99%`):** al menos `713` de `720` cierres horarios ejecutados sin pérdida de ventana por `ClockSkew` ni bloqueos `run.lock` huérfanos; `0` crashes o panics del binario.
2. **Integridad contable absoluta (`100%`):** todas las entradas de `logs/m2b_audit_ledger.jsonl` verifican `verify_audit_ledger_line == true` y `"drift_1e8": 0`.
3. **Coherencia `B2-01` vs `A0_4H`:** telemetría completa de ambos motores sin desviaciones inexplicadas de señal, verificando que el arrastre de comisiones de `1h` no degrade el retorno ajustado por riesgo respecto a `4h`.
4. **Control de riesgo TWR:** máximo drawdown TWR observado en `--paper` dentro de la envolvente esperada (`<= 25%`) y prueba satisfactoria en frío de `DRAIN_MODE.lock` y recuperación de `HALT_LOCK`.
5. **Alistamiento de seguridad `--live-spot`:** API Key Spot V5 dedicada (`Spot Trade` únicamente, IP whitelist de Tokio, cuenta `UNIFIED`), respaldo previo del estado paper y activación explícita del Double Safety Lock (`BYBIT_SPOT_LIVE_EXECUTION=true` y `BYBIT_SPOT_ACK_LIVE=YES_I_ACCEPT_0X_SPOT_RISK`).
