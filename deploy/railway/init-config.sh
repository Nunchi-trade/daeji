#!/usr/bin/env bash
# init-config.sh -- Generate keys and threshold shares for Railway deployment
set -euo pipefail

NUM_VALIDATORS="${NUM_VALIDATORS:-3}"
THRESHOLD="${THRESHOLD:-2}"
CHAIN_ID="${CHAIN_ID:-1337}"
SHARED_DIR="${SHARED_DIR:-/shared}"

echo "[init] Running keygen setup (${NUM_VALIDATORS} validators, threshold ${THRESHOLD})..."
/usr/local/bin/keygen setup \
    --validators="${NUM_VALIDATORS}" \
    --threshold="${THRESHOLD}" \
    --chain-id="${CHAIN_ID}" \
    --output-dir="${SHARED_DIR}"

echo "[init] Patching peers.json hostnames for Railway..."
sed -i \
    -e 's/node0:30303/validator-0.railway.internal:30303/g' \
    -e 's/node1:30303/validator-1.railway.internal:30303/g' \
    -e 's/node2:30303/validator-2.railway.internal:30303/g' \
    "${SHARED_DIR}/peers.json"

echo "[init] Running trusted dealer DKG..."
/usr/local/bin/keygen dkg-deal \
    --validators="${NUM_VALIDATORS}" \
    --threshold="${THRESHOLD}" \
    --output-dir="${SHARED_DIR}"

echo "[init] Generating kora.toml config..."
cat > "${SHARED_DIR}/kora.toml" <<EOF
chain_id = ${CHAIN_ID}
data_dir = "/data"

[execution]
gas_limit = ${GAS_LIMIT:-250000000}
block_time_ms = ${BLOCK_TIME_MS:-50}

[hdc]
enabled = ${HDC_ENABLED:-true}

[rpc]
http_addr = "0.0.0.0:8545"
EOF
echo "[init] kora.toml written:"
cat "${SHARED_DIR}/kora.toml"

echo "[init] Setting permissions..."
for i in $(seq 0 $((NUM_VALIDATORS - 1))); do
    chown -R 1000:1000 "${SHARED_DIR}/node${i}"
done

echo "[init] Init complete. Contents of ${SHARED_DIR}:"
ls -la "${SHARED_DIR}/"
for i in $(seq 0 $((NUM_VALIDATORS - 1))); do
    echo "--- node${i} ---"
    ls -la "${SHARED_DIR}/node${i}/"
done
cat "${SHARED_DIR}/peers.json"

echo "[init] Done. Stop this service and start validators."
