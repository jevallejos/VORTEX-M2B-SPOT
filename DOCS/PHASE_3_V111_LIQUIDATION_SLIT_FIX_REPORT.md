# VORTEX-M2B-SPOT — Reporte de Arquitectura y Verificación de Fase 3 (`v1.1.1`)

**Proyecto:** `VORTEX-M2B-SPOT` (`v1.1.1`)  
**Alcance:** Eliminación de la rendija de liquidación sub-`2.0%` (`LIQUIDATE_CURRENT_THRESH = 0.02`) en `should_rebalance` y `plan_rebalance_orders` + Cobertura determinista (`$1,060.00`) + Compatibilidad en caliente (`schema_version = 2`)  
**Estrategia primaria:** `B2-01` (`1h` Spot `0x` TSMOM Vol-Targeted, `LONG_FLAT`)  
**Telemetría paralela:** `A0_4H` (`4h` UTC Shadow Book en `--paper`)  
**Venue objetivo:** Bybit Spot V5 (`UNIFIED`, sin apalancamiento, sin cortos)  
**Línea base previa:** `e20cabd` (`v1.1.0`, conservando endurecimientos de `51ea185`, `fbfdb38` y `e20cabd`)  
**Repositorio origen:** `https://github.com/jevallejos/VORTEX-M2B-SPOT.git`

---

## Sección 1: Veredicto Ejecutivo e Inventario Exacto

### 1.1 Veredicto Ejecutivo

**VEREDICTO:** **APROBADO PARA DESPLIEGUE EN CALIENTE `v1.1.1` (`--paper`, 6 pares con `XAUTUSDT`, `schema_version = 2` intacto) EN TOKIO.**

La actualización `v1.1.1` de `vortex-m2b-spot` corrige una rendija geométrica de liquidación heredada en `src/domain/strategy.rs` donde la guarda `current_w >= LIQUIDATE_CURRENT_THRESH` (`0.02`) impedía liquidar posiciones abiertas y plenamente operables en Bybit Spot V5 dentro del intervalo `[0.4717%, 2.0%)` (sobre un portafolio de `$1,060.00` con `min_order_amt = $5.00`) cuando `target_w < 0.015` (`LIQUIDATE_TARGET_THRESH`) y la diferencia de pesos quedaba por debajo del `DEADBAND = 0.08`:

