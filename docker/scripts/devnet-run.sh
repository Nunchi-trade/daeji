#!/usr/bin/env bash
set -eo pipefail

# Parse arguments
INTERACTIVE_DKG=false
NUM_VALIDATORS=4
while [[ $# -gt 0 ]]; do
    case $1 in
        --interactive-dkg)
            INTERACTIVE_DKG=true
            shift
            ;;
        --nodes)
            NUM_VALIDATORS="$2"
            shift 2
            ;;
        --nodes=*)
            NUM_VALIDATORS="${1#*=}"
            shift
            ;;
        *)
            shift
            ;;
    esac
done

if ! [[ "$NUM_VALIDATORS" =~ ^[0-9]+$ ]] || [[ "$NUM_VALIDATORS" -lt 1 ]]; then
    echo "num_validators must be a positive integer" >&2
    exit 1
fi

THRESHOLD=$((NUM_VALIDATORS - (NUM_VALIDATORS - 1) / 3))

if [[ "$NUM_VALIDATORS" -eq 4 ]]; then
    COMPOSE_FILE="compose/devnet.yaml"
else
    COMPOSE_FILE="compose/devnet.generated.yaml"
fi

validator_services() {
    local services=""
    for ((i = 0; i < NUM_VALIDATORS; i++)); do
        services+="validator-node${i} "
    done
    echo -n "$services"
}

dkg_services() {
    local services=""
    for ((i = 0; i < NUM_VALIDATORS; i++)); do
        services+="dkg-node${i} "
    done
    echo -n "$services"
}

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

# Icons
CHECK="✓"
CROSS="✗"
ARROW="→"
SPIN="◐◓◑◒"

spin_idx=0
spinner() {
    printf "\r  ${CYAN}${SPIN:spin_idx:1}${NC} %s" "$1"
    spin_idx=$(( (spin_idx + 1) % 4 ))
}

clear_line() {
    printf "\r                                                              \r"
}

# Run a command with a spinner, suppressing output
run_with_spinner() {
    local msg=$1
    shift
    local logfile=$(mktemp)
    
    # Start command in background
    "$@" > "$logfile" 2>&1 &
    local pid=$!
    
    # Spin while waiting
    while kill -0 "$pid" 2>/dev/null; do
        spinner "$msg"
        sleep 0.15
    done
    
    # Get exit code
    wait "$pid"
    local exit_code=$?
    
    clear_line
    
    if [[ $exit_code -ne 0 ]]; then
        cat "$logfile"
        rm -f "$logfile"
        return $exit_code
    fi
    
    rm -f "$logfile"
    return 0
}

print_header() {
    echo ""
    echo -e "${BOLD}${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
    if [[ "$INTERACTIVE_DKG" == "true" ]]; then
        echo -e "${BOLD}${BLUE}║${NC}        ${BOLD}KORA DEVNET${NC} ${GREEN}(Interactive DKG)${NC}                 ${BOLD}${BLUE}║${NC}"
    else
        echo -e "${BOLD}${BLUE}║${NC}        ${BOLD}KORA DEVNET${NC} ${YELLOW}(Trusted Dealer)${NC}                  ${BOLD}${BLUE}║${NC}"
    fi
    echo -e "${BOLD}${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
    echo ""
    echo -e "  ${DIM}Chain ID:${NC} ${CHAIN_ID:-1337}  ${DIM}│${NC}  ${DIM}Validators:${NC} ${NUM_VALIDATORS}  ${DIM}│${NC}  ${DIM}Threshold:${NC} ${THRESHOLD}"
    echo ""
}

print_phase() {
    local phase=$1
    local desc=$2
    echo -e "${BOLD}${CYAN}[$phase]${NC} ${BOLD}$desc${NC}"
}

print_success() {
    echo -e "  ${GREEN}${CHECK}${NC} $1"
}

print_skip() {
    echo -e "  ${YELLOW}${ARROW}${NC} $1 ${DIM}(cached)${NC}"
}

print_error() {
    echo -e "  ${RED}${CROSS}${NC} $1"
}

