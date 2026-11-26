# VORTEX-M2B-SPOT — Reporte de Arquitectura y Verificación de Fase 2 (`v1.1.0`)

**Proyecto:** `VORTEX-M2B-SPOT` (`v1.1.0`)  
**Alcance:** Universo Spot `*USDT` Dinámico Configurable (`M2B_UNIVERSE_SYMBOLS`) + Incorporación por defecto de `XAUTUSDT` + Persistencia Fail-Closed `schema_version = 2`  
**Estrategia primaria:** `B2-01` (`1h` Spot `0x` TSMOM Vol-Targeted, `LONG_FLAT`)  
**Telemetría paralela:** `A0_4H` (`4h` UTC Shadow Book en `--paper`)  
**Venue objetivo:** Bybit Spot V5 (`UNIFIED`, sin apalancamiento, sin cortos)  
**Línea base previa:** `fbfdb38` (`v1.0.2`, conservando endurecimientos de `51ea185` y `fbfdb38`)  
**Repositorio origen:** `https://github.com/jevallejos/VORTEX-M2B-SPOT.git`

---

## Sección 1: Veredicto Ejecutivo e Inventario Exacto

### 1.1 Veredicto Ejecutivo

**VEREDICTO:** **APROBADO PARA DESPLIEGUE `v1.1.0` (`--paper`, 6 pares con `XAUTUSDT`) EN TOKIO.**

La actualización `v1.1.0` de `vortex-m2b-spot` migra el universo estático de 5 símbolos (`[T; 5]`) a un universo Spot `*USDT` dinámico configurable (`1 <= N <= 20`, por defecto `BTCUSDT,ETHUSDT,SOLUSDT,LINKUSDT,AVAXUSDT,XAUTUSDT`) con contrato de estado persistente `schema_version = 2` Fail-Closed, manteniendo intacta toda la matemática certificada de `B2-01` / `A0_4H`, el invariante contable entero **`Drift == 0`** y todas las guardas de seguridad de `v1.0.1` (`51ea185`) y `v1.0.2` (`fbfdb38`):

