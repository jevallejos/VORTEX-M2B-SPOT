# VORTEX-M2B-SPOT — Runbook Operacional (`--paper`, Tokio VPS)

**Versión del motor:** `1.0.0` (`vortex-m2b-spot`)  
**Modo cubierto:** Validación `--paper` de 30 días (cero llamadas privadas de órdenes a Bybit)  
**Directorio objetivo en Tokio:** `/home/ubuntu/vortex-m2b-spot`  
**Repositorio fuente:** `https://github.com/jevallejos/VORTEX-M2B-SPOT.git`

---

## 1. Regla de Aislamiento Estricto (Tokio)

> **CRÍTICO:** Este despliegue es **100% independiente** de cualquier otro servicio en el host Tokio.
> - **PROHIBIDO** leer, modificar, reiniciar o referenciar `/home/ubuntu/vortex-rs` o sus unidades `systemd`.
> - Toda instalación, configuración, estado y bitácora de `VORTEX-M2B-SPOT` reside exclusivamente en `/home/ubuntu/vortex-m2b-spot` (código fuente clonado p.ej. en `/home/ubuntu/src/VORTEX-M2B-SPOT` o `/home/ubuntu/VORTEX-M2B-SPOT` y runtime instalado en `/home/ubuntu/vortex-m2b-spot`).

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

## 3. Clonado e Inicialización de `.env` (`chmod 600`)