print_endpoints() {
    local last_p2p=$((30400 + NUM_VALIDATORS - 1))
    local last_rpc=$((8545 + NUM_VALIDATORS - 1))
    echo ""
    echo -e "${BOLD}Endpoints${NC}"
    echo -e "  ${DIM}P2P:${NC}        localhost:30400-${last_p2p}"
    echo -e "  ${DIM}RPC:${NC}        localhost:8545-${last_rpc}"
    echo -e "  ${DIM}Secondary:${NC}  localhost:30500"
    echo -e "  ${DIM}Prometheus:${NC} http://localhost:9090"
    echo -e "  ${DIM}Grafana:${NC}    http://localhost:3000"
    echo ""
    echo -e "${DIM}Run 'just devnet-stats' for live monitoring${NC}"
    echo ""
}

check_dkg_outputs() {
    local expected_checksum=""

    for ((i = 0; i < NUM_VALIDATORS; i++)); do
        local volume="kora-devnet_data_node${i}"

        if ! docker volume inspect "$volume" >/dev/null 2>&1; then
            return 1
        fi

        if ! docker run --rm -v "${volume}:/data" alpine \
            test -f /data/share.key -a -f /data/output.json >/dev/null 2>&1; then
            return 1
        fi

        local checksum
        checksum=$(docker run --rm -v "${volume}:/data" alpine \
            sha256sum /data/output.json 2>/dev/null | awk '{print $1}')

        if [[ -z "$checksum" ]]; then
            return 1
        fi

        if [[ -z "$expected_checksum" ]]; then
            expected_checksum="$checksum"
        elif [[ "$checksum" != "$expected_checksum" ]]; then
            return 1
        fi
    done

    return 0
}

clear_dkg_outputs() {
    for ((i = 0; i < NUM_VALIDATORS; i++)); do
        local volume="kora-devnet_data_node${i}"
        docker volume inspect "$volume" >/dev/null 2>&1 || continue
        docker run --rm -v "${volume}:/data" alpine \
            rm -f /data/share.key /data/output.json /data/dkg_state.json >/dev/null 2>&1 || true
    done
}

clear_runtime_state() {
    local volume
    for ((i = 0; i < NUM_VALIDATORS; i++)); do
        volume="kora-devnet_runtime_node${i}"
        docker volume inspect "$volume" >/dev/null 2>&1 || continue
        docker run --rm -v "${volume}:/runtime" alpine \
            sh -c 'rm -rf /runtime/* /runtime/.[!.]* /runtime/..?*' >/dev/null 2>&1 || true
    done

    for volume in kora-devnet_runtime_secondary0; do
        docker volume inspect "$volume" >/dev/null 2>&1 || continue
        docker run --rm -v "${volume}:/runtime" alpine \
            sh -c 'rm -rf /runtime/* /runtime/.[!.]* /runtime/..?*' >/dev/null 2>&1 || true
    done
}

clear_startup_barrier() {
    local volume="kora-devnet_startup_barrier"
    docker volume inspect "$volume" >/dev/null 2>&1 || return 0
    docker run --rm -v "${volume}:/barrier" alpine \
        sh -c 'rm -f /barrier/*.ready && chown -R 1000:1000 /barrier' >/dev/null 2>&1 || true
}

cd "$(dirname "$0")/.."

if [[ "$COMPOSE_FILE" != "compose/devnet.yaml" ]]; then
    ./scripts/generate-devnet-compose.sh "$NUM_VALIDATORS" > "$COMPOSE_FILE"
fi

print_header

# Phase 0: Build
print_phase "0/3" "Building Docker image"
if run_with_spinner "Building kora:local image..." docker buildx bake --allow=fs.read=.. -f docker-bake.hcl kora-local; then
    print_success "Image built successfully"
else
    print_error "Build failed"
    exit 1
fi

# Check existing state
CONFIG_EXISTS=false
SHARES_EXIST=false

cached_validator_count() {
    docker volume inspect kora-devnet_shared_config >/dev/null 2>&1 || return 1
    docker run --rm -v kora-devnet_shared_config:/shared alpine \
        cat /shared/peers.json 2>/dev/null | jq -r '.validators // empty' 2>/dev/null
}

if docker volume inspect kora-devnet_shared_config >/dev/null 2>&1 && \
    docker run --rm -v kora-devnet_shared_config:/shared alpine test -f /shared/peers.json 2>/dev/null && \
    docker volume inspect kora-devnet_data_secondary0 >/dev/null 2>&1 && \
    docker run --rm -v kora-devnet_data_secondary0:/data alpine test -f /data/validator.key 2>/dev/null; then
    cached_count="$(cached_validator_count)"
    if [[ "$cached_count" == "$NUM_VALIDATORS" ]]; then
        CONFIG_EXISTS=true
    fi