1. **Invariancia matemática certificada (`569` días en Bybit Spot, `6` pares `5P + XAUTUSDT`):** `src/domain/indicators.rs` permanece sin cambios; `src/domain/strategy.rs` conserva fijo el numerador $\sigma_{\text{target}} / 4 = 0.125$ (sin dividir por $N$), el piso anualizado $\sigma_{\text{vol\_floor}} = 0.15$, el tope por activo $W_{\max} = 0.40$ y el tope bruto $L_{\max} = 1.00$ con reescalado proporcional.
2. **Aislamiento Clean DDD puro (`std`):** `src/domain/` tiene **0 crates externos** y **0 usos** de `unsafe`, `.unwrap()`, `.expect()`, `panic!()`, `todo!()` o `unimplemented!()`.
3. **Contrato Fail-Closed de universo y `schema_version = 2`:** Tanto el arranque (`Universe::parse_csv` / `AssetSymbol::new`) como el cargador de `primary_state.json` y `m2b_shadow_4h_state.json` (`FsStateStore`) abortan explícitamente ante esquemas `v1` heredados, diferencias en símbolos u orden respecto a `M2B_UNIVERSE_SYMBOLS`, o longitudes vectoriales corruptas. Prohibida toda migración silenciosa.
4. **Cobertura de verificación `100 %` aprobada (`54/54` pruebas):** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets` (`28` unitarias en `src/lib.rs`, `11` E2E en `tests/application_e2e_mocks.rs`, `10` de física en `tests/domain_physics.rs`, `5` de red-team hygiene en `tests/red_team_hygiene.rs`) y `cargo build --release` finalizaron con código `0`.

---

### 1.2 Inventario Exacto de Archivos Modificados y Creados (`v1.1.0`)

Medición directa sobre disco tras formateo (`cargo fmt -- --config newline_style=Unix`) y normalización LF (`cr=0`, `bom=False`):

| Ruta Exacta | Acción | Capa Arquitectónica | Líneas | Bytes | Delta Git (`vs fbfdb38`) |
|---|---|---|---:|---:|---|
| `Cargo.toml` | Modificado | `Root/CI` | 33 | 682 | `+1 / -1` (`version = "1.1.0"`) |
| `Cargo.lock` | Actualizado | `Root/CI` | 777 | 20,119 | `+1 / -1` (`version = "1.1.0"`) |
| `.env.example` | Modificado | `Root/CI` | 13 | 358 | `+1 / -0` (`M2B_UNIVERSE_SYMBOLS`) |
| `README.md` | Modificado | `Docs` | 38 | 3,467 | `+11 / -11` |
| `deploy/install_tokio.sh` | Modificado | `Deploy` | 51 | 2,146 | `+5 / -0` (aviso `schema_version=1`) |
| `src/main.rs` | Modificado | `Infrastructure` | 363 | 14,081 | `+50 / -23` |
| `src/domain/error.rs` | Modificado | `Domain` | 238 | 7,585 | `+84 / -2` |
| `src/domain/types.rs` | Modificado | `Domain` | 884 | 24,820 | `+284 / -68` |
| `src/domain/ports.rs` | Modificado | `Domain` | 57 | 2,150 | `+19 / -12` |
| `src/domain/accounting.rs` | Modificado | `Domain` | 799 | 27,991 | `+160 / -73` |
| `src/domain/strategy.rs` | Modificado | `Domain` | 711 | 24,860 | `+129 / -81` |
| `src/application/reconcile.rs` | Modificado | `Application` | 150 | 5,338 | `+63 / -31` |
| `src/application/cycle.rs` | Modificado | `Application` | 1,336 | 48,742 | `+239 / -101` |
| `src/infrastructure/config.rs` | Modificado | `Infrastructure` | 336 | 11,611 | `+49 / -1` |
| `src/infrastructure/bybit_spot_v5.rs` | Modificado | `Infrastructure` | 1,436 | 48,571 | `+130 / -80` |
| `src/infrastructure/state_store.rs` | Modificado | `Infrastructure` | 870 | 29,653 | `+263 / -87` |
| `tests/domain_physics.rs` | Modificado | `Tests` | 901 | 32,351 | `+272 / -75` |
| `tests/application_e2e_mocks.rs` | Modificado | `Tests` | 1,760 | 60,247 | `+719 / -136` |
| `tests/red_team_hygiene.rs` | Modificado | `Tests` | 306 | 9,112 | `+46 / -4` |
| `DOCS/PHASE_2_V110_DYNAMIC_UNIVERSE_XAUT_REPORT.md` | Creado | `Docs` | — | — | Nuevo reporte forense Fase 2 |
| `DOCS/RUNBOOK_TOKIO_PAPER.md` | Modificado | `Docs` | — | — | Actualizado `v1.1.0` + migración `v1 -> v2` |
| `DOCS/INDEX.md` | Modificado | `Docs` | — | — | Índice actualizado a `v1.1.0` |

> **Archivos constitucionalmente preservados sin modificaciones:** `src/domain/indicators.rs` (`269` líneas, `8,148` bytes), `deploy/vortex-m2b-spot.timer` (`12` líneas, `231` bytes), `deploy/vortex-m2b-spot.service`, `deploy/vspot`, `src/infrastructure/telegram.rs`, `.github/workflows/ci.yml`, `.gitattributes`, `.gitignore`, `DOCS/PHASE_1_ARCHITECTURE_AND_VERIFICATION_REPORT.md`.

---

## Sección 2: Auditoría Matemática y Física (`B2-01` / `A0_4H` + Régimen `XAUTUSDT`)

### 2.1 Prueba de Invariancia de Ecuaciones (`sigma_target / 4 = 0.125` Fijo)

La transición de $5$ activos fijos (`5P`) a un universo configurable $1 \le N \le 20$ (`6` pares por defecto con `XAUTUSDT`) conserva **exactamente** todas las constantes y ecuaciones certificadas en la ablación de `569` días en Bybit Spot (`1h` Sharpe `+1.41`, MaxDD `10.23%`, Calmar `+1.98`; `4h` Sharpe `+1.48`, MaxDD `8.81%`, Calmar `+2.45`).

En particular, en `compute_vol_targeted_weights` (`src/domain/strategy.rs:161-207`), el presupuesto base por activo mantiene fijo el numerador $\sigma_{\text{target}} / 4.0 = 0.125$ (`src/domain/strategy.rs:191`) **sin dividir por $N$**:

```rust
let denom = sigma_ann[i].max(VOL_FLOOR_ANN);
// Frozen B2-01/A0_4H scaling: keep numerator (SIGMA_TARGET / 4.0) = 0.125 independent of N.
raw.push(((SIGMA_TARGET / 4.0) / denom * tsmom[i]).clamp(0.0, W_MAX));
```

Esta invariancia asegura que el tamaño asignado a `BTCUSDT..AVAXUSDT` no se diluya artificialmente al incorporar `XAUTUSDT` (de $N=5$ a $N=6$), delegando únicamente en el tope agregado $L_{\max} = 1.00$ el reescalado proporcional cuando $\sum_{j=0}^{N-1} w_j^{\text{raw}} > 1.00$.

---

### 2.2 Tabla de Trazabilidad Matemática (`Fórmula LaTeX -> archivo:línea`)

| Componente Matemático | Fórmula Certificada | Implementación Exacta (`archivo:línea`) |
|---|---|---|
| **Constante GK y piso $\sigma_{\text{bar}}$** | $c_{\text{GK}} = 2\ln(2) - 1, \quad \sigma_{\text{floor}} = 10^{-4}$ | `src/domain/indicators.rs:4-5` |
| **Varianza GK causal por barra** | $v_t = \max\!\left(0,\; \frac{1}{2}\ln^2\!\left(\frac{H_t}{L_t}\right) - c_{\text{GK}}\ln^2\!\left(\frac{C_t}{O_t}\right)\right)$ | `src/domain/indicators.rs:53-55` |
| **EWMA y anualización (`1h` / `4h`)** | $\alpha = \frac{2}{N_{\text{eff}} + 1},\; \sigma_{\text{bar},t} = \max(\sqrt{\tilde{\sigma}_t^2}, 10^{-4}),\; \sigma_{\text{ann},t} = \sigma_{\text{bar},t}\sqrt{B_{\text{day}} \cdot 365}$ (`84`/`24.0`, `21`/`6.0`) | `src/domain/indicators.rs:9-15,43-45,62-70` |
| **TSMOM multi-horizonte** | $z_{t,k} = \frac{\ln(C_t / C_{t-k})}{\sigma_{\text{bar},t}\sqrt{k}},\; S_t = \sum_{(k,w_k)} w_k \tanh(z_{t,k}/\varphi),\; \varphi = 1.618033988749895$ | `src/domain/indicators.rs:7,11,15,145-160` |
| **Peso *Vol-Targeted* `LONG_FLAT`** | $w_i^{\text{raw}} = \operatorname{clamp}\!\left(\frac{0.125}{\max(\sigma_{\text{ann},i},\; 0.15)} \cdot S_i,\; 0,\; 0.40\right)$ | `src/domain/strategy.rs:12-15,189-191` |
| **Tope de exposición bruta Spot `0x`** | $s = \min\!\left(1,\; \frac{L_{\max}}{\sum_{j=0}^{N-1} w_j^{\text{raw}}}\right)$ si $\sum w_j^{\text{raw}} > 0$, $w_i^{\text{target}} = w_i^{\text{raw}} \cdot s$ | `src/domain/strategy.rs:194-206` |
| **Deadband y liquidación total** | $|w_i^{\text{target}} - w_i^{\text{cur}}| \ge 0.08 \;\lor\; (w_i^{\text{target}} < 0.015 \land w_i^{\text{cur}} \ge 0.02)$ | `src/domain/strategy.rs:16-19,308-310,414-441` |
| **Anti-dust en ventas parciales** | Si `rem_qty > 0` y (`rem_val < min_order_amt` $\lor$ `rem_qty < min_order_qty`) $\implies$ liquidación total | `src/domain/strategy.rs:455-462` |
| **Reserva `0.3%` Buy + `SELL_FIRST_BUY_SECOND`** | $\text{budget} = \left\lfloor \frac{(\text{cash}_0 + \sum \text{net\_sell}) \cdot (10000 - 30)}{10000} \right\rfloor$ | `src/domain/strategy.rs:20,488-525` |
| **Conservación en libros (`Drift == 0`)** | $\left(C_{\text{USDT}} + \sum_{i=0}^{N-1} B_i\right) - \left(D_{\text{net}} + P_{\text{realized, gross}} - F_{\text{sell, USDT}}\right) = 0$ | `src/domain/accounting.rs:314-335` |
| **Conservación MTM (`Drift == 0`)** | $E(p) - \left(D_{\text{net}} + (P_{\text{realized, gross}} - F_{\text{sell, USDT}}) + U(p)\right) = 0$ | `src/domain/accounting.rs:337-355` |
| **TWR y Hard Drawdown Halt (`> 25%`)** | $\text{DD}_{1\text{e}10} = \left\lfloor \frac{(\text{TWR}_{\text{peak}} - \text{TWR}_{\text{index}})\cdot 10^{10}}{\text{TWR}_{\text{peak}}} \right\rfloor > 2\,500\,000\,000$ | `src/domain/accounting.rs:7,299-312,592-620` |

---

### 2.3 Comportamiento en Baja Volatilidad de `XAUTUSDT` (`p10 = 11.8%`, `p50 = 19.4%`)

A diferencia de los 5 criptoactivos nativos (`BTC..AVAX`), `XAUTUSDT` (oro tokenizado) presenta regímenes prolongados de baja volatilidad anualizada (percentil 10 observado `p10 = 11.8%`, mediana `p50 = 19.4%`). El diseño congelado protege al portafolio contra sobre-asignación por volatilidad ultra baja mediante la acción conjunta del piso `VOL_FLOOR_ANN = 0.15`, el tope por activo `W_MAX = 0.40` y el tope agregado `L_MAX = 1.00`:

1. **Régimen `p10` (`sigma_ann = 12.0% < 15.0%`):**
   - El denominador se fija en $\max(0.12, 0.15) = 0.15$.
   - Para una señal moderada $S_{\text{XAUT}} = 0.30$:
     $$w_{\text{XAUT}}^{\text{raw}} = \frac{0.125}{0.15} \times 0.30 = 0.2500$$
     (en ausencia de piso `0.15`, $0.125 / 0.12 \times 0.30 = 0.3125$; a `p10 = 11.8%`, sin piso habría subido a $0.3178$).
   - Para una señal fuerte $S_{\text{XAUT}} = 0.72$:
     $$\frac{0.125}{0.15} \times 0.72 = 0.6000 \xrightarrow{\text{clamp}[0, 0.40]} w_{\text{XAUT}}^{\text{raw}} = 0.4000$$
2. **Régimen `p50` (`sigma_ann = 19.4% > 15.0%`):**
   - El piso no restringe el denominador ($\max(0.194, 0.15) = 0.194$).
   - Para $S_{\text{XAUT}} = 0.40$: $w_{\text{XAUT}}^{\text{raw}} = (0.125 / 0.194) \times 0.40 \approx 0.25773196$.
   - Para $S_{\text{XAUT}} = 0.80$: $(0.125 / 0.194) \times 0.80 \approx 0.51546392 \to W_{\max} = 0.4000$.
3. **Reescalado proporcional `5P + XAUTUSDT` bajo `L_max = 1.00`:**
   - Verificado en cálculo cerrado (`tests/domain_physics.rs:xaut_low_volatility_regime_floor_wmax_and_lmax_rescaling_closed_form`) con $\sigma_{\text{ann}} = [0.25, 0.30, 0.40, 0.50, 0.25, 0.12]$ y $S = [0.90, 0.80, 0.80, 0.90, 0.60, 0.72]$:
     - $w^{\text{raw}} = [0.40,\; 0.333\overline{3},\; 0.25,\; 0.225,\; 0.30,\; 0.40]$, $\sum w_j^{\text{raw}} = 1.9083333\overline{3} > 1.00$.
     - Factor de escala $s = 1.00 / 1.9083333\overline{3} \approx 0.52401747$.
     - Pesos finales $w_i^{\text{target}} = w_i^{\text{raw}} \cdot s$ suman exactamente `1.00`, asignando a `XAUTUSDT` $w_5^{\text{target}} \approx 0.20960699$.

---

## Sección 3: Arquitectura Clean DDD y Contrato `schema_version = 2`

### 3.1 Pureza de Dominio y Tipos Dinámicos

- **`src/domain/` (`types.rs`, `error.rs`, `ports.rs`, `indicators.rs`, `strategy.rs`, `accounting.rs`):** 100 % `std` puro sin dependencias externas (`tests/red_team_hygiene.rs:domain_has_zero_external_crate_imports` = PASS).
- **Reemplazo de `Symbol` / `UNIVERSE_LEN = 5`:**
  - `SymbolId(usize)` (`src/domain/types.rs:20-35`): índice denso O(1) dentro de `Universe` para arreglos contables y de señales.
  - `AssetSymbol(String)` (`src/domain/types.rs:37-84`): wrapper validado en construcción (`len ∈ [6, 20]`, ASCII alfanumérico en mayúsculas, sufijo estricto `USDT` y distinto de `"USDT"`), con extractor `base_coin()` (`"XAUTUSDT" -> "XAUT"`).
  - `Universe` (`src/domain/types.rs:86-170`): valida `1 <= N <= 20` y unicidad estricta; construye el defecto `BTCUSDT,ETHUSDT,SOLUSDT,LINKUSDT,AVAXUSDT,XAUTUSDT` (`DEFAULT_UNIVERSE_SYMBOLS`, `src/domain/types.rs:15-18`) o parsea `M2B_UNIVERSE_SYMBOLS` vía `Universe::parse_csv` (recorte de espacios + normalización a mayúsculas, rechazando entradas vacías y duplicados).
  - **`make_order_link_id` (`src/domain/types.rs:483-504`):** Preserva el formato literal `m2b_{closed_hour_ts_ms}_{SYMBOL}_{buy|sell}` cuando su longitud es `<= 36` caracteres (ej. `m2b_1728000000000_XAUTUSDT_buy` = `30` caracteres), y usa un hash FNV-1a compacto de 8 hex + `b`/`s` (`<= 28` caracteres) únicamente para símbolos largos válidos de `14..=20` caracteres, cumpliendo siempre el límite de `36` caracteres de Bybit V5.

---

### 3.2 Contrato JSON `schema_version = 2` y Matriz Fail-Closed

Tanto `state/primary_state.json` como `state/m2b_shadow_4h_state.json` persisten exactamente:

- `"schema_version": 2` (`STATE_SCHEMA_VERSION`, `src/domain/types.rs:10`)
- `"symbols": ["BTCUSDT", "ETHUSDT", "SOLUSDT", "LINKUSDT", "AVAXUSDT", "XAUTUSDT"]`
- Todos los campos contables y de telemetría existentes en `v1.0.2` (`mode`, `last_processed_hour_ts_ms` / `last_processed_4h_close_ts_ms`, `portfolio.{cash_1e8, positions, total_deposits_1e8, realized_pnl_gross_1e8, cum_sell_fees_usdt_1e8, cum_buy_fees_base_1e8, cum_buy_fees_usdt_est_1e8, twr_index_1e10, twr_peak_1e10, last_marked_equity_1e8, halted}`, `last_target_weights`, `last_current_weights`, `last_sigma_ann`, `last_tsmom`, `last_prices_1e8`, `last_drift_1e8`), donde cada vector por activo tiene longitud estrictamente igual a `symbols.len()`.

**Matriz de Validación Fail-Closed (`config.rs`, `types.rs`, `state_store.rs`, `cycle.rs`):**

| Condición Inválida / Desacuerdo | Punto de Guardia (`archivo:línea`) | Error Tipado Devuelto | Comportamiento |
|---|---|---|---|
| `M2B_UNIVERSE_SYMBOLS` no definida o vacía | `src/infrastructure/config.rs:180-187` | — | Carga el defecto de 6 pares (`BTC..AVAX,XAUTUSDT`) |
| Lista CSV con tokens vacíos (`"BTCUSDT,"`), no-`USDT`, `"USDT"`, longitud `<6` o `>20`, caracteres no ASCII-alfanuméricos, duplicados o `>20` pares | `src/domain/types.rs:41-67,92-142`; `src/infrastructure/config.rs:181-183` | `DomainError::InvalidUniverse` / `InfraError::Config` | Aborta el arranque antes de red o disco |
| Archivo de estado en disco con `schema_version != 2` (ej. `v1.0.2` sin `"symbols"`) | `src/infrastructure/state_store.rs:159,199,286-301` (`StateSchemaHeader` previo al DTO completo) | `InfraError::SchemaVersionMismatch { expected: 2, actual, path }` | **Aborta sin modificar disco**; prohíbe migración silenciosa |
| `symbols` del archivo difiere en contenido u orden respecto al universo configurado | `src/infrastructure/state_store.rs:325-332`; `src/domain/types.rs:802-807` | `InfraError::UniverseMismatch` / `DomainError::UniverseMismatch` | Aborta con lista esperada vs actual |
| Cualquier vector por activo (`positions`, `cum_buy_fees_base_1e8`, `last_target_weights`, `last_current_weights`, `last_sigma_ann`, `last_tsmom`, `last_prices_1e8`) con `len != N` | `src/infrastructure/state_store.rs:334-354`; `src/domain/types.rs:809-818`; `src/domain/accounting.rs:58-71` | `InfraError::CorruptStateLength` / `DomainError::CorruptStateLength` | Aborta antes de construir `SpotPortfolioLedger` |

### 3.3 Conciliación `UNIFIED` (`XAUT`) y Salida CLI Dinámica

- **Extracción de `base_coin` (`src/infrastructure/bybit_spot_v5.rs:84-141`):** `parse_wallet_balance_for_universe` y `BybitSpotV5Client::fetch_wallet_snapshot` mapean cada moneda de `/v5/account/wallet-balance?accountType=UNIFIED` comparando `c.coin == sym.base_coin()` (`XAUTUSDT -> XAUT`, `BTCUSDT -> BTC`, etc.) y reconciliando vía `reconcile_live_wallet` (`src/application/reconcile.rs:34-101`). Cualquier discrepancia en `XAUT` superior a `base_precision` activa `HALT_LOCK` y `AppError::ReconciliationMismatch`.
- **CLI `status` y `health` (`src/main.rs:98-317`):** Imprimen `Universe (N) : <csv>`, iteran dinámicamente sobre `cfg.universe.iter_with_ids()` (mostrando tabla por símbolo tanto para `Primary B2-01` como para `Shadow A0_4H` y validando todos los instrumentos en `health`).

---

## Sección 4: Preservación de Regresiones (`51ea185` + `fbfdb38`) e Integridad `Drift == 0`

1. **Regresión `51ea185` (`DRAIN_MODE.lock` unificado):**
   - `src/application/cycle.rs:309`: `allow_buys = !drain_mode && !halt_lock && !ledger.is_halted();` — `DRAIN_MODE.lock` bloquea compras nuevas tanto en `--paper` como en `--live-spot` sin abortar el ciclo.
   - `src/application/cycle.rs:379-384,938-949`: `guard_live_order_request` rechaza toda orden `OrderSide::Buy` si `drain_mode || halt_lock || ledger_halted`, mientras permite ejecutar ventas de salida (`OrderSide::Sell`) cuando `TSMOM` cae.
   - `src/main.rs:78-89` y `src/application/cycle.rs:480-489`: reportan `drain_mode=true/false` en `CYCLE_EXECUTED` y alertas Telegram.
2. **Regresión `fbfdb38` (Bybit Spot V5 API & Timer `systemd`):**
   - **Parseo en 2 etapas (`src/infrastructure/bybit_spot_v5.rs:933-960`):** `parse_bybit_envelope` deserializa `RawBybitEnvelope` y evalúa `ret_code != 0` **antes** de deserializar `result` (evitando falsos errores de parseo ante `"result": {}` en `retCode=10016`/`10006`).
   - **Reintentos GET con firma HMAC fresca (`src/infrastructure/bybit_spot_v5.rs:219-249,911-931`):** `get_private_envelope` regenera `ts_ms` y `sign_bybit_v5` dentro del closure de `retry_idempotent_get` (`backoff 1s -> 2s -> 4s`).
   - **Cero reintentos ciegos en `POST /v5/order/create` (`src/infrastructure/bybit_spot_v5.rs:273-305,777-801`):** `post_private_once` ejecuta un único envío `POST`; si ocurre timeout/5xx consulta `query_order_by_link_id`, nunca reenvía ciegamente. Verificado end-to-end contra socket loopback HTTP real en `tests/application_e2e_mocks.rs:bybit_v5_v102_regression_transient_get_retries_and_retcode_before_result`.
   - **Timer `systemd` (`deploy/vortex-m2b-spot.timer:6-7`):** Conserva exactamente `OnCalendar=*-*-* *:00:15 UTC` y `RandomizedDelaySec=10` (asegurado también por `tests/red_team_hygiene.rs:v110_release_metadata_and_v102_regression_guards`).
3. **Invariante entero `Drift == 0` (`1 <= N <= 20`):**
   - Verificado para `N = 1`, `N = 6` (`5P + XAUTUSDT`) y `N = 20` en `SpotPortfolioLedger` (`src/domain/accounting.rs:766-798` y `tests/domain_physics.rs:universe_validation_fail_closed_and_variable_n_accounting`), manteniendo asimetría Spot V5 (`Buy` en base truncado a `base_step`; `Sell` en USDT) y `drift_book_1e8 == drift_mtm_1e8 == 0`.

---

## Sección 5: Evidencia Bruta de Verificación

Salidas literales observadas en PowerShell dentro de `D:\Desarrollo\VORTEX-M2B-SPOT`:

### 5.1 Compuertas `fmt`, `clippy`, `test --all-targets` (`54/54`) y `build --release`

```text
FMT_EXIT=0
    Checking vortex-m2b-spot v1.1.0 (D:\Desarrollo\VORTEX-M2B-SPOT)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.98s
