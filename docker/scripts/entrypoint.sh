#!/bin/bash
set -euo pipefail

VALIDATOR_INDEX=${VALIDATOR_INDEX:-0}
IS_BOOTSTRAP=${IS_BOOTSTRAP:-false}
BOOTSTRAP_PEERS=${BOOTSTRAP_PEERS:-""}
CHAIN_ID=${CHAIN_ID:-1337}
DATA_DIR=${DATA_DIR:-/data}
SHARED_DIR=${SHARED_DIR:-/shared}
BASE_P2P_PORT=${BASE_P2P_PORT:-30303}
BASE_RPC_PORT=${BASE_RPC_PORT:-8545}

MODE="${1:-validator}"
shift || true

log() { echo "[entrypoint] $*"; }
error() { echo "[entrypoint] ERROR: $*" >&2; exit 1; }

auto_config_needs_init() {
    [[ "${RESET_AUTO_CONFIG:-false}" == "true" ]] && return 0
    [[ -f "${SHARED_DIR}/genesis.json" && -f "${SHARED_DIR}/peers.json" ]] || return 0

    local validators threshold chain_id
    validators=$(jq -r '.validators // empty' "${SHARED_DIR}/peers.json" 2>/dev/null || true)
    threshold=$(jq -r '.threshold // empty' "${SHARED_DIR}/peers.json" 2>/dev/null || true)
    chain_id=$(jq -r '.chain_id // empty' "${SHARED_DIR}/genesis.json" 2>/dev/null || true)

    if [[ "$validators" == "$NUM_VALIDATORS" && "$threshold" == "$THRESHOLD" && "$chain_id" == "$CHAIN_ID" ]]; then
        return 1
    fi

    return 0
}

patch_auto_bootstrappers() {
    for i in $(seq 0 $((NUM_VALIDATORS - 1))); do
        local port=$((BASE_P2P_PORT + i))
        sed -i "s/node${i}:30303/127.0.0.1:${port}/g" "${SHARED_DIR}/peers.json"
    done
}

prepare_auto_node() {
    local idx="$1"
    local node_data="${DATA_DIR}/node${idx}"
    local p2p_port=$((BASE_P2P_PORT + idx))
    local rpc_port=$((BASE_RPC_PORT + idx))
    local rpc_addr="127.0.0.1:${rpc_port}"
    if [[ "$idx" == "0" ]]; then
        rpc_addr="0.0.0.0:${PORT:-8545}"
    fi

    mkdir -p "${node_data}"
    cp "${SHARED_DIR}/node${idx}/validator.key" "${node_data}/"
    cp "${SHARED_DIR}/node${idx}/share.key" "${node_data}/"
    cp "${SHARED_DIR}/node${idx}/output.json" "${node_data}/"
    cp "${SHARED_DIR}/genesis.json" "${node_data}/"

    cat > "${SHARED_DIR}/kora-node${idx}.toml" <<AUTOEOF
chain_id = ${CHAIN_ID}
data_dir = "${node_data}"

[network]
listen_addr = "127.0.0.1:${p2p_port}"
dialable_addr = "127.0.0.1:${p2p_port}"

[execution]
gas_limit = ${GAS_LIMIT:-250000000}
block_time_ms = ${BLOCK_TIME_MS:-50}

[hdc]
enabled = ${HDC_ENABLED:-true}

[rpc]
http_addr = "${rpc_addr}"
AUTOEOF
}

stop_auto_validators() {
    local status="${1:-0}"
    shift || true
    for pid in "$@"; do
        kill "$pid" 2>/dev/null || true
    done
    wait "$@" 2>/dev/null || true
    exit "$status"
}

