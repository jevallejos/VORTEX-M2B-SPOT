# VORTEX-M2B-SPOT — Runbook Operacional (`--paper`, Tokio VPS)

**Versión del motor:** `1.1.1` (`vortex-m2b-spot`, `schema_version = 2`)  
**Modo cubierto:** Validación `--paper` de 30 días (cero llamadas privadas de órdenes a Bybit)  
**Universo por defecto (`6` pares):** `BTCUSDT,ETHUSDT,SOLUSDT,LINKUSDT,AVAXUSDT,XAUTUSDT`  
**Directorio objetivo en Tokio:** `/home/ubuntu/vortex-m2b-spot`  
**Repositorio fuente:** `https://github.com/jevallejos/VORTEX-M2B-SPOT.git`

---

## 1. Regla de Aislamiento Estricto (Tokio)

> **CRÍTICO:** Este despliegue es **100% independiente** de cualquier otro servicio en el host Tokio.
> - **PROHIBIDO** leer, modificar, reiniciar o referenciar otros repositorios o unidades `systemd` ajenas a `vortex-m2b-spot`.
> - Toda instalación, configuración, estado y bitácora de `VORTEX-M2B-SPOT` reside exclusivamente en `/home/ubuntu/vortex-m2b-spot` (código fuente clonado p.ej. en `/home/ubuntu/VORTEX-M2B-SPOT` y runtime instalado en `/home/ubuntu/vortex-m2b-spot`).

---

## 2. Pre-requisitos del Host Tokio (Ubuntu)

Verificar antes de instalar:

1. **Toolchain Rust (`>= 1.71`)**:
   ```bash
   rustc --version
   cargo --version
   ```
2. **Sincronización NTP activa** (`chrony` o `systemd-timesyncd`), indispensable para mantener `|local_ms - server_ms| <= MAX_CLOCK_SKEW_MS` (límite por defecto: `2000` ms):
   ```bash
   timedatectl status
   ```
   Confirmar `System clock synchronized: yes` y `NTP service: active`.
3. **Conectividad HTTPS saliente** hacia `https://api.bybit.com` (y `https://api.telegram.org` si se habilitan alertas).

---

## 3. Clonado, Configuración de `.env` (`chmod 600`) y Universo (`M2B_UNIVERSE_SYMBOLS`)

1. Clonar el repositorio fuente en un directorio de trabajo limpio:
   ```bash
   cd /home/ubuntu
   git clone https://github.com/jevallejos/VORTEX-M2B-SPOT.git VORTEX-M2B-SPOT
   cd /home/ubuntu/VORTEX-M2B-SPOT
   ```

2. Preparar `/home/ubuntu/vortex-m2b-spot` y crear `/home/ubuntu/vortex-m2b-spot/.env` desde `.env.example` con permisos `0600` (o dejar que `deploy/install_tokio.sh` lo cree automáticamente si no existe):
   ```bash
   mkdir -p /home/ubuntu/vortex-m2b-spot/{bin,state,logs}
   install -m 0600 /home/ubuntu/VORTEX-M2B-SPOT/.env.example /home/ubuntu/vortex-m2b-spot/.env
   ```