CLIPPY_EXIT=0
   Compiling vortex-m2b-spot v1.1.0 (D:\Desarrollo\VORTEX-M2B-SPOT)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 2.85s
     Running unittests src\lib.rs (target\debug\deps\vortex_m2b_spot-492b093662a0d1ff.exe)

running 28 tests
test application::cycle::tests::defensive_buy_guard_blocks_buys_under_drain_or_halt_and_permits_sells ... ok
test application::cycle::tests::live_spot_double_safety_lock_denies_without_ack ... ok
test application::cycle::tests::live_spot_halt_lock_rejects_cycle_fail_closed ... ok
test application::reconcile::tests::reconcile_detects_deposit_and_base_mismatch ... ok
test domain::accounting::tests::buy_sell_fee_asymmetry_and_zero_drift ... ok
test domain::accounting::tests::hard_drawdown_halt_triggers_above_25_percent ... ok
test domain::accounting::tests::deposit_and_withdrawal_preserve_twr_and_drift ... ok
test domain::accounting::tests::variable_n_1_6_20_zero_drift_and_length_guards ... ok
test domain::indicators::tests::gk_ewma_rejects_invalid_inputs ... ok
test domain::indicators::tests::gk_ewma_respects_floor_and_causal_update ... ok
test domain::indicators::tests::tsmom_multi_horizon_works_and_checks_history ... ok
test domain::strategy::tests::filter_and_aggregate_1h_to_4h_works ... ok
test domain::strategy::tests::plan_rebalance_sell_first_anti_dust_and_buy_reserve ... ok
test domain::strategy::tests::vol_targeted_weights_respect_caps_and_l_max ... ok
test infrastructure::bybit_spot_v5::tests::parse_envelope_returns_exchange_api_error_when_result_is_empty_object ... ok
test infrastructure::bybit_spot_v5::tests::parse_envelope_succeeds_on_ret_code_zero_and_valid_result ... ok
test infrastructure::bybit_spot_v5::tests::parse_envelope_ret_code_zero_malformed_result_includes_bounded_sanitized_prefix ... ok
test infrastructure::bybit_spot_v5::tests::retry_idempotent_get_uses_1s_2s_4s_backoff_and_fails_fast_on_non_retryable ... ok
test infrastructure::bybit_spot_v5::tests::test_paper_noop_execution_port_errors ... ok
test infrastructure::bybit_spot_v5::tests::parse_wallet_balance_extracts_xaut_for_default_universe ... ok
test infrastructure::bybit_spot_v5::tests::retryable_error_classifier_covers_transient_codes_and_rejects_fatal_errors ... ok
test infrastructure::bybit_spot_v5::tests::test_sign_bybit_v5_deterministic ... ok
test infrastructure::config::tests::test_dotenv_parser_and_token_fallback ... ok
test infrastructure::config::tests::test_universe_env_parsing_and_fail_closed_validation ... ok
test domain::strategy::tests::signals_b2_01_and_a0_4h_smoke ... ok
test application::cycle::tests::paper_cycle_executes_idempotently_and_updates_shadow_4h ... ok
test infrastructure::state_store::tests::test_audit_seal_and_verify ... ok
test infrastructure::state_store::tests::test_schema_v2_fail_closed_rejects_v1_mismatch_and_corrupt_len ... ok