case "$MODE" in
    setup)
        log "Running setup mode..."
        exec /usr/local/bin/keygen setup "$@"
        ;;
        
    dkg)
        log "Running DKG ceremony mode..."
        
        [[ -f "${SHARED_DIR}/peers.json" ]] || error "peers.json not found"
        [[ -f "${DATA_DIR}/validator.key" ]] || error "validator.key not found"
        
        if [[ -f "${DATA_DIR}/share.key" && -f "${DATA_DIR}/output.json" ]]; then
            log "DKG already completed (share.key exists)"
            exit 0
        fi
        
        if [[ "$IS_BOOTSTRAP" != "true" && -n "$BOOTSTRAP_PEERS" ]]; then
            BOOTSTRAP_HOST=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f1)
            BOOTSTRAP_PORT=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f2)
            
            log "Waiting for bootstrap peer ${BOOTSTRAP_HOST}:${BOOTSTRAP_PORT}..."
            timeout=120
            while ! nc -z "$BOOTSTRAP_HOST" "$BOOTSTRAP_PORT" 2>/dev/null; do
                timeout=$((timeout - 1))
                [[ $timeout -le 0 ]] && error "Timeout waiting for bootstrap peer"
                sleep 1
            done
            log "Bootstrap peer reachable"
        fi
        
        exec /usr/local/bin/kora dkg \
            --data-dir "$DATA_DIR" \
            --peers "${SHARED_DIR}/peers.json" \
            --chain-id "$CHAIN_ID" \
            "$@"
        ;;
        
    auto)
        log "Running auto-init mode..."

        NUM_VALIDATORS=${NUM_VALIDATORS:-1}
        THRESHOLD=${THRESHOLD:-1}

        if auto_config_needs_init; then
            log "Initializing key material (${NUM_VALIDATORS} validators, threshold ${THRESHOLD})..."
            rm -rf "${SHARED_DIR}"/node* "${SHARED_DIR}"/secondary* \
                "${SHARED_DIR}/genesis.json" "${SHARED_DIR}/peers.json" \
                "${SHARED_DIR}"/kora*.toml "${DATA_DIR}"/node*

            /usr/local/bin/keygen setup \
                --validators="${NUM_VALIDATORS}" \
                --threshold="${THRESHOLD}" \
                --chain-id="${CHAIN_ID}" \
                --output-dir="${SHARED_DIR}"

            patch_auto_bootstrappers

            log "Running trusted dealer DKG..."
            /usr/local/bin/keygen dkg-deal \
                --validators="${NUM_VALIDATORS}" \
                --threshold="${THRESHOLD}" \
                --output-dir="${SHARED_DIR}"

            log "Auto-init keygen complete"
        else
            log "Existing auto-init key material matches requested validator set"
        fi

        for i in $(seq 0 $((NUM_VALIDATORS - 1))); do
            prepare_auto_node "$i"
        done

        if [[ "$NUM_VALIDATORS" -le 1 ]]; then
            CONFIG_FILE="${CONFIG_FILE:-${SHARED_DIR}/kora-node${VALIDATOR_INDEX}.toml}"
            cp "${SHARED_DIR}/genesis.json" "${DATA_DIR}/" 2>/dev/null || true
            cp "${SHARED_DIR}/node${VALIDATOR_INDEX}/validator.key" "${DATA_DIR}/" 2>/dev/null || true
            cp "${SHARED_DIR}/node${VALIDATOR_INDEX}/share.key" "${DATA_DIR}/" 2>/dev/null || true
            cp "${SHARED_DIR}/node${VALIDATOR_INDEX}/output.json" "${DATA_DIR}/" 2>/dev/null || true
            touch "${DATA_DIR}/.ready"

            log "Starting single validator..."
            exec /usr/local/bin/kora --config "${CONFIG_FILE}" validator \
                --data-dir "$DATA_DIR" \
                --peers "${SHARED_DIR}/peers.json" \
                --chain-id "$CHAIN_ID" \
                "$@"
        fi

        log "Starting ${NUM_VALIDATORS} validators in this container..."
        touch "${DATA_DIR}/.ready"

        pids=()
        trap 'stop_auto_validators 143 "${pids[@]}"' TERM INT
        for i in $(seq 0 $((NUM_VALIDATORS - 1))); do
            node_data="${DATA_DIR}/node${i}"
            config_file="${SHARED_DIR}/kora-node${i}.toml"
            log "Starting validator ${i} with ${config_file}"
            /usr/local/bin/kora --config "${config_file}" validator \
                --data-dir "${node_data}" \
                --peers "${SHARED_DIR}/peers.json" \
                --chain-id "$CHAIN_ID" \
                "$@" &
            pids+=("$!")
            sleep 1
        done

        wait -n "${pids[@]}"
        status=$?
        log "A validator exited with status ${status}; stopping remaining validators"
        stop_auto_validators "$status" "${pids[@]}"
        ;;

    validator)
        log "Running validator mode..."

        [[ -f "${SHARED_DIR}/genesis.json" ]] || error "genesis.json not found"
        [[ -f "${DATA_DIR}/validator.key" ]] || error "validator.key not found"
        [[ -f "${DATA_DIR}/share.key" ]] || error "share.key not found (run DKG first)"
        [[ -f "${DATA_DIR}/output.json" ]] || error "output.json not found (run DKG first)"
        
        cp "${SHARED_DIR}/genesis.json" "${DATA_DIR}/" 2>/dev/null || true
        touch "${DATA_DIR}/.ready"
        
        if [[ "$IS_BOOTSTRAP" != "true" && -n "$BOOTSTRAP_PEERS" ]]; then
            BOOTSTRAP_HOST=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f1)
            BOOTSTRAP_PORT=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f2)
            
            log "Waiting for bootstrap peer ${BOOTSTRAP_HOST}:${BOOTSTRAP_PORT}..."
            timeout=120
            while ! nc -z "$BOOTSTRAP_HOST" "$BOOTSTRAP_PORT" 2>/dev/null; do
                timeout=$((timeout - 1))
                [[ $timeout -le 0 ]] && error "Timeout waiting for bootstrap peer"
                sleep 1
            done
        fi
        
        CONFIG_ARG=""
        if [[ -n "${CONFIG_FILE:-}" ]]; then
            CONFIG_ARG="--config $CONFIG_FILE"
        fi

        exec /usr/local/bin/kora $CONFIG_ARG validator \
            --data-dir "$DATA_DIR" \
            --peers "${SHARED_DIR}/peers.json" \
            --chain-id "$CHAIN_ID" \
            "$@"
        ;;

    secondary)
        log "Running secondary peer mode..."

        [[ -f "${SHARED_DIR}/peers.json" ]] || error "peers.json not found"
        [[ -f "${DATA_DIR}/validator.key" ]] || error "validator.key not found"

        touch "${DATA_DIR}/.ready"

        if [[ "$IS_BOOTSTRAP" != "true" && -n "$BOOTSTRAP_PEERS" ]]; then
            BOOTSTRAP_HOST=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f1)
            BOOTSTRAP_PORT=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f2)

            log "Waiting for bootstrap peer ${BOOTSTRAP_HOST}:${BOOTSTRAP_PORT}..."
            timeout=120
            while ! nc -z "$BOOTSTRAP_HOST" "$BOOTSTRAP_PORT" 2>/dev/null; do
                timeout=$((timeout - 1))
                [[ $timeout -le 0 ]] && error "Timeout waiting for bootstrap peer"
                sleep 1
            done
        fi

        exec /usr/local/bin/kora secondary \
            --data-dir "$DATA_DIR" \
            --peers "${SHARED_DIR}/peers.json" \
            --chain-id "$CHAIN_ID" \
            "$@"
        ;;
        
    *)
        exec "$MODE" "$@"
        ;;
esac