1. Clonar el repositorio fuente en un directorio de trabajo limpio (sin tocar `/home/ubuntu/vortex-rs`):
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
   ```
   - Para recibir alertas en Telegram durante los 30 días de `--paper`, completar `TELEGRAM_BOT_TOKEN` y `TELEGRAM_CHAT_ID`.
   - Mantener `BYBIT_API_KEY=`, `BYBIT_API_SECRET=`, `BYBIT_SPOT_LIVE_EXECUTION=false` y `BYBIT_SPOT_ACK_LIVE=` vacíos/desactivados durante toda la fase `--paper`.
   - Confirmar permisos estrictos del archivo `.env`:
     ```bash
     ls -la /home/ubuntu/vortex-m2b-spot/.env
     # Esperado: -rw------- 1 ubuntu ubuntu ... .env
     ```

---

## 4. Instalación Automatizada (`deploy/install_tokio.sh`)

Ejecutar el script de instalación provisto en el repositorio:

```bash
cd /home/ubuntu/VORTEX-M2B-SPOT
bash deploy/install_tokio.sh
```

Qué realiza exactamente `deploy/install_tokio.sh` (`deploy/install_tokio.sh:1-46`):
1. Crea `/home/ubuntu/vortex-m2b-spot/{bin,state,logs}` (`línea 13`).
2. Si `/home/ubuntu/vortex-m2b-spot/.env` no existe, copia `.env.example` con `install -m 0600`; si ya existe, lo preserva (`líneas 15-20`).
3. Compila el binario en perfil `release` con mínima prioridad CPU/IO (`nice -n 19 ionice -c 3 cargo build --release`, `línea 24`).
4. Instala el binario atómicamente (`vortex-m2b-spot.tmp.$$` -> `mv -f`) en `/home/ubuntu/vortex-m2b-spot/bin/vortex-m2b-spot` (`líneas 33-35`).
5. Instala el helper de solo lectura `deploy/vspot` en `/usr/local/bin/vspot` con permisos `0755` (`línea 38`).
6. Instala `deploy/vortex-m2b-spot.service` y `deploy/vortex-m2b-spot.timer` en `/etc/systemd/system/` con permisos `0644`, ejecuta `systemctl daemon-reload` y habilita `systemctl enable --now vortex-m2b-spot.timer` (`líneas 40-44`).

---

## 5. Verificación Post-Instalación de `systemd` y Conectividad

1. **Verificar configuración del timer `systemd`** (`deploy/vortex-m2b-spot.timer:5-9`):
   - `OnCalendar=*-*-* *:00:08 UTC` (dispara 8 segundos después de cada cierre horario UTC)
   - `RandomizedDelaySec=4` (jitter controlado entre `:00:08` y `:00:12` UTC)
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
   - `vspot health` debe imprimir `=== VORTEX-M2B-SPOT HEALTH OK ===`, `Clock Skew (ms) : <N> (limit 2000)`, `State Dir : state (rw)`, `Logs Dir : logs (rw)` y las 5 reglas Spot (`BTCUSDT`, `ETHUSDT`, `SOLUSDT`, `LINKUSDT`, `AVAXUSDT`) con `status=Trading`.
   - `vspot timer` debe mostrar `vortex-m2b-spot.timer` en estado `active (waiting)` con el próximo disparo al minuto `:00:08 UTC`.

4. **Ejecución manual inicial (opcional para validar primer ciclo `--paper`)**:
   ```bash
   cd /home/ubuntu/vortex-m2b-spot
   ./bin/vortex-m2b-spot run --paper
   vspot status
   vspot ledger
   ```
   Confirmar `drift_1e8=0` tanto en la salida `CYCLE_EXECUTED` como en `vspot status` para `B2-01` y (al cruzar un cierre `4h` UTC `00/04/08/12/16/20 UTC`) para `A0_4H`.

---

## 6. Monitoreo Diario y Semanal con `/usr/local/bin/vspot`

El script `/usr/local/bin/vspot` (`deploy/vspot:1-79`) provee 5 subcomandos de **solo lectura**:

| Comando | Qué inspecciona | Frecuencia recomendada |
|---|---|---|
| `vspot status` | Estado de `Primary B2-01 (1h)`, `Shadow A0_4H (4h UTC Paper)`, pesos `CUR_W`/`TGT_W`, `SIGMA_ANN`, `TSMOM`, `Drift (1e8)`, `TWR Drawdown` y presencia de `DRAIN_MODE.lock` / `HALT_LOCK`. | Diaria |
| `vspot logs` | Últimas 100 líneas del journal de `systemd` (`journalctl -u vortex-m2b-spot.service -n 100 --no-pager`). | Diaria |
| `vspot ledger` | Últimas 20 entradas selladas (FNV-1a 64-bit) de `/home/ubuntu/vortex-m2b-spot/logs/m2b_audit_ledger.jsonl`. | Diaria / Semanal |
| `vspot health` | Conectividad pública a Bybit V5, clock skew (`<= 2000ms`), estado `Trading` de los 5 pares y permisos `rw` de `state/` y `logs/`. | Semanal o tras incidentes |
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

## 7. Procedimientos de Emergencia (`DRAIN_MODE.lock` y `HALT_LOCK`)

### 7.1 `DRAIN_MODE.lock` (Drenaje ordenado: bloquea nuevas compras, permite ventas de salida)

- **Efecto en `--paper`:** `allow_buys` pasa a `false` (`src/application/cycle.rs:268`). El motor **no abre ni amplía posiciones** (`Buy` bloqueadas), pero **sí ejecuta ventas de salida** (`Sell`) cuando el rebalanceador o el umbral de liquidación lo requieren.
- **Efecto en `--live-spot`:** Bloquea la ejecución live en la puerta de seguridad (`AppError::LiveSafetyLockDenied("DRAIN_MODE.lock is active")`, `src/application/cycle.rs:88-92`).

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

### 7.2 `HALT_LOCK` (Parada dura por Drawdown TWR `> 25%`, discrepancia de conciliación o intervención manual)

- **Cuándo se activa automáticamente:**
  1. Drawdown TWR estrictamente superior al `25%` (`dd_1e10 > 2_500_000_000`, `src/domain/accounting.rs:7,565`), escribiendo `state/HALT_LOCK` y evento `HARD_DRAWDOWN_HALT` (`src/application/cycle.rs:460-503`).
  2. Discrepancia de saldo base `> base_precision` en conciliación `--live-spot` (`src/application/cycle.rs:213-247`), escribiendo `state/HALT_LOCK` y evento `RECONCILIATION_MISMATCH`.
- **Efecto en `--paper`:** `allow_buys = false`, `ledger.is_halted() = true`; las ventas de liquidación siguen siendo evaluables si la señal cae por debajo del umbral, pero no hay nuevas compras.
- **Efecto en `--live-spot`:** Rechazo inmediato fail-closed (`AppError::HaltLockActive`, `src/application/cycle.rs:93-95`).

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
3. Solo tras autorización explícita del operador, remover `HALT_LOCK` (si el campo `portfolio.halted` quedó `true` dentro de `state/primary_state.json`, se debe auditar/reiniciar el libro antes de reanudar compras):
   ```bash
   rm -f /home/ubuntu/vortex-m2b-spot/state/HALT_LOCK
   ```

### 7.3 Detener temporalmente el timer `systemd`

```bash
sudo systemctl stop vortex-m2b-spot.timer
sudo systemctl disable vortex-m2b-spot.timer
```

---

## 8. Checklist de Auditoría de 30 Días (`--paper` -> Pre-requisito `--live-spot`)

Antes de considerar la promoción desde `--paper` hacia `--live-spot`, verificar **todos** los siguientes puntos al cumplirse los 30 días (~720 ciclos horarios `1h` y ~180 cierres `4h` UTC):

- [ ] **Continuidad de ciclos (`B2-01`)**: `>= 99%` de las 720 horas UTC ejecutadas puntualmente (`*:00:08..12 UTC`), sin fallos repetidos de reloj (`ClockSkew`) ni `run.lock` colgados.
- [ ] **Telemetría paralela (`A0_4H`)**: `180` cierres `4h` UTC (`00/04/08/12/16/20 UTC`) registrados en `state/m2b_shadow_4h_state.json` y `logs/m2b_audit_ledger.jsonl` con `SHADOW_4H_CYCLE`.
- [ ] **Invariante contable**: `100%` de las líneas en `logs/m2b_audit_ledger.jsonl` con `"drift_1e8":0` y sello `fnv1a64_hex` válido (cero manipulaciones o truncamientos).
- [ ] **Sin `HALT_LOCK` no resueltos**: Ningún evento inesperado de `HARD_DRAWDOWN_HALT`; cualquier drawdown registrado dentro de límites de riesgo esperados (`<= 25%` TWR).
- [ ] **Comparativa `B2-01` (1h) vs `A0_4H` (4h)**: Evaluar TWR final, máximo drawdown TWR, rotación acumulada y arrastre de comisiones (`cum_sell_fees_usdt_1e8` + `cum_buy_fees_usdt_est_1e8`) para confirmar superioridad o equivalencia controlada de `B2-01`.
- [ ] **Checklist de promoción a `--live-spot` (cuando el operador apruebe)**:
  1. Configurar API Key Spot V5 de Bybit con permisos estrictos de **Spot Trade** (sin retiros, IP whitelisted al host Tokio).
  2. Confirmar cuenta Bybit en modo `UNIFIED` con capital inicial USDT depositado.
  3. Activar el **Double Safety Lock** en `.env`:
     - `BYBIT_SPOT_LIVE_EXECUTION=true`
     - `BYBIT_SPOT_ACK_LIVE=YES_I_ACCEPT_0X_SPOT_RISK`
  4. Actualizar `ExecStart` del servicio `systemd` de `--paper` a `--live-spot` (previo respaldo del estado paper y arranque limpio del estado live desde `/v5/account/wallet-balance`).