test result: ok. 28 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running unittests src\main.rs (target\debug\deps\vortex_m2b_spot-9209a70e1a12b747.exe)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests\application_e2e_mocks.rs (target\debug\deps\application_e2e_mocks-b0ee1e8c85f8a490.exe)

running 11 tests
test bybit_v5_hmac_sha256_known_test_vectors ... ok
test live_spot_double_safety_lock_and_lockfile_rejections ... ok
test config_env_universe_parsing_defaults_and_fail_closed ... ok
test multi_hour_crossing_4h_utc_boundary_and_idempotency ... ok
test telegram_notifier_failure_does_not_abort_cycle_or_persistence ... ok
test fs_state_store_atomic_persistence_audit_seal_and_run_lock_contention ... ok
test drain_mode_in_live_spot_blocks_new_buys_and_executes_exit_sells ... ok
test e2e_six_pair_universe_with_xaut_4h_boundary_live_reconciliation_and_order_link_id ... ok
test drain_mode_in_paper_blocks_new_buys_and_allows_exit_sells ... ok
test state_store_schema_v2_fail_closed_rejects_v1_universe_mismatch_order_and_corrupt_lengths ... ok
test bybit_v5_v102_regression_transient_get_retries_and_retcode_before_result ... ok

test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.01s

     Running tests\domain_physics.rs (target\debug\deps\domain_physics-12415b3bbb4e2508.exe)

