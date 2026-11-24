#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
TARGET_DIR="/home/ubuntu/vortex-m2b-spot"
BIN_DIR="${TARGET_DIR}/bin"
STATE_DIR="${TARGET_DIR}/state"
LOGS_DIR="${TARGET_DIR}/logs"
SYSTEMD_DIR="/etc/systemd/system"

echo "[1/6] Preparing target directories at ${TARGET_DIR}..."
mkdir -p "${BIN_DIR}" "${STATE_DIR}" "${LOGS_DIR}"

if [[ ! -f "${TARGET_DIR}/.env" && -f "${REPO_DIR}/.env.example" ]]; then
    echo "[2/6] Initializing ${TARGET_DIR}/.env from .env.example (paper defaults)..."
    install -m 0600 "${REPO_DIR}/.env.example" "${TARGET_DIR}/.env"
else
    echo "[2/6] Preserving existing ${TARGET_DIR}/.env..."
fi

echo "[3/6] Building release binary with low CPU/IO priority..."
cd "${REPO_DIR}"
nice -n 19 ionice -c 3 cargo build --release

RELEASE_BIN="${REPO_DIR}/target/release/vortex-m2b-spot"
if [[ ! -x "${RELEASE_BIN}" ]]; then
    echo "ERROR: release binary missing at ${RELEASE_BIN}" >&2
    exit 1
fi

echo "[4/6] Installing binary atomically to ${BIN_DIR}/vortex-m2b-spot..."
TMP_BIN="${BIN_DIR}/vortex-m2b-spot.tmp.$$"
install -m 0755 "${RELEASE_BIN}" "${TMP_BIN}"
mv -f "${TMP_BIN}" "${BIN_DIR}/vortex-m2b-spot"

echo "[5/6] Installing read-only helper /usr/local/bin/vspot..."
sudo install -m 0755 "${REPO_DIR}/deploy/vspot" /usr/local/bin/vspot

echo "[6/6] Installing systemd service and timer in --paper mode..."
sudo install -m 0644 "${REPO_DIR}/deploy/vortex-m2b-spot.service" "${SYSTEMD_DIR}/vortex-m2b-spot.service"
sudo install -m 0644 "${REPO_DIR}/deploy/vortex-m2b-spot.timer" "${SYSTEMD_DIR}/vortex-m2b-spot.timer"
sudo systemctl daemon-reload
sudo systemctl enable --now vortex-m2b-spot.timer

echo "Done. Verify with: vspot health && vspot timer"