3. Editar `/home/ubuntu/vortex-m2b-spot/.env` y verificar los valores obligatorios para la ventana `--paper`:
   ```dotenv
   BYBIT_BASE_URL=https://api.bybit.com
   BYBIT_API_KEY=
   BYBIT_API_SECRET=
   BYBIT_RECV_WINDOW_MS=5000
   BYBIT_SPOT_LIVE_EXECUTION=false
   BYBIT_SPOT_ACK_LIVE=
   PAPER_INITIAL_CASH_USDT=1000.00
   TELEGRAM_BOT_TOKEN=
   TELEGRAM_CHAT_ID=
   VORTEX_STATE_DIR=state
   VORTEX_LOGS_DIR=logs
   MAX_CLOCK_SKEW_MS=2000
   M2B_UNIVERSE_SYMBOLS=BTCUSDT,ETHUSDT,SOLUSDT,LINKUSDT,AVAXUSDT,XAUTUSDT
   ```
   - **`M2B_UNIVERSE_SYMBOLS` (opcional, Fail-Closed):**
     - Si se omite o queda vacía, `v1.1.1` utiliza exactamente `BTCUSDT,ETHUSDT,SOLUSDT,LINKUSDT,AVAXUSDT,XAUTUSDT` (`6` pares).
     - Si se define, debe listar entre `1` y `20` pares Spot `*USDT` únicos, ASCII alfanuméricos de longitud `6..=20` (se recortan espacios y se normalizan a mayúsculas). Cualquier duplicado, entrada vacía o par no-`USDT` aborta el arranque fail-closed.
   - Para recibir alertas en Telegram durante los 30 días de `--paper`, completar `TELEGRAM_BOT_TOKEN` y `TELEGRAM_CHAT_ID`.
   - Mantener `BYBIT_API_KEY=`, `BYBIT_API_SECRET=`, `BYBIT_SPOT_LIVE_EXECUTION=false` y `BYBIT_SPOT_ACK_LIVE=` vacíos/desactivados durante toda la fase `--paper`.
   - Confirmar permisos estrictos del archivo `.env`:
     ```bash
     ls -la /home/ubuntu/vortex-m2b-spot/.env
     # Esperado: -rw------- 1 ubuntu ubuntu ... .env
     ```

---

## 4. Actualización del Motor y Contrato de Estado (`schema_version = 2`)

### 4.1 Migración Controlada `v1.0.x -> v1.1.x` (`schema_version = 1 -> 2`)

> **REGLA FAIL-CLOSED `schema_version = 2`:** `vortex-m2b-spot` `v1.1.x` **prohíbe** migraciones silenciosas. Si existen `state/primary_state.json` o `state/m2b_shadow_4h_state.json` de `v1.0.x` (`schema_version = 1`) o generados con un orden/conjunto distinto de `M2B_UNIVERSE_SYMBOLS`, el motor abortará explícitamente con `SchemaVersionMismatch` o `UniverseMismatch` sin sobrescribir disco.

Si el host Tokio ejecutaba una instalación heredada `v1.0.1` / `v1.0.2` (`schema_version = 1`), ejecutar este respaldo **antes** de correr `v1.1.x`:

```bash
# 1. Detener el temporizador y confirmar ausencia de ciclo activo
sudo systemctl stop vortex-m2b-spot.timer
ls -la /home/ubuntu/vortex-m2b-spot/state/run.lock 2>/dev/null || echo "OK: no run.lock"

# 2. Respaldar archivos v1.0.x (schema_version = 1) y ledger de auditoría
BACKUP_DIR="/home/ubuntu/vortex-m2b-spot/backups/v1_$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "${BACKUP_DIR}"
mv /home/ubuntu/vortex-m2b-spot/state/primary_state.json "${BACKUP_DIR}/" 2>/dev/null || true
mv /home/ubuntu/vortex-m2b-spot/state/m2b_shadow_4h_state.json "${BACKUP_DIR}/" 2>/dev/null || true
cp -a /home/ubuntu/vortex-m2b-spot/logs/m2b_audit_ledger.jsonl "${BACKUP_DIR}/" 2>/dev/null || true

# 3. Actualizar código fuente a v1.1.x y revisar .env
cd /home/ubuntu/VORTEX-M2B-SPOT
git pull --ff-only origin main
grep -E '^M2B_UNIVERSE_SYMBOLS=' /home/ubuntu/vortex-m2b-spot/.env || echo "M2B_UNIVERSE_SYMBOLS=BTCUSDT,ETHUSDT,SOLUSDT,LINKUSDT,AVAXUSDT,XAUTUSDT" >> /home/ubuntu/vortex-m2b-spot/.env
```

## 4.2 Actualización en Caliente `v1.1.0 -> v1.1.1` (`schema_version = 2` Intacto)