running 10 tests
test garman_klass_ewma_closed_form_floor_nan_and_annualization ... ok
test hard_drawdown_halt_strictly_above_25_percent ... ok
test spot_v5_asymmetric_fee_accounting_and_zero_drift ... ok
test anti_dust_escalation_and_below_5_usdt_buy_skipped ... ok
test external_deposit_plus_50_usdt_is_neutral_to_twr_and_trading_pnl ... ok
test strict_sell_first_buy_second_with_zero_initial_cash_and_reserve ... ok
test tsmom_1h_72_168_504_closed_form_and_1h_to_4h_aggregator ... ok
test xaut_low_volatility_regime_floor_wmax_and_lmax_rescaling_closed_form ... ok
test universe_validation_fail_closed_and_variable_n_accounting ... ok
test zero_lookahead_b2_01_bit_identical_after_future_perturbation ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running tests\red_team_hygiene.rs (target\debug\deps\red_team_hygiene-2d250652e419b2ad.exe)

running 5 tests
test v110_release_metadata_and_v102_regression_guards ... ok
test src_has_zero_panic_unwrap_expect_todo_unimplemented_or_unsafe ... ok
test domain_has_zero_external_crate_imports ... ok
test gitignore_env_example_and_secret_hygiene ... ok
test governed_files_are_utf8_no_bom_pure_lf_and_zero_banned_strings ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s