fi

if check_dkg_outputs; then
    SHARES_EXIST=true
else
    clear_dkg_outputs
fi

echo ""

# Phase 1: Configuration
print_phase "1/3" "Configuration"
if [[ "$CONFIG_EXISTS" != "true" ]]; then
    if [[ "$INTERACTIVE_DKG" == "true" ]]; then
        # Interactive DKG: only run setup (no dkg-deal)
        if run_with_spinner "Generating peer configuration..." docker compose -f "$COMPOSE_FILE" run --rm init-setup; then
            print_success "Generated peer configuration"
        else
            print_error "Configuration failed"
            exit 1
        fi
    else
        # Trusted dealer: run setup + dkg-deal
        if run_with_spinner "Generating peer configuration..." docker compose -f "$COMPOSE_FILE" run --rm init-config; then
            print_success "Generated peer configuration"
        else
            print_error "Configuration failed"
            exit 1
        fi
    fi
else
    print_skip "Peer configuration exists"
fi

if [[ "$INTERACTIVE_DKG" == "true" ]]; then
    if [[ "$SHARES_EXIST" != "true" ]]; then
        echo ""
        print_phase "1.5/3" "Interactive DKG Ceremony"

        # DKG jobs use the same node0..node3 hostnames as validators. Stop validators first so
        # Docker DNS cannot route ceremony traffic to stale validator containers.
        docker compose -f "$COMPOSE_FILE" stop $(validator_services) >/dev/null 2>&1 || true
        
        # Start DKG nodes
        run_with_spinner "Starting DKG nodes..." docker compose -f "$COMPOSE_FILE" --profile interactive-dkg up -d \
            $(dkg_services)
        
        # Wait for DKG completion
        start_time=$(date +%s)
        timeout=300  # 5 minutes for DKG
        
        while true; do
            # Check if all DKG containers have exited successfully (use -a to include stopped containers)
            EXITED=$(docker compose -f "$COMPOSE_FILE" ps -a --format json 2>/dev/null | \
                jq -r 'select(.Service | startswith("dkg-")) | select(.State == "exited") | select(.ExitCode == 0) | .Service' 2>/dev/null | wc -l | tr -d ' ')
            
            FAILED=$(docker compose -f "$COMPOSE_FILE" ps -a --format json 2>/dev/null | \
                jq -r 'select(.Service | startswith("dkg-")) | select(.State == "exited") | select(.ExitCode != 0) | .Service' 2>/dev/null | wc -l | tr -d ' ')
            
            elapsed=$(($(date +%s) - start_time))
            
            if [[ "$FAILED" -gt 0 ]]; then
                clear_line
                print_error "DKG ceremony failed"
                echo ""
                echo -e "${RED}DKG node logs:${NC}"
                docker compose -f "$COMPOSE_FILE" logs $(dkg_services) --tail=50
                exit 1
            fi
            
            if [[ "$EXITED" -ge "$NUM_VALIDATORS" ]]; then
                clear_line
                print_success "Interactive DKG ceremony completed"
                break
            fi
            
            if [[ "$elapsed" -ge "$timeout" ]]; then
                clear_line
                print_error "Timeout waiting for DKG ceremony"
                exit 1
            fi
            
            spinner "Running DKG ceremony... (${elapsed}s)"
            sleep 0.15
        done
        
        # Stop DKG containers (they should already be stopped)
        docker compose -f "$COMPOSE_FILE" --profile interactive-dkg stop $(dkg_services) 2>/dev/null || true
    else
        print_skip "DKG shares exist"
    fi
else
    if [[ "$SHARES_EXIST" != "true" ]]; then
        echo ""
        print_phase "1.5/3" "Trusted Dealer DKG"

        if run_with_spinner "Generating threshold shares..." docker compose -f "$COMPOSE_FILE" run --rm init-config; then
            print_success "Threshold shares generated"
        else
            print_error "Trusted dealer DKG failed"
            exit 1
        fi
    else
        print_skip "Threshold shares exist"
    fi
fi

echo ""

# Phase 2: Validators and secondary peers
print_phase "2/3" "Starting validators and secondary peers"