`vortex-m2b-spot` `v1.1.1` **conserva intacto** `STATE_SCHEMA_VERSION = 2` (`src/domain/types.rs:10`) y el esquema JSON completo de `state/primary_state.json`, `state/m2b_shadow_4h_state.json` y `logs/m2b_audit_ledger.jsonl`. La única corrección funcional reside en la eliminación de la rendija `< 2.0%` en `should_rebalance` y `plan_rebalance_orders` (`src/domain/strategy.rs:306-310,404-441`).

> **ACTUALIZACIÓN EN CALIENTE SIN REINICIO NI MIGRACIÓN:** Para hosts Tokio que ya corren `v1.1.0` (`schema_version = 2`), **NO** archivar, borrar ni reiniciar `state/primary_state.json`, `state/m2b_shadow_4h_state.json`, `logs/m2b_audit_ledger.jsonl` ni `.env`. El binario `v1.1.1` carga directamente el estado `v1.1.0` existente preservando `Drift == 0`.

```bash
# 1. Ejecutar fuera de la ventana horaria (:00:15..:01:00 UTC) y confirmar ausencia de run.lock
ls -la /home/ubuntu/vortex-m2b-spot/state/run.lock 2>/dev/null || echo "OK: no run.lock"

# 2. Descargar v1.1.1 e instalar el binario atómicamente en caliente (preservando state/ y .env)
cd /home/ubuntu/VORTEX-M2B-SPOT
git pull --ff-only origin main && bash deploy/install_tokio.sh

# 3. Verificar versión del binario instalado, salud, estado schema_version=2 y timer
/home/ubuntu/vortex-m2b-spot/bin/vortex-m2b-spot --version
# Esperado: vortex-m2b-spot 1.1.1
vspot health
vspot status
vspot timer
```

Tras la actualización, cualquier posición abierta tradable (`>= min_order_amt` y `>= min_order_qty`) previamente atrapada en `(0%, 2.0%)` con `target_w < 0.015` (`LIQUIDATE_TARGET_THRESH`) se liquidará automáticamente al `100%` en el siguiente cierre horario UTC con `Drift == 0`, mientras los residuos de polvo sub-mínimo continuarán omitiéndose silenciosamente sin churning (`0` órdenes).

---

## 5. Instalación Automatizada (`deploy/install_tokio.sh`)

Ejecutar el script de instalación provisto en el repositorio:

```bash
cd /home/ubuntu/VORTEX-M2B-SPOT
bash deploy/install_tokio.sh
```

Qué realiza exactamente `deploy/install_tokio.sh` (`deploy/install_tokio.sh:1-51`):
1. Crea `/home/ubuntu/vortex-m2b-spot/{bin,state,logs}` (`línea 13`).
2. Si `/home/ubuntu/vortex-m2b-spot/.env` no existe, copia `.env.example` con `install -m 0600`; si ya existe, lo preserva (`líneas 15-20`). Advierte si detecta `"schema_version": 1` en `state/primary_state.json` (`líneas 22-25`).
3. Compila el binario en perfil `release` con mínima prioridad CPU/IO (`nice -n 19 ionice -c 3 cargo build --release`, `línea 29`).
4. Instala el binario atómicamente (`vortex-m2b-spot.tmp.$$` -> `mv -f`) en `/home/ubuntu/vortex-m2b-spot/bin/vortex-m2b-spot` (`líneas 38-40`).
5. Instala el helper de solo lectura `deploy/vspot` en `/usr/local/bin/vspot` con permisos `0755` (`línea 43`).
6. Instala `deploy/vortex-m2b-spot.service` y `deploy/vortex-m2b-spot.timer` en `/etc/systemd/system/` con permisos `0644`, ejecuta `systemctl daemon-reload` y habilita `systemctl enable --now vortex-m2b-spot.timer` (`líneas 45-49`).

---

## 6. Verificación Post-Instalación de `systemd` y Conectividad