TEST_EXIT=0
   Compiling vortex-m2b-spot v1.1.0 (D:\Desarrollo\VORTEX-M2B-SPOT)
    Finished `release` profile [optimized] target(s) in 55.83s
BUILD_EXIT=0
```

### 5.2 Auditoría UTF-8 sin BOM / LF Puro, Binario Release y Estado Git

```text
--- GIT ---
 M .env.example
 M Cargo.lock
 M Cargo.toml
 M README.md
 M deploy/install_tokio.sh
 M src/application/cycle.rs
 M src/application/reconcile.rs
 M src/domain/accounting.rs
 M src/domain/error.rs
 M src/domain/ports.rs
 M src/domain/strategy.rs
 M src/domain/types.rs
 M src/infrastructure/bybit_spot_v5.rs
 M src/infrastructure/config.rs
 M src/infrastructure/state_store.rs
 M src/main.rs
 M tests/application_e2e_mocks.rs
 M tests/domain_physics.rs
 M tests/red_team_hygiene.rs
 .env.example                        |   1 +
 Cargo.lock                          |   2 +-
 Cargo.toml                          |   2 +-
 README.md                           |  22 +-
 deploy/install_tokio.sh             |   5 +
 src/application/cycle.rs            | 340 +++++++++-----
 src/application/reconcile.rs        |  94 ++--
 src/domain/accounting.rs            | 233 +++++++---
 src/domain/error.rs                 |  86 +++-
 src/domain/ports.rs                 |  31 +-
 src/domain/strategy.rs              | 210 +++++----
 src/domain/types.rs                 | 352 ++++++++++++---
 src/infrastructure/bybit_spot_v5.rs | 210 +++++----
 src/infrastructure/config.rs        |  50 ++-
 src/infrastructure/state_store.rs   | 350 +++++++++++----
 src/main.rs                         |  73 ++-
 tests/application_e2e_mocks.rs      | 855 ++++++++++++++++++++++++++++++------
 tests/domain_physics.rs             | 347 +++++++++++----
 tests/red_team_hygiene.rs           |  50 ++-
 19 files changed, 2538 insertions(+), 775 deletions(-)
