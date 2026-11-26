# VORTEX-M2B-SPOT — Índice de Documentación

Documentación técnica y operacional de **VORTEX-M2B-SPOT** (`v1.1.1`, `schema_version = 2`): motor determinístico `1h` Spot `0x` TSMOM Vol-Targeted (`B2-01`) con libro sombra paralelo `4h` UTC (`A0_4H`) para Bybit Spot V5 (`UNIFIED`, `LONG_FLAT`) y universo Spot `*USDT` configurable (por defecto `6` pares: `BTCUSDT`, `ETHUSDT`, `SOLUSDT`, `LINKUSDT`, `AVAXUSDT`, `XAUTUSDT`).

---

## Documentos Principales

| Documento | Propósito |
|---|---|
| [`README.md`](../README.md) | Resumen ejecutivo `v1.1.1`, `M2B_UNIVERSE_SYMBOLS`, parámetros congelados (`B2-01` / `A0_4H`), liquidación total sub-`2.0%` sin churning, contrato `schema_version = 2` y comandos CLI (`run --paper`, `run --live-spot`, `status`, `health`). |
| [`DOCS/PHASE_3_V111_LIQUIDATION_SLIT_FIX_REPORT.md`](PHASE_3_V111_LIQUIDATION_SLIT_FIX_REPORT.md) | Reporte forense de Fase 3 (`v1.1.1`): eliminación de la rendija de liquidación (`LIQUIDATE_CURRENT_THRESH = 0.02`), análisis matemático antes/después sobre `$1,060.00`, auditoría de call sites, prueba E2E de compatibilidad en caliente `v1.1.0 -> v1.1.1` (`schema_version = 2` intacto), preservación de regresiones (`51ea185`, `fbfdb38`, `e20cabd`), evidencia verbatim (`60/60` pruebas) y handoff en Tokio. |
| [`DOCS/PHASE_2_V110_DYNAMIC_UNIVERSE_XAUT_REPORT.md`](PHASE_2_V110_DYNAMIC_UNIVERSE_XAUT_REPORT.md) | Reporte forense de Fase 2 (`v1.1.0`): inventario exacto de cambios, prueba de invariancia matemática ($\sigma_{\text{target}} / 4 = 0.125$ fijo) y análisis `XAUTUSDT` bajo piso `0.15`, contrato Fail-Closed `schema_version = 2`, preservación de regresiones (`51ea185` + `fbfdb38`), evidencia verbatim de verificación (`54/54` pruebas) y handoff a Tokio. |
| [`DOCS/PHASE_1_ARCHITECTURE_AND_VERIFICATION_REPORT.md`](PHASE_1_ARCHITECTURE_AND_VERIFICATION_REPORT.md) | Reporte forense histórico de Fase 1 (`v1.0.0`): arquitectura Hexagonal Clean DDD, ecuaciones base y línea base inicial. |
| [`DOCS/RUNBOOK_TOKIO_PAPER.md`](RUNBOOK_TOKIO_PAPER.md) | Runbook operacional paso a paso (`v1.1.1`) para despliegue `--paper` en `/home/ubuntu/vortex-m2b-spot` (Tokio VPS), configuración de `M2B_UNIVERSE_SYMBOLS`, actualización en caliente `v1.1.0 -> v1.1.1` (`schema_version = 2` intacto), migración respaldada heredada `v1.0.x -> v1.1.x` (`schema_version = 1 -> 2`), monitoreo vía `vspot` y procedimientos `DRAIN_MODE.lock` / `HALT_LOCK`. |

---

## Artefactos Clave de Despliegue, Configuración y CI