1. **Verificar configuración del timer `systemd`** (`deploy/vortex-m2b-spot.timer:5-9`):
   - `OnCalendar=*-*-* *:00:15 UTC` (dispara 15 segundos después de cada cierre horario UTC, evitando congestión global de `:00:00..:00:10 UTC`)
   - `RandomizedDelaySec=10` (jitter controlado entre `:00:15` y `:00:25` UTC)
   - `Persistent=true` (recupera disparo pendiente si el host estuvo apagado)
   - `Unit=vortex-m2b-spot.service`

2. **Verificar la unidad de servicio** (`deploy/vortex-m2b-spot.service:6-20`):
   - `Type=oneshot`, `User=ubuntu`, `Group=ubuntu`
   - `WorkingDirectory=/home/ubuntu/vortex-m2b-spot`
   - `EnvironmentFile=-/home/ubuntu/vortex-m2b-spot/.env`
   - `ExecStart=/home/ubuntu/vortex-m2b-spot/bin/vortex-m2b-spot run --paper`
   - `Nice=15`, `IOSchedulingClass=best-effort`, `IOSchedulingPriority=7`
   - `NoNewPrivileges=true`, `PrivateTmp=true`

3. **Ejecutar chequeos inmediatos vía `/usr/local/bin/vspot`**:
   ```bash
   vspot health
   vspot timer
   vspot status
   ```
   - `vspot health` debe imprimir `=== VORTEX-M2B-SPOT HEALTH OK ===`, `Clock Skew (ms) : <N> (limit 2000)`, `State Dir : state (rw)`, `Logs Dir : logs (rw)`, `Universe (6) : BTCUSDT,ETHUSDT,SOLUSDT,LINKUSDT,AVAXUSDT,XAUTUSDT` y las reglas Spot de los **6 pares** (`BTCUSDT`, `ETHUSDT`, `SOLUSDT`, `LINKUSDT`, `AVAXUSDT`, `XAUTUSDT`) con `status=Trading`.
   - `vspot timer` debe mostrar `vortex-m2b-spot.timer` en estado `active (waiting)` con el próximo disparo al minuto `:00:15 UTC` (ventana efectiva `:00:15..:00:25 UTC`).

4. **Ejecución manual inicial (opcional para validar primer ciclo `--paper` y `schema_version = 2`)**:
   ```bash
   cd /home/ubuntu/vortex-m2b-spot
   ./bin/vortex-m2b-spot run --paper
   vspot status
   vspot ledger
   ```
   Confirmar `Schema Version : 2` y `drift_1e8=0` tanto en la salida `CYCLE_EXECUTED` como en `vspot status` para `B2-01` (con los 6 símbolos incluyendo `XAUTUSDT`) y, al cruzar un cierre `4h` UTC (`00/04/08/12/16/20 UTC`), para `A0_4H`.

5. **Endurecimiento operativo conservado (`v1.0.2`, Bybit V5 y ventana de disparo)**:
   - **Parseo en 2 etapas (`src/infrastructure/bybit_spot_v5.rs`):** `parse_bybit_envelope` evalúa `retCode != 0` antes de parsear `result`, devolviendo `InfraError::ExchangeApi` ante errores transitorios con `"result": {}`.
   - **Reintentos acotados `1s/2s/4s` en `GET`:** Las consultas idempotentes `GET` reintentan hasta 3 veces con re-firma HMAC fresca ante errores transitorios; `POST /v5/order/create` nunca se reintenta ciegamente.
   - **Ventana de disparo anti-congestión:** `OnCalendar=*-*-* *:00:15 UTC` con `RandomizedDelaySec=10`.

---

## 7. Monitoreo Diario y Semanal con `/usr/local/bin/vspot`

El script `/usr/local/bin/vspot` (`deploy/vspot:1-79`) provee 5 subcomandos de **solo lectura**:

| Comando | Qué inspecciona | Frecuencia recomendada |
|---|---|---|
| `vspot status` | Universo activo, `Schema Version: 2`, estado de `Primary B2-01 (1h)` y `Shadow A0_4H (4h UTC Paper)` iterando sobre todos los pares configurados, pesos `CUR_W`/`TGT_W`, `SIGMA_ANN`, `TSMOM`, `Drift (1e8)`, `TWR Drawdown` y `DRAIN_MODE.lock` / `HALT_LOCK`. | Diaria |
| `vspot logs` | Últimas 100 líneas del journal de `systemd` (`journalctl -u vortex-m2b-spot.service -n 100 --no-pager`). | Diaria |
| `vspot ledger` | Últimas 20 entradas selladas (FNV-1a 64-bit) de `/home/ubuntu/vortex-m2b-spot/logs/m2b_audit_ledger.jsonl`. | Diaria / Semanal |
| `vspot health` | Conectividad pública a Bybit V5, clock skew (`<= 2000ms`), estado `Trading` de todos los pares configurados (6 por defecto) y permisos `rw` de `state/` y `logs/`. | Semanal o tras incidentes |
| `vspot timer` | Próxima ejecución y estado de `vortex-m2b-spot.timer` (`systemctl list-timers` + `systemctl status`). | Semanal |

### Verificación semanal adicional (integridad y espacio en disco)

```bash
# 1. Confirmar ausencia de archivos temporales huérfanos o run.lock atascado
ls -la /home/ubuntu/vortex-m2b-spot/state /home/ubuntu/vortex-m2b-spot/logs

# 2. Confirmar que el drift contable permanece exactamente en 0 en todo el ledger
grep -E '"drift_1e8":[^0]' /home/ubuntu/vortex-m2b-spot/logs/m2b_audit_ledger.jsonl || echo "OK: all drift_1e8 == 0"

# 3. Confirmar cero errores inesperados en las últimas 24h de journal
journalctl -u vortex-m2b-spot.service --since "24 hours ago" -p err --no-pager
```

---

## 8. Procedimientos de Emergencia (`DRAIN_MODE.lock` y `HALT_LOCK`)

### 8.1 `DRAIN_MODE.lock` (Drenaje ordenado unificado: bloquea nuevas compras, permite ventas de salida)

- **Efecto unificado en `--paper` y `--live-spot`:**
  - `DRAIN_MODE.lock` **no aborta** el ciclo horario en ningún modo.
  - `allow_buys` pasa estrictamente a `false` (`src/application/cycle.rs:309`), y en `--live-spot` una guarda defensiva adicional (`guard_live_order_request`, `src/application/cycle.rs:379-384,938-949`) rechaza cualquier orden `Buy` antes de invocar `SpotExecutionPort::execute_spot_order`. El motor **no abre ni amplía posiciones** (`0` órdenes `Buy`).
  - Las órdenes de salida o reducción (`Sell`) planificadas por caída de señal (`TSMOM` negativo / `LIQUIDATE_TARGET_THRESH`) **sí se ejecutan normalmente** en ambos modos, preservando idempotencia `orderLinkId`, conciliación de wallet (incluyendo `XAUT`) y `Drift == 0`.
  - La salida `CYCLE_EXECUTED` (`src/main.rs:78-89`) y las alertas Telegram (`src/application/cycle.rs:480-489`) reportan explícitamente `drain_mode=true/false`.

**Activar `DRAIN_MODE.lock`:**
```bash
echo "ts_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ) reason=manual_drain" > /home/ubuntu/vortex-m2b-spot/state/DRAIN_MODE.lock
vspot status
```

**Desactivar `DRAIN_MODE.lock`:**
```bash
rm -f /home/ubuntu/vortex-m2b-spot/state/DRAIN_MODE.lock
vspot status
```

### 8.2 `HALT_LOCK` (Parada dura por Drawdown TWR `> 25%`, discrepancia de conciliación o intervención manual)

- **Cuándo se activa automáticamente:**
  1. Drawdown TWR estrictamente superior al `25%` (`dd_1e10 > 2_500_000_000`, `src/domain/accounting.rs:8,609`), escribiendo `state/HALT_LOCK` y evento `HARD_DRAWDOWN_HALT` (`src/application/cycle.rs:516-559`).
  2. Discrepancia de saldo base `> base_precision` en conciliación `--live-spot` (`src/application/cycle.rs:254-288`), escribiendo `state/HALT_LOCK` y evento `RECONCILIATION_MISMATCH`.