fbfdb38 fix(infra): parse Bybit V5 retCode before result and retry transient GET errors
51ea185 fix(live-spot): allow orderly exit sells while blocking buys under DRAIN_MODE.lock
f60a5d4 feat: initialize VORTEX-M2B-SPOT v1.0.0 1h Spot 0x engine and 4h shadow
--- BIN ---
vortex-m2b-spot 1.1.0
```

Todos los archivos gobernados del repositorio registran `cr=0` (LF puro) y `bom=False` (UTF-8 sin BOM), validado además automáticamente por `tests/red_team_hygiene.rs` tras la generación de los documentos en `DOCS/`.

---

## Sección 6: Riesgos Residuales y Handoff Operacional a Tokio

### 6.1 Riesgos Residuales de `v1.1.0` (`5P + XAUTUSDT`)

| Riesgo | Detalle Cuantitativo / Operacional | Mitigación / Seguimiento |
|---|---|---|
| **Rotación `XAUTUSDT` en `1h` (`B2-01`) vs `4h` (`A0_4H`)** | La vela `1h` de `XAUTUSDT` puede cruzar el umbral de deadband (`0.08`) con mayor frecuencia que `4h` durante consolidaciones del oro, generando arrastre adicional de comisiones (`10 bps` por lado). | Mantener libro sombra `A0_4H` en `state/m2b_shadow_4h_state.json` para auditar la diferencia de TWR neto y arrastre (`cum_sell_fees_usdt_1e8` + `cum_buy_fees_usdt_est_1e8`) durante la ventana `--paper`. |
| **Compresión de volatilidad del oro (`p10 = 11.8% < 0.15`)** | En periodos muy calmos de `XAUTUSDT`, el piso `VOL_FLOOR_ANN = 0.15` evita sobre-apalancamiento teórico de riesgo, pero mantiene hasta `W_max = 0.40` cuando el TSMOM es elevado. | Comportamiento verificado y esperado por la ablación de `569` días (`1h` MaxDD `10.23%`, `4h` MaxDD `8.81%`). El cortacircuito `TWR DD > 25%` permanece activo. |
| **Incremento de consultas HTTP por ciclo (`5 -> 6` pares)** | Cada ciclo consulta `1` `/v5/market/time`, `6` `instruments-info`, `6` `tickers` y `6` `kline` (`19` GET públicos por hora). | Sigue muy por debajo de los límites de Bybit V5 y cuenta con backoff `1s/2s/4s` en GET transitorios (`fbfdb38`). |
| **Ruptura intencional de estado `v1.0.2` (`schema_version = 1`)** | Un host Tokio que ya posea `state/primary_state.json` de `v1.0.2` fallará cerrado al arrancar `v1.1.0` con `SchemaVersionMismatch (expected v2, found v1)`. | Procedimiento obligatorio de respaldo y arranque limpio documentado en la Sección 6.2 y en `DOCS/RUNBOOK_TOKIO_PAPER.md`. |

### 6.2 Procedimiento de Handoff en Tokio (Respaldo `v1.0.2` previo a arranque `v1.1.0`)

Ejecutar en el host Tokio **antes** de reactivar `vortex-m2b-spot.timer` tras actualizar a `v1.1.0`:

1. Detener temporalmente el temporizador y confirmar que no hay ejecución activa (`run.lock` ausente):
   ```bash
   sudo systemctl stop vortex-m2b-spot.timer
   ls -la /home/ubuntu/vortex-m2b-spot/state
   ```
2. Respaldar íntegro el estado `schema_version = 1` y el ledger de auditoría previo (`v1.0.2`):
   ```bash
   BACKUP_TS="$(date -u +%Y%m%dT%H%M%SZ)"
   mkdir -p "/home/ubuntu/vortex-m2b-spot/backups/v1_${BACKUP_TS}"
   mv /home/ubuntu/vortex-m2b-spot/state/primary_state.json "/home/ubuntu/vortex-m2b-spot/backups/v1_${BACKUP_TS}/" 2>/dev/null || true
   mv /home/ubuntu/vortex-m2b-spot/state/m2b_shadow_4h_state.json "/home/ubuntu/vortex-m2b-spot/backups/v1_${BACKUP_TS}/" 2>/dev/null || true
   cp -a /home/ubuntu/vortex-m2b-spot/logs/m2b_audit_ledger.jsonl "/home/ubuntu/vortex-m2b-spot/backups/v1_${BACKUP_TS}/" 2>/dev/null || true
   ```
3. Actualizar `.env` si se desea fijar explícitamente `M2B_UNIVERSE_SYMBOLS=BTCUSDT,ETHUSDT,SOLUSDT,LINKUSDT,AVAXUSDT,XAUTUSDT` (opcional: si se omite, `v1.1.0` usa exactamente esos 6 pares por defecto).
4. Compilar e instalar `v1.1.0` con `bash deploy/install_tokio.sh`.
5. Verificar conectividad y estado `Trading` de los `6` pares (`BTCUSDT..AVAXUSDT,XAUTUSDT`) con `vspot health`, ejecutar `./bin/vortex-m2b-spot run --paper` (opcional para inicializar `schema_version = 2`), comprobar `vspot status` y reactivar `sudo systemctl start vortex-m2b-spot.timer`.