1. **Eliminación completa de la rendija `< 2.0%` (`src/domain/strategy.rs:306-310,404-441`):** Se suprime la constante `LIQUIDATE_CURRENT_THRESH = 0.02`. `should_rebalance(target_w, current_w)` liquida cualquier peso positivo (`current_w > 0.0`) cuando `target_w < LIQUIDATE_TARGET_THRESH` (`0.015`), y `plan_rebalance_orders` refuerza el disparo por cantidad base (`raw_target_w < LIQUIDATE_TARGET_THRESH && pos_qty.0 > 0`) aun si el peso flotante reportado fuera `0.0`.
2. **Silencio contable garantizado sobre polvo sub-mínimo (`0` churning, `Drift == 0`):** La emisión de la orden de liquidación total (`OrderSide::Sell`, `full_liquidation: true`) queda acotada inferiormente únicamente por las reglas del instrumento (`sellable >= rule.min_order_qty` y `Money1e8(notional_1e8) >= rule.min_order_amt`). Todo residuo por debajo de estos mínimos se omite silenciosamente (`0` órdenes, `0` errores, `0` churning).
3. **Compatibilidad en caliente `v1.1.0 -> v1.1.1` (`STATE_SCHEMA_VERSION = 2` intacto):** `src/domain/types.rs:10` conserva sin cambios `schema_version = 2` y el formato JSON completo de `state/primary_state.json` y `state/m2b_shadow_4h_state.json`. Un host Tokio en `v1.1.0` actualiza en caliente a `v1.1.1` sin migración ni reinicio de estado.
4. **Cobertura de verificación `100 %` aprobada (`60/60` pruebas):** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets` (`30` unitarias en `src/lib.rs`, `12` E2E en `tests/application_e2e_mocks.rs`, `13` de física en `tests/domain_physics.rs`, `5` de higiene en `tests/red_team_hygiene.rs`) y `cargo build --release` (`vortex-m2b-spot 1.1.1`) finalizaron con código `0`.

---

### 1.2 Inventario Exacto de Archivos Modificados y Creados (`v1.1.1`)

Medición directa sobre disco (`BOM=False`, `CR=0`, UTF-8 sin BOM y LF puro):

| Ruta Exacta | Acción | Capa Arquitectónica | Líneas | Bytes | Delta Git (`vs e20cabd`) |
|---|---|---|---:|---:|---|
| `Cargo.toml` | Modificado | `Root/CI` | 33 | 682 | `+1 / -1` (`version = "1.1.1"`) |
| `Cargo.lock` | Actualizado | `Root/CI` | 777 | 20,119 | `+1 / -1` (`version = "1.1.1"`) |
| `README.md` | Modificado | `Docs` | 39 | 3,975 | `+3 / -2` (`v1.1.1`, liquidación y link Fase 3) |
| `src/domain/strategy.rs` | Modificado | `Domain` | 856 | 31,001 | `+149 / -4` (eliminación rendija `<2%` + 2 tests unitarios) |
| `tests/domain_physics.rs` | Modificado | `Tests` | 1,247 | 44,857 | `+349 / -3` (`3` tests deterministas `$1,060.00`) |
| `tests/application_e2e_mocks.rs` | Modificado | `Tests` | 1,980 | 68,641 | `+220 / -0` (`sorted_state_key_sets` + carga caliente `v1.1.0 -> v1.1.1`) |
| `tests/red_team_hygiene.rs` | Modificado | `Tests` | 320 | 9,743 | `+17 / -3` (`v111_release_metadata_and_regression_guards`) |
| `DOCS/PHASE_3_V111_LIQUIDATION_SLIT_FIX_REPORT.md` | Creado | `Docs` | — | — | Nuevo reporte forense Fase 3 |
| `DOCS/RUNBOOK_TOKIO_PAPER.md` | Modificado | `Docs` | 292 | 17,854 | Actualizado `v1.1.1` + Sección `4.2` hot upgrade `v1.1.0 -> v1.1.1` |
| `DOCS/INDEX.md` | Modificado | `Docs` | 41 | 6,229 | Índice `v1.1.1` + enlace Fase 3 + conteo `60/60` pruebas |

> **Archivos constitucionalmente preservados sin modificaciones:** `src/domain/indicators.rs` (`269` líneas, `8,148` bytes), `src/domain/types.rs` (`STATE_SCHEMA_VERSION: u32 = 2` en línea `10`), `src/domain/accounting.rs`, `src/domain/error.rs`, `src/domain/ports.rs`, `src/application/cycle.rs`, `src/application/reconcile.rs`, `src/infrastructure/*`, `src/main.rs` (`363` líneas, `14,081` bytes), `src/lib.rs`, `deploy/*` (incluyendo `deploy/vortex-m2b-spot.timer` con `12` líneas / `231` bytes), `.env.example`, `.gitattributes`, `.gitignore`, `.github/workflows/ci.yml`, `DOCS/PHASE_1_ARCHITECTURE_AND_VERIFICATION_REPORT.md` (`436` líneas, `38,244` bytes) y `DOCS/PHASE_2_V110_DYNAMIC_UNIVERSE_XAUT_REPORT.md` (`362` líneas, `30,887` bytes).

---

## Sección 2: Análisis Matemático Antes/Después de la Rendija `< 2.0%`

### 2.1 Diagnóstico de la Rendija (`$1,060.00`, 6 pares con `XAUTUSDT`, `min_order_amt = $5.00`)

En Bybit Spot V5 con `min_order_amt = $5.00` (`500_000_000`) sobre un portafolio de `$1,060.00` (`106_000_000_000`), el peso mínimo ejecutable por nocional es:

$$w_{\min,\text{order}} = \frac{\$5.00}{\$1{,}060.00} = \frac{500\,000\,000}{106\,000\,000\,000} \approx 0.00471698 \quad (0.4717\%)$$

Cualquier posición abierta con peso actual $w_i^{\text{cur}} \in [0.004717,\; 0.020)$ es estrictamente operable en el exchange (`notional >= $5.00`), pero en `v1.1.0` quedaba atrapada cuando la señal caía a $w_i^{\text{target}} < 0.015$ (`LIQUIDATE_TARGET_THRESH`):

1. La condición de *deadband* fallaba porque $|w_i^{\text{target}} - w_i^{\text{cur}}| < 0.02 < 0.08$ (`DEADBAND`).
2. La condición de liquidación de `v1.1.0` (`w_i^{\text{target}} < 0.015 && w_i^{\text{cur}} >= 0.02`) también fallaba porque $w_i^{\text{cur}} < 0.02$.

Este intervalo ciego $[0.4717\%, 2.0\%)$ ocurre orgánicamente en producción en dos situaciones frecuentes:
- **Post-recorte parcial:** Un recorte válido (`|w_target - w_prev| >= 0.08`) lleva el peso hacia `1.6%` (`w_1 ≈ 0.0160`), y en la hora siguiente `TSMOM` cae a `<= 0` (`target_w = 0.0`).
- **Compra acotada por efectivo libre:** Una entrada de `XAUTUSDT` con efectivo remanente limitado (`$12.19`) abre una posición de `~1.15%` (`$12.125`), y al caer `target_w` a `0.0` la posición queda atrapada sin salida.

---

### 2.2 Ecuación Antes (`v1.1.0`) vs Después (`v1.1.1`) y Trazabilidad en Código

- **Antes (`v1.1.0`, con `LIQUIDATE_CURRENT_THRESH = 0.02`):**
  $$\text{should\_rebalance}(w_i^{\text{target}}, w_i^{\text{cur}}) \iff |w_i^{\text{target}} - w_i^{\text{cur}}| \ge 0.08 \;\lor\; \left(w_i^{\text{target}} < 0.015 \;\land\; w_i^{\text{cur}} \ge 0.02\right)$$
- **Después (`v1.1.1`, rendija eliminada):**
  $$\text{should\_rebalance}(w_i^{\text{target}}, w_i^{\text{cur}}) \iff |w_i^{\text{target}} - w_i^{\text{cur}}| \ge 0.08 \;\lor\; \left(w_i^{\text{target}} < 0.015 \;\land\; w_i^{\text{cur}} > 0\right)$$

En `src/domain/strategy.rs`:
- **Constantes preservadas (`src/domain/strategy.rs:12-20`):** `SIGMA_TARGET = 0.50`, `W_MAX = 0.40`, `L_MAX = 1.00`, `VOL_FLOOR_ANN = 0.15`, `DEADBAND_DELTA = 0.20`, `DEADBAND = 0.08`, `LIQUIDATE_TARGET_THRESH = 0.015`, `BUY_SLIP_RESERVE_BPS = 30`. Se elimina `pub const LIQUIDATE_CURRENT_THRESH: f64 = 0.02;`.
- **Predicado `should_rebalance` (`src/domain/strategy.rs:306-310`):**
  ```rust
  #[inline]
  pub fn should_rebalance(target_w: f64, current_w: f64) -> bool {
      (target_w - current_w).abs() >= DEADBAND
          || (target_w < LIQUIDATE_TARGET_THRESH && current_w > 0.0)
  }
  ```
- **Refuerzo en `plan_rebalance_orders` (`src/domain/strategy.rs:404-441`):** Antes de la compuerta se lee `let pos_qty = input.positions[i].qty_1e8;` (`:409`), se calcula `let liquidate_open_pos = raw_target_w < LIQUIDATE_TARGET_THRESH && pos_qty.0 > 0;` (`:410`), y la compuerta evalúa `if !should_rebalance(raw_target_w, current_w) && !liquidate_open_pos { continue; }` (`:411-413`). Cuando `raw_target_w < LIQUIDATE_TARGET_THRESH`, se fuerza `target_w = 0.0` (`:415-419`), se trunca `sellable = pos_qty.floor_to_step(rule.base_precision)?` (`:425`), se calcula `notional_1e8 = mul_div_floor_nonneg(sellable.0, price.0, SCALE_1E8)?` (`:426`) y se emite `OrderSide::Sell` al `100%` (`full_liquidation: true`, `target_weight: 0.0`, `:428-439`) **únicamente** si `sellable >= rule.min_order_qty && Money1e8(notional_1e8) >= rule.min_order_amt` (`:427`). Si no alcanza dichos mínimos del instrumento, retorna sin emitir orden ni error (`0` órdenes, `0` churning).

---

### 2.3 Auditoría Completa de Call Sites (`should_rebalance` y `0.02`)

Búsqueda exhaustiva en todo el árbol `src/` y `tests/`:

| Símbolo / Archivo | Ubicación Exacta | Hallazgo Verificado |
|---|---|---|
| `should_rebalance` (único call site en producción) | `src/domain/strategy.rs:411` | **1 único call site** dentro de `plan_rebalance_orders` (`src/domain/strategy.rs:404-441`); llamadas de test unitario únicamente en `src/domain/strategy.rs:715-723`. |
| `plan_rebalance_orders` (`B2-01` y `A0_4H`) | `src/application/cycle.rs:311` (`B2_01`) y `:607` (`A0_4H`) | Ambos motores (`1h` primario y `4h` sombra) delegan en la misma función corregida de dominio. |
| `src/main.rs` (`cmd_status` y `print_usage`) | `src/main.rs:19` y `:98-240` | **0 llamadas** a `should_rebalance`; `cmd_status` imprime solo columnas de estado (`SYMBOL`, `QTY`, `PRICE`, `CUR_W`, `TGT_W`, `SIGMA_ANN`, `TSMOM`) sin etiquetas de rebalanceo pendiente para polvo sub-mínimo; `print_usage` toma automáticamente `env!("CARGO_PKG_VERSION")` (`1.1.1`). |
| `LIQUIDATE_CURRENT_THRESH` / `0.02` | Todo `src/` | Eliminada por completo; prohibida por regresión en `tests/red_team_hygiene.rs:279-282`. |

---

### 2.4 Vectores Numéricos Deterministas (`$1,060.00`, Escenarios A, B y C + Unitarios)

Todos los escenarios operan con el universo por defecto de `6` pares (`BTCUSDT,ETHUSDT,SOLUSDT,LINKUSDT,AVAXUSDT,XAUTUSDT`), comisión `DEFAULT_SPOT_FEE_RATE = 10 bps` (`10 / 10_000`) y `min_order_amt = $5.00` (`500_000_000`):

1. **Escenario A — Post-recorte `9.6% -> 1.6% -> 0.0%` en `LINKUSDT` (`tests/domain_physics.rs:903-1026`, `post_trim_two_hour_sequence_9_6_to_1_6_to_0_liquidates_fully_zero_drift`):**
   - **Estado inicial:** `LINKUSDT` @ `$10.00` (`1_000_000_000`), `base_precision = 0.001` (`100_000`), `cash = $958.24` (`95_824_000_000`), `10.176 LINK` (`1_017_600_000`, `$101.76`, $w_0 = 0.096$), equity `$1,060.00` (`106_000_000_000`), `Drift == 0`.
   - **Hora 1 (`target_w[3] = 0.016`):** `|0.016 - 0.096| = 0.08 >= 0.08` $\implies$ vende `8.480 LINK` (`848_000_000`, `$84.80` = `8_480_000_000`, `full_liquidation = false`, fee `10 bps = $0.0848` = `8_480_000`). Remanente: `1.696 LINK` (`169_600_000`, `$16.96 >= $5.00`), equity post-Hora 1 = `$1,059.9152` (`105_991_520_000`), peso $w_1 = 1\,696\,000\,000 / 105\,991\,520\,000 \approx 0.01600128 \in (0.015,\; 0.020)$, `Drift == 0`.
   - **Hora 2 (`target_w = [0.0; 6]`):** En `v1.1.0`, `|0.0 - 0.01600128| < 0.08` y `0.01600128 < 0.02` emitía **0 órdenes** (posición atrapada). En `v1.1.1`, `should_rebalance(0.0, 0.01600128) = true` $\implies$ emite `OrderSide::Sell` al `100%` (`full_liquidation = true`, `1.696 LINK` = `169_600_000`, nocional `$16.96` = `1_696_000_000`, fee `$0.01696` = `1_696_000`). Posición final: `qty_1e8 = 0`, `cost_basis_usdt_1e8 = 0`, `drift_book_1e8 == 0`, `drift_mtm_1e8 == 0`.

2. **Escenario B — Compra acotada por efectivo libre `1.15% -> 0.0%` en `XAUTUSDT` (`tests/domain_physics.rs:1028-1168`, `cash_constrained_buy_1_15_pct_then_zero_target_liquidates_fully_zero_drift`):**
   - **Estado inicial:** `BTC..AVAX` @ `$100.00` (`4 * 2.0956 + 2.0957 = 10.4781`) ocupan `$1,047.81` (`98.85%` de `$1,060.00`), dejando `cash = $12.19` (`1_219_000_000`); `XAUTUSDT` @ `$2,500.00` (`250_000_000_000`), `base_precision = 0.00001` (`1_000`).
   - **Hora 1 (`target_h1 = [0.17, 0.17, 0.17, 0.17, 0.17, 0.15]`):** Los 5 pares existentes permanecen dentro del deadband (`|0.17 - 0.1977| < 0.08`). `XAUTUSDT` ejecuta compra con presupuesto reservado `floor(1_219_000_000 * 9_970 / 10_000) = 1_215_343_000` (`$12.15343 >= $5.00`), `gross_base = 486_137`, `fee_base = 486`, posición neta truncada `0.00485 XAUT` (`485_000`, MTM `$12.125`), equity post-Hora 1 = `$1,059.97157` (`105_997_157_000`), peso $w_{\text{post}} = 1\,212\,500\,000 / 105\,997\,157\,000 \approx 0.01143899 \in (0.00471698,\; 0.020)$, `Drift == 0`.
   - **Hora 2 (`target_h2[5] = 0.0`):** En `v1.1.0` quedaba atrapada (`0.01143899 < 0.02`). En `v1.1.1` emite `OrderSide::Sell` al `100%` (`full_liquidation = true`, `0.00485 XAUT` = `485_000`, nocional `$12.125` = `1_212_500_000`). Posición final: `qty_1e8 = 0`, `cost_basis_usdt_1e8 = 0`, `drift_book_1e8 == 0`, `drift_mtm_1e8 == 0`.

3. **Escenario C — Polvo sub-mínimo `$0.06` / sub-step `$0.05` sin churning en 5 ciclos (`tests/domain_physics.rs:1170-1247`, `sub_minimum_dust_0_06_usdt_zero_churn_across_consecutive_cycles_zero_drift`):**
   - **Estado inicial:** `0.003 AVAX` (`300_000`) @ `$20.00` = `$0.06 < $5.00 min_order_amt` y `0.0005 SOL` (`50_000`) @ `$100.00` = `$0.05` (`< 0.001 base_precision`), `cash = $1,059.89` (`105_989_000_000`), equity `$1,060.00`.
   - **Ejecución (`5` ciclos consecutivos con `target_w = [0.0; 6]`):** `should_rebalance` retorna `true` y entra al bloque de liquidación, pero `sellable < min_order_qty` (`SOLUSDT`) y `notional_1e8 < min_order_amt` (`AVAXUSDT`) filtran silenciosamente la emisión: **`0` órdenes, `0` errores, `0` churning**, posiciones sin cambios (`300_000` y `50_000`), `drift_book_1e8 == 0`, `drift_mtm_1e8 == 0`.

4. **Pruebas unitarias directas de dominio (`src/domain/strategy.rs:712-855`):**
   - `should_rebalance_liquidates_any_positive_weight_below_target_thresh` (`:712-725`): verifica `!should_rebalance(0.0, 0.0)`, `!should_rebalance(0.0149, 0.0)`, `should_rebalance(0.0, 0.0001)`, `should_rebalance(0.0, 0.0115)`, `should_rebalance(0.0, 0.016)`, `should_rebalance(0.0149, 0.016)`, `!should_rebalance(0.015, 0.016)`, `should_rebalance(0.016, 0.096)` y `!should_rebalance(0.05, 0.01)`.
   - `plan_rebalance_liquidates_sub_2pct_tradable_and_skips_dust_without_churn` (`:727-855`): verifica sobre `$1,060.00` (`$10.00` por par, `min_order_amt = $5.00`) la liquidación simultánea de `BTCUSDT` (`$16.96`, `1.60%`, `target_w = 0.0`), `ETHUSDT` (`$12.15`, `~1.15%`, `target_w = 0.014`) y `LINKUSDT` (`$10.00` con `current_w = 0.0` pero `qty_1e8 = 100_000_000 > 0`), omitiendo `SOLUSDT` (`$0.06`) tanto en el ciclo conjunto como en ciclo aislado de polvo (`0` órdenes).

---

## Sección 3: Compatibilidad en Caliente `v1.1.0 -> v1.1.1` (`schema_version = 2` Intacto) y Preservación de Regresiones

### 3.1 Prueba End-to-End de Carga en Caliente `v1.1.0 -> v1.1.1`

El ensayo `hot_load_v110_schema_v2_state_runs_1h_and_4h_and_liquidates_sub_2pct_positions` junto con el helper `sorted_state_key_sets` (`tests/application_e2e_mocks.rs:1762-1980`) demuestra compatibilidad binaria/JSON estricta sin migración ni reinicio:

1. **Carga directa de estado literal `v1.1.0` (`schema_version = 2`):**
   - `primary_state.json`: `schema_version = 2`, 6 pares por defecto, `last_processed_hour_ts_ms = 1730170800000` (`BASE_UTC_MIDNIGHT_MS + 603 * ONE_HOUR_MS`), `cash_1e8 = 103_079_268_000`, `LINKUSDT` = `0.10000000` (`10_000_000` @ `$169.81`, peso `1.601981% ≈ 1.602%`) y `XAUTUSDT` = `0.07200000` (`7_200_000` @ `$169.81`, peso `1.153426% ≈ 1.153%`), equity `$1,060.00`, `last_drift_1e8 = 0`.
   - `m2b_shadow_4h_state.json`: `schema_version = 2`, 6 pares por defecto, `last_processed_4h_close_ts_ms = 1730160000000` (`BASE_UTC_MIDNIGHT_MS + 600 * ONE_HOUR_MS`), `cash_1e8 = 104_777_368_000`, `XAUTUSDT` = `0.07200000` (`1.153426% ≈ 1.153%`), `last_drift_1e8 = 0`.
2. **Ciclo `1h` + frontera `4h` UTC (`604h` UTC close):**
   - `B2_01` liquida las 2 posiciones atrapadas (`orders_executed = 2`: `LINKUSDT` y `XAUTUSDT` pasan a `Qty1e8::ZERO`), con `drift_1e8 = 0`, `shadow_4h_updated = true`, `halted = false`, `drain_mode = false`.
   - `A0_4H` actualiza `last_processed_4h_close_ts_ms` a `604h` UTC close y liquida `XAUTUSDT` (`Qty1e8::ZERO`) con `last_drift_1e8 = 0`.
   - Los conjuntos ordenados de claves JSON (`root` + `portfolio`) medidos por `sorted_state_key_sets` son **idénticos** antes y después de ejecutar `v1.1.1` tanto para `primary_state.json` como para `m2b_shadow_4h_state.json`. Todas las líneas del ledger (`logs/m2b_audit_ledger.jsonl`) pasan `verify_audit_ledger_line` (sello FNV-1a 64-bit válido) y registran `"drift_1e8":0`.
3. **Ciclo subsecuente (`605h` UTC close):** Ya en plano (`target_weights = [0.0; 6]`), ejecuta **`0` órdenes** (`exec.total_calls() == 0`), `shadow_4h_updated = false` y `drift_1e8 = 0`.

### 3.2 Preservación de Regresiones (`51ea185`, `fbfdb38`, `e20cabd`)

Verificada por construcción y por `tests/red_team_hygiene.rs:241-293` (`v111_release_metadata_and_regression_guards`):
- **`51ea185`:** `DRAIN_MODE.lock` unificado en `--paper` y `--live-spot` (bloquea `Buy`, permite `Sell` de salida).
- **`fbfdb38`:** Parseo en 2 etapas de Bybit V5 (`retCode` previo a `result`), reintentos GET `1s/2s/4s` con HMAC fresco, cero reintentos ciegos POST y timer `systemd` con `OnCalendar=*-*-* *:00:15 UTC` y `RandomizedDelaySec=10`.
- **`e20cabd`:** Universo dinámico `*USDT` (`1..=20`, 6 pares por defecto con `XAUTUSDT`), `STATE_SCHEMA_VERSION = 2` Fail-Closed, numerador fijo $\sigma_{\text{target}} / 4 = 0.125$, y ausencia total de la constante heredada de universo fijo en `src/`.

---

## Sección 4: Evidencia Bruta Literal (`fmt`, `clippy`, `test --all-targets`, `build --release`, UTF-8/LF y `git diff --stat`)

Salida literal observada en PowerShell (`D:\Desarrollo\VORTEX-M2B-SPOT`) tras completar los cambios de código y pruebas de `v1.1.1` (`60/60` pruebas PASS, binario `vortex-m2b-spot 1.1.1`), inmediatamente previa a la actualización documental final en `DOCS/` (verificada posteriormente en cierre con `BOM=False`, `CR=0` y compuerta `cargo`):

```text
FMT_EXIT=0
    Checking vortex-m2b-spot v1.1.1 (D:\Desarrollo\VORTEX-M2B-SPOT)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.58s
CLIPPY_EXIT=0
   Compiling vortex-m2b-spot v1.1.1 (D:\Desarrollo\VORTEX-M2B-SPOT)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 16.41s
     Running unittests src\lib.rs (target\debug\deps\vortex_m2b_spot-a1dbb76c2a209cf1.exe)

running 30 tests
test application::cycle::tests::defensive_buy_guard_blocks_buys_under_drain_or_halt_and_permits_sells ... ok
test domain::accounting::tests::buy_sell_fee_asymmetry_and_zero_drift ... ok
test application::cycle::tests::live_spot_double_safety_lock_denies_without_ack ... ok
test application::reconcile::tests::reconcile_detects_deposit_and_base_mismatch ... ok
test domain::accounting::tests::deposit_and_withdrawal_preserve_twr_and_drift ... ok
test domain::accounting::tests::hard_drawdown_halt_triggers_above_25_percent ... ok
test application::cycle::tests::live_spot_halt_lock_rejects_cycle_fail_closed ... ok
test domain::accounting::tests::variable_n_1_6_20_zero_drift_and_length_guards ... ok
test domain::indicators::tests::gk_ewma_rejects_invalid_inputs ... ok
test domain::indicators::tests::gk_ewma_respects_floor_and_causal_update ... ok
test domain::indicators::tests::tsmom_multi_horizon_works_and_checks_history ... ok
test domain::strategy::tests::filter_and_aggregate_1h_to_4h_works ... ok
test domain::strategy::tests::plan_rebalance_liquidates_sub_2pct_tradable_and_skips_dust_without_churn ... ok
test domain::strategy::tests::plan_rebalance_sell_first_anti_dust_and_buy_reserve ... ok
test domain::strategy::tests::should_rebalance_liquidates_any_positive_weight_below_target_thresh ... ok
test domain::strategy::tests::vol_targeted_weights_respect_caps_and_l_max ... ok
test infrastructure::bybit_spot_v5::tests::retry_idempotent_get_uses_1s_2s_4s_backoff_and_fails_fast_on_non_retryable ... ok
test infrastructure::bybit_spot_v5::tests::parse_envelope_returns_exchange_api_error_when_result_is_empty_object ... ok
test infrastructure::bybit_spot_v5::tests::parse_envelope_succeeds_on_ret_code_zero_and_valid_result ... ok
test infrastructure::bybit_spot_v5::tests::retryable_error_classifier_covers_transient_codes_and_rejects_fatal_errors ... ok
test infrastructure::bybit_spot_v5::tests::parse_wallet_balance_extracts_xaut_for_default_universe ... ok
test infrastructure::bybit_spot_v5::tests::parse_envelope_ret_code_zero_malformed_result_includes_bounded_sanitized_prefix ... ok
test infrastructure::bybit_spot_v5::tests::test_paper_noop_execution_port_errors ... ok
test infrastructure::bybit_spot_v5::tests::test_sign_bybit_v5_deterministic ... ok
test infrastructure::config::tests::test_universe_env_parsing_and_fail_closed_validation ... ok
test domain::strategy::tests::signals_b2_01_and_a0_4h_smoke ... ok
test infrastructure::config::tests::test_dotenv_parser_and_token_fallback ... ok
test application::cycle::tests::paper_cycle_executes_idempotently_and_updates_shadow_4h ... ok
test infrastructure::state_store::tests::test_audit_seal_and_verify ... ok
test infrastructure::state_store::tests::test_schema_v2_fail_closed_rejects_v1_mismatch_and_corrupt_len ... ok

test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running unittests src\main.rs (target\debug\deps\vortex_m2b_spot-c9c7a41f3f024415.exe)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests\application_e2e_mocks.rs (target\debug\deps\application_e2e_mocks-f37ace3342991c87.exe)

running 12 tests
test bybit_v5_hmac_sha256_known_test_vectors ... ok
test config_env_universe_parsing_defaults_and_fail_closed ... ok
test live_spot_double_safety_lock_and_lockfile_rejections ... ok
test multi_hour_crossing_4h_utc_boundary_and_idempotency ... ok
test drain_mode_in_live_spot_blocks_new_buys_and_executes_exit_sells ... ok
test fs_state_store_atomic_persistence_audit_seal_and_run_lock_contention ... ok
test hot_load_v110_schema_v2_state_runs_1h_and_4h_and_liquidates_sub_2pct_positions ... ok
test e2e_six_pair_universe_with_xaut_4h_boundary_live_reconciliation_and_order_link_id ... ok
test telegram_notifier_failure_does_not_abort_cycle_or_persistence ... ok
test drain_mode_in_paper_blocks_new_buys_and_allows_exit_sells ... ok
test state_store_schema_v2_fail_closed_rejects_v1_universe_mismatch_order_and_corrupt_lengths ... ok
test bybit_v5_v102_regression_transient_get_retries_and_retcode_before_result ... ok

test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.01s

     Running tests\domain_physics.rs (target\debug\deps\domain_physics-1b6751818bbe44b6.exe)

running 13 tests
test garman_klass_ewma_closed_form_floor_nan_and_annualization ... ok
test spot_v5_asymmetric_fee_accounting_and_zero_drift ... ok
test hard_drawdown_halt_strictly_above_25_percent ... ok
test external_deposit_plus_50_usdt_is_neutral_to_twr_and_trading_pnl ... ok
test post_trim_two_hour_sequence_9_6_to_1_6_to_0_liquidates_fully_zero_drift ... ok
test anti_dust_escalation_and_below_5_usdt_buy_skipped ... ok
test cash_constrained_buy_1_15_pct_then_zero_target_liquidates_fully_zero_drift ... ok
test strict_sell_first_buy_second_with_zero_initial_cash_and_reserve ... ok
test sub_minimum_dust_0_06_usdt_zero_churn_across_consecutive_cycles_zero_drift ... ok
test xaut_low_volatility_regime_floor_wmax_and_lmax_rescaling_closed_form ... ok
test tsmom_1h_72_168_504_closed_form_and_1h_to_4h_aggregator ... ok
test universe_validation_fail_closed_and_variable_n_accounting ... ok
test zero_lookahead_b2_01_bit_identical_after_future_perturbation ... ok

test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running tests\red_team_hygiene.rs (target\debug\deps\red_team_hygiene-9271a27785ff7044.exe)

running 5 tests
test v111_release_metadata_and_regression_guards ... ok
test src_has_zero_panic_unwrap_expect_todo_unimplemented_or_unsafe ... ok
test domain_has_zero_external_crate_imports ... ok
test gitignore_env_example_and_secret_hygiene ... ok
test governed_files_are_utf8_no_bom_pure_lf_and_zero_banned_strings ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s

TEST_EXIT=0
   Compiling vortex-m2b-spot v1.1.1 (D:\Desarrollo\VORTEX-M2B-SPOT)
    Finished `release` profile [optimized] target(s) in 50.80s
BUILD_EXIT=0
---BIN---
vortex-m2b-spot 1.1.1
---DIFF_NAMES---
Cargo.lock
Cargo.toml
README.md
src/domain/strategy.rs
tests/application_e2e_mocks.rs
tests/domain_physics.rs
tests/red_team_hygiene.rs
---DIFF_STAT---
 Cargo.lock                     |   2 +-
 Cargo.toml                     |   2 +-
 README.md                      |   5 +-
 src/domain/strategy.rs         | 153 +++++++++++++++++-
 tests/application_e2e_mocks.rs | 220 ++++++++++++++++++++++++++
 tests/domain_physics.rs        | 352 ++++++++++++++++++++++++++++++++++++++++-
 tests/red_team_hygiene.rs      |  20 ++-
 7 files changed, 740 insertions(+), 14 deletions(-)
```

Asimismo, `tests/red_team_hygiene.rs:governed_files_are_utf8_no_bom_pure_lf_and_zero_banned_strings` (`:133-177`) verifica automáticamente en todo archivo gobernado (incluyendo `DOCS/`) codificación UTF-8 válida sin BOM (`BOM=False`), fin de línea LF puro (`CR=0`) y ausencia de cadenas heredadas prohibidas.

---

## Sección 5: Handoff de Actualización en Caliente en Tokio (`v1.1.0 -> v1.1.1`)

Para un host Tokio que ya ejecuta `v1.1.0` (`schema_version = 2`), la actualización a `v1.1.1` es **100% en caliente, sin reinicio ni migración de estado** (ver detalle operacional en [`DOCS/RUNBOOK_TOKIO_PAPER.md`](RUNBOOK_TOKIO_PAPER.md) Sección `4.2`):

1. **Confirmar ausencia de ejecución horaria activa** (ejecutar fuera de la ventana `:00:15..:01:00 UTC`):
   ```bash
   ls -la /home/ubuntu/vortex-m2b-spot/state/run.lock 2>/dev/null || echo "OK: no run.lock"
   ```
2. **Conservar intactos los archivos operativos** (`state/primary_state.json`, `state/m2b_shadow_4h_state.json`, `logs/m2b_audit_ledger.jsonl` y `.env`). **No** mover, borrar ni reiniciar el libro.
3. **Actualizar el código fuente e instalar atómicamente el binario `1.1.1`:**
   ```bash
   cd /home/ubuntu/VORTEX-M2B-SPOT
   git pull --ff-only origin main && bash deploy/install_tokio.sh
   ```
   `deploy/install_tokio.sh` compila `--release` (`nice -n 19 ionice -c 3`) e instala atómicamente vía `.tmp.$$ -> mv -f` en `/home/ubuntu/vortex-m2b-spot/bin/vortex-m2b-spot`.
4. **Verificar versión instalada, salud y estado posterior:**
   ```bash
   /home/ubuntu/vortex-m2b-spot/bin/vortex-m2b-spot --version
   # Esperado: vortex-m2b-spot 1.1.1
   vspot health
   vspot status
   vspot timer
   ```
   Confirmar `Schema Version : 2` y `Drift (1e8) : 0` tanto en `Primary B2-01` como en `Shadow A0_4H`. En el siguiente disparo `:00:15 UTC`, cualquier posición abierta tradable sub-`2.0%` cuyo peso objetivo sea `< 0.015` se liquidará automáticamente al `100%` preservando `Drift == 0`.