| Archivo | Descripción |
|---|---|
| [`Cargo.toml`](../Cargo.toml) | Manifiesto Rust `v1.1.1` (`edition = 2021`, `rust-version = 1.71`), lints estrictos (`unsafe_code = "forbid"`, `unwrap_used/expect_used/panic/todo/unimplemented = "deny"`) y perfil `release` (`lto = "fat"`, `panic = "abort"`). |
| [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) | Pipeline CI (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets`, `cargo build --release`). |
| [`.env.example`](../.env.example) | Plantilla de variables de entorno con valores seguros por defecto para `--paper`, `M2B_UNIVERSE_SYMBOLS` (6 pares con `XAUTUSDT`) y placeholders vacíos de credenciales. |
| [`.gitattributes`](../.gitattributes) | Reglas de fin de línea `eol=lf` para código Rust, TOML, Markdown, YAML, scripts shell y `deploy/vspot`. |
| [`deploy/install_tokio.sh`](../deploy/install_tokio.sh) | Instalador idempotente para `/home/ubuntu/vortex-m2b-spot` (`bin/`, `state/`, `logs/`, `.env` `0600`, aviso si detecta `schema_version=1`, `vspot`, unidades `systemd`). |
| [`deploy/vortex-m2b-spot.service`](../deploy/vortex-m2b-spot.service) | Servicio `systemd` `Type=oneshot` ejecutando `vortex-m2b-spot run --paper`. |
| [`deploy/vortex-m2b-spot.timer`](../deploy/vortex-m2b-spot.timer) | Temporizador `systemd` horario (`*:00:15 UTC`, `RandomizedDelaySec=10`, `Persistent=true`). |
| [`deploy/vspot`](../deploy/vspot) | CLI operacional de solo lectura (`/usr/local/bin/vspot`: `status`, `logs`, `ledger`, `health`, `timer`). |

---

## Suites de Verificación (`60/60` pruebas)

| Suite | Alcance |
|---|---|
| [`src/lib.rs`](../src/lib.rs) (pruebas unitarias internas) | Módulos `domain` (`accounting`, `indicators`, `strategy` incluyendo `should_rebalance_liquidates_any_positive_weight_below_target_thresh` y `plan_rebalance_liquidates_sub_2pct_tradable_and_skips_dust_without_churn`), `application` (`cycle`, `reconcile`) e `infrastructure` (`bybit_spot_v5`, `config`, `state_store`) (`30` tests). |
| [`tests/domain_physics.rs`](../tests/domain_physics.rs) | Ecuaciones cerradas GK-EWMA, TSMOM multi-horizonte, agregador `1h -> 4h`, invariante zero-lookahead bit-idéntico (6 pares), régimen baja volatilidad `XAUTUSDT` (`12%` bajo piso `0.15` y `19.4%` + rescalado `L_max=1.00`), validación Fail-Closed de universo (`1..=20`), contabilidad `Drift == 0` para `N=1,6,20`, asimetría Spot V5, neutralidad TWR (`+$50`), anti-dust, `SELL_FIRST_BUY_SECOND`, hard DD `> 25%` y secuencias `$1,060.00` de liquidación sub-`2.0%` (`9.6% -> 1.6% -> 0%`, compra acotada `1.15% -> 0%`, y silencio sin churning en polvo `$0.06` / `$0.05`) (`13` tests). |
| [`tests/application_e2e_mocks.rs`](../tests/application_e2e_mocks.rs) | Ciclo horario E2E `6` pares (`BTC..AVAX,XAUTUSDT`) con mocks deterministas, cruce `4h` UTC, conciliación `XAUT` y `orderLinkId`, carga en caliente `v1.1.0 -> v1.1.1` (`schema_version = 2`) con liquidación sub-`2.0%` en `1h`/`4h`, guardas Fail-Closed `schema_version = 2`, regresión `v1.0.2` HTTP loopback real (`retCode` antes de `result`, reintento GET con HMAC fresco y cero reintentos POST), `DRAIN_MODE.lock`, Double Safety Lock `--live-spot`, Telegram y `M2B_UNIVERSE_SYMBOLS` (`12` tests). |
| [`tests/red_team_hygiene.rs`](../tests/red_team_hygiene.rs) | Auditoría automática: cero `unsafe`/`unwrap`/`expect`/`panic`/`todo`/`unimplemented` en `src/`, cero crates externos en `src/domain/`, UTF-8 sin BOM + LF puro en todo archivo gobernado, higiene de `.gitignore`/`.env.example`/secretos, metadatos `v1.1.1`, ausencia de `LIQUIDATE_CURRENT_THRESH`, preservación de `LIQUIDATE_TARGET_THRESH = 0.015` y guardas de regresión `51ea185`/`fbfdb38`/`e20cabd` (`5` tests). |
