# VORTEX-M2B-SPOT — Índice de Documentación

Documentación técnica y operacional de **VORTEX-M2B-SPOT** (`v1.0.0`): motor determinístico `1h` Spot `0x` TSMOM Vol-Targeted (`B2-01`) con libro sombra paralelo `4h` UTC (`A0_4H`) para Bybit Spot V5 (`UNIFIED`, `LONG_FLAT`).

---

## Documentos Principales

| Documento | Propósito |
|---|---|
| [`README.md`](../README.md) | Resumen ejecutivo, parámetros congelados del universo (`B2-01` / `A0_4H`) y comandos CLI (`run --paper`, `run --live-spot`, `status`, `health`). |
| [`DOCS/PHASE_1_ARCHITECTURE_AND_VERIFICATION_REPORT.md`](PHASE_1_ARCHITECTURE_AND_VERIFICATION_REPORT.md) | Reporte forense de Fase 1: inventario exacto de archivos, auditoría matemática (`Fórmula LaTeX -> archivo:línea`), arquitectura Hexagonal Clean DDD, contabilidad `i128` (`Drift == 0`), evidencia bruta de verificación, riesgos residuales y criterios de promoción. |
| [`DOCS/RUNBOOK_TOKIO_PAPER.md`](RUNBOOK_TOKIO_PAPER.md) | Runbook operacional paso a paso para despliegue `--paper` de 30 días en `/home/ubuntu/vortex-m2b-spot` (Tokio VPS), monitoreo vía `vspot` y procedimientos `DRAIN_MODE.lock` / `HALT_LOCK`. |

---

## Artefactos Clave de Despliegue, Configuración y CI

| Archivo | Descripción |
|---|---|
| [`Cargo.toml`](../Cargo.toml) | Manifiesto Rust (`edition = 2021`, `rust-version = 1.71`), lints estrictos (`unsafe_code = "forbid"`, `unwrap_used/expect_used/panic/todo/unimplemented = "deny"`) y perfil `release` (`lto = "fat"`, `panic = "abort"`). |
| [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) | Pipeline CI (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets`, `cargo build --release`). |
| [`.env.example`](../.env.example) | Plantilla de variables de entorno con valores seguros por defecto para `--paper` y placeholders vacíos de credenciales. |
| [`.gitattributes`](../.gitattributes) | Reglas de fin de línea `eol=lf` para código Rust, TOML, Markdown, YAML, scripts shell y `deploy/vspot`. |
| [`deploy/install_tokio.sh`](../deploy/install_tokio.sh) | Instalador idempotente para `/home/ubuntu/vortex-m2b-spot` (`bin/`, `state/`, `logs/`, `.env` `0600`, `vspot`, unidades `systemd`). |
| [`deploy/vortex-m2b-spot.service`](../deploy/vortex-m2b-spot.service) | Servicio `systemd` `Type=oneshot` ejecutando `vortex-m2b-spot run --paper`. |
| [`deploy/vortex-m2b-spot.timer`](../deploy/vortex-m2b-spot.timer) | Temporizador `systemd` horario (`*:00:08 UTC`, `RandomizedDelaySec=4`, `Persistent=true`). |
| [`deploy/vspot`](../deploy/vspot) | CLI operacional de solo lectura (`/usr/local/bin/vspot`: `status`, `logs`, `ledger`, `health`, `timer`). |

---

## Suites de Verificación

| Suite | Alcance |
|---|---|
| [`tests/domain_physics.rs`](../tests/domain_physics.rs) | Ecuaciones cerradas GK-EWMA, TSMOM multi-horizonte, agregador `1h -> 4h`, invariante zero-lookahead bit-idéntico, asimetría de comisiones Spot V5, neutralidad TWR ante depósitos (`+$50`), anti-dust, `SELL_FIRST_BUY_SECOND` y hard DD `> 25%`. |
| [`tests/application_e2e_mocks.rs`](../tests/application_e2e_mocks.rs) | Ciclo horario E2E con mocks deterministas, cruce de frontera `4h` UTC, idempotencia, Double Safety Lock `--live-spot`, `DRAIN_MODE.lock`, resiliencia ante fallos de Telegram, vectores HMAC-SHA256 Bybit V5, persistencia atómica y sello FNV-1a. |
| [`tests/red_team_hygiene.rs`](../tests/red_team_hygiene.rs) | Auditoría automática: cero `unsafe`/`unwrap`/`expect`/`panic`/`todo`/`unimplemented` en `src/`, cero crates externos en `src/domain/`, UTF-8 sin BOM + LF puro en todo archivo gobernado, e higiene de `.gitignore`/`.env.example`/secretos. |