docker compose -f "$COMPOSE_FILE" stop \
    $(validator_services) secondary-node0 >/dev/null 2>&1 || true
clear_runtime_state
clear_startup_barrier

if [[ "${COMPOSE_PROFILES:-}" == *observability* ]]; then
    run_with_spinner "Launching validator, secondary, and observability containers..." docker compose -f "$COMPOSE_FILE" --profile observability up -d \
        $(validator_services) secondary-node0 \
        prometheus grafana loki promtail
else
    run_with_spinner "Launching validator and secondary containers..." docker compose -f "$COMPOSE_FILE" up -d \
        $(validator_services) secondary-node0
fi

# Wait for validators with spinner
start_time=$(date +%s)
timeout=120

while true; do
    HEALTHY=$(docker compose -f "$COMPOSE_FILE" ps --format json 2>/dev/null | \
        jq -r 'select(.Service | startswith("validator-")) | select(.Health == "healthy") | .Service' 2>/dev/null | wc -l | tr -d ' ')
    
    elapsed=$(($(date +%s) - start_time))
    
    if [[ "$HEALTHY" -ge "$NUM_VALIDATORS" ]]; then
        clear_line
        print_success "All ${NUM_VALIDATORS} validators healthy"
        break
    fi
    
    if [[ "$elapsed" -ge "$timeout" ]]; then
        clear_line
        print_error "Timeout waiting for validators"
        exit 1
    fi
    
    spinner "Waiting for validators... (${HEALTHY}/${NUM_VALIDATORS} healthy, ${elapsed}s)"
    sleep 0.15
done

start_time=$(date +%s)
timeout=120

while true; do
    SECONDARY_HEALTH=$(docker compose -f "$COMPOSE_FILE" ps --format json 2>/dev/null | \
        jq -r 'select(.Service == "secondary-node0") | .Health' 2>/dev/null || echo "unknown")

    elapsed=$(($(date +%s) - start_time))

    if [[ "$SECONDARY_HEALTH" == "healthy" ]]; then
        clear_line
        print_success "Secondary peer healthy"
        break
    fi

    if [[ "$elapsed" -ge "$timeout" ]]; then
        clear_line
        print_error "Timeout waiting for secondary peer"
        exit 1
    fi

    spinner "Waiting for secondary peer... (${SECONDARY_HEALTH:-unknown}, ${elapsed}s)"
    sleep 0.15
done

echo ""

# Phase 3: Ready
print_phase "3/3" "Devnet ready"

echo ""
echo -e "  ${GREEN}┌────────────┬────────────┬─────────┐${NC}"
echo -e "  ${GREEN}│${NC} ${BOLD}Node${NC}       ${GREEN}│${NC} ${BOLD}Status${NC}     ${GREEN}│${NC} ${BOLD}Port${NC}    ${GREEN}│${NC}"
echo -e "  ${GREEN}├────────────┼────────────┼─────────┤${NC}"

for ((i = 0; i < NUM_VALIDATORS; i++)); do
    status=$(docker compose -f "$COMPOSE_FILE" ps --format json 2>/dev/null | \
        jq -r "select(.Service == \"validator-node$i\") | .Health" 2>/dev/null || echo "unknown")
    
    if [[ "$status" == "healthy" ]]; then
        status_str="${GREEN}healthy${NC}    "
    else
        status_str="${YELLOW}${status}${NC}"
    fi
    
    p2p_port=$((30400 + i))
    printf "  ${GREEN}│${NC} node%-6s ${GREEN}│${NC} %b ${GREEN}│${NC} %-7s ${GREEN}│${NC}\n" "$i" "$status_str" "$p2p_port"
done

secondary_status=$(docker compose -f "$COMPOSE_FILE" ps --format json 2>/dev/null | \
    jq -r 'select(.Service == "secondary-node0") | .Health' 2>/dev/null || echo "unknown")

if [[ "$secondary_status" == "healthy" ]]; then
    secondary_status_str="${GREEN}healthy${NC}    "
else
    secondary_status_str="${YELLOW}${secondary_status}${NC}"
fi

printf "  ${GREEN}│${NC} secondary0 ${GREEN}│${NC} %b ${GREEN}│${NC} 30500   ${GREEN}│${NC}\n" "$secondary_status_str"

echo -e "  ${GREEN}└────────────┴────────────┴─────────┘${NC}"

print_endpoints