- **Efecto en `--paper`:** `allow_buys = false`, `ledger.is_halted() = true`; no hay nuevas compras.
- **Efecto en `--live-spot`:** Rechazo inmediato fail-closed (`AppError::HaltLockActive`, `src/application/cycle.rs:91-93`).

**Activar manualmente `HALT_LOCK`:**
```bash
echo "ts_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ) reason=manual_operator_halt" > /home/ubuntu/vortex-m2b-spot/state/HALT_LOCK
```

**Investigación y limpieza tras un `HALT_LOCK`:**
1. Inspeccionar la causa registrada:
   ```bash
   cat /home/ubuntu/vortex-m2b-spot/state/HALT_LOCK
   vspot status
   vspot ledger
   ```
2. Corregir la causa raíz (revisar eventos `HARD_DRAWDOWN_HALT` o `RECONCILIATION_MISMATCH` en `logs/m2b_audit_ledger.jsonl`).
3. Solo tras autorización explícita del operador, remover `HALT_LOCK` (si `portfolio.halted` quedó `true` dentro de `state/primary_state.json`, auditar/reiniciar el libro antes de reanudar compras):
   ```bash
   rm -f /home/ubuntu/vortex-m2b-spot/state/HALT_LOCK
   ```

### 8.3 Detener temporalmente el timer `systemd`

```bash
sudo systemctl stop vortex-m2b-spot.timer
sudo systemctl disable vortex-m2b-spot.timer
```

---

## 9. Checklist de Auditoría de 30 Días (`--paper` -> Pre-requisito `--live-spot`)

Antes de considerar la promoción desde `--paper` hacia `--live-spot`, verificar **todos** los siguientes puntos al cumplirse los 30 días (~720 ciclos horarios `1h` y ~180 cierres `4h` UTC):

- [ ] **Continuidad de ciclos (`B2-01`)**: `>= 99%` de las 720 horas UTC ejecutadas puntualmente (`*:00:15..25 UTC`), sin fallos repetidos de reloj (`ClockSkew`) ni `run.lock` colgados.
- [ ] **Telemetría paralela (`A0_4H`)**: `180` cierres `4h` UTC (`00/04/08/12/16/20 UTC`) registrados en `state/m2b_shadow_4h_state.json` (`schema_version = 2`) y `logs/m2b_audit_ledger.jsonl` con `SHADOW_4H_CYCLE`.
- [ ] **Invariante contable**: `100%` de las líneas en `logs/m2b_audit_ledger.jsonl` con `"drift_1e8":0` y sello `fnv1a64_hex` válido.
- [ ] **Sin `HALT_LOCK` no resueltos**: Ningún evento inesperado de `HARD_DRAWDOWN_HALT`; drawdown observado `<= 25%` TWR.
- [ ] **Comparativa `B2-01` (1h) vs `A0_4H` (4h) con `XAUTUSDT`**: Evaluar TWR final, máximo drawdown TWR y arrastre de comisiones (`cum_sell_fees_usdt_1e8` + `cum_buy_fees_usdt_est_1e8`), especialmente la contribución de rotación de `XAUTUSDT`.
- [ ] **Checklist de promoción a `--live-spot` (cuando el operador apruebe)**:
  1. Configurar API Key Spot V5 de Bybit con permisos estrictos de **Spot Trade** (sin retiros, IP whitelisted al host Tokio).
  2. Confirmar cuenta Bybit en modo `UNIFIED` con capital inicial USDT depositado.
  3. Activar el **Double Safety Lock** en `.env`:
     - `BYBIT_SPOT_LIVE_EXECUTION=true`
     - `BYBIT_SPOT_ACK_LIVE=YES_I_ACCEPT_0X_SPOT_RISK`
  4. Actualizar `ExecStart` del servicio `systemd` de `--paper` a `--live-spot` (previo respaldo del estado paper y arranque limpio del estado live desde `/v5/account/wallet-balance`).
