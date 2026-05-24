#!/usr/bin/env bash
set -eo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

REFRESH_INTERVAL=${1:-0.3}
CHAIN_ID="${CHAIN_ID:-1337}"
COMPOSE_PROJECT="${COMPOSE_PROJECT_NAME:-kora-devnet}"
COMPOSE_FILE="${DEVNET_COMPOSE_FILE:-compose/devnet.yaml}"
RPC_TIMEOUT="${RPC_TIMEOUT:-0.3}"
NODE_STATUS_PAYLOAD='{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}'
FOLLOWER_SERVICE="secondary-node0"
FOLLOWER_P2P_PORT="-"
VALIDATOR_COUNT=0
CONSENSUS_THRESHOLD=0
declare -a NODE_IDS=()
declare -a SERVICE_NAMES=()
declare -a CONTAINER_NAMES=()
declare -a RPC_PORTS=()
declare -a P2P_PORTS=()
declare -a METRICS_PORTS=()
declare -a PREV_FINALIZED=()
declare -a PREV_SAMPLE_MS=()

# Portable millisecond timestamp (macOS date lacks %N)
millis() {
    if perl -MTime::HiRes=time -e 'printf "%d\n", time()*1000' 2>/dev/null; then
        return
    elif python3 -c 'import time; print(int(time.time()*1000))' 2>/dev/null; then
        return
    else
        # Fallback: second-precision (loses sub-second accuracy for blocks/s)
        echo "$(date +%s)000"
    fi
}

cleanup() {
    tput cnorm
    echo ""
    exit 0
}
trap cleanup INT TERM

format_uptime() {
    local s=$1
    if [[ $s -ge 86400 ]]; then printf "%dd%dh" $((s/86400)) $((s%86400/3600))
    elif [[ $s -ge 3600 ]]; then printf "%dh%dm" $((s/3600)) $((s%3600/60))
    elif [[ $s -ge 60 ]]; then printf "%dm%ds" $((s/60)) $((s%60))
    else printf "%ds" $s; fi
}

compose_ps_json() {
    docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" ps -a --format json 2>/dev/null | \
        jq -s 'map(if type == "array" then .[] else . end)' 2>/dev/null || echo "[]"
}

discover_topology() {
    local ps_json rows idx service container rpc p2p metrics

    ps_json=$(compose_ps_json)
    NODE_IDS=()
    SERVICE_NAMES=()
    CONTAINER_NAMES=()
    RPC_PORTS=()
    P2P_PORTS=()
    METRICS_PORTS=()

    rows=$(echo "$ps_json" | jq -r '
        .[]
        | select(.Service | test("^validator-node[0-9]+$"))
        | (.Service | capture("validator-node(?<idx>[0-9]+)$").idx | tonumber) as $idx
        | [
            $idx,
            .Service,
            (.Name // .Names // "-"),
            (([.Publishers[]? | select(.TargetPort == 8545 and .PublishedPort != 0) | .PublishedPort] | unique | first) // "-"),
            (([.Publishers[]? | select(.TargetPort == 30303 and .PublishedPort != 0) | .PublishedPort] | unique | first) // "-"),
            (([.Publishers[]? | select(.TargetPort == 9002 and .PublishedPort != 0) | .PublishedPort] | unique | first) // "-")
        ] | @tsv
    ' 2>/dev/null | sort -n)

    while IFS=$'\t' read -r idx service container rpc p2p metrics; do
        [[ -n "${idx:-}" ]] || continue
        NODE_IDS+=("$idx")
        SERVICE_NAMES+=("${service:-validator-node$idx}")
        CONTAINER_NAMES+=("${container:-"-"}")
        RPC_PORTS+=("${rpc:-"-"}")
        P2P_PORTS+=("${p2p:-"-"}")
        METRICS_PORTS+=("${metrics:-"-"}")
    done <<< "$rows"

    VALIDATOR_COUNT=${#NODE_IDS[@]}
    if [[ "$VALIDATOR_COUNT" -gt 0 ]]; then
        CONSENSUS_THRESHOLD=$((VALIDATOR_COUNT - (VALIDATOR_COUNT - 1) / 3))
    else
        CONSENSUS_THRESHOLD=0
    fi

    FOLLOWER_P2P_PORT=$(echo "$ps_json" | jq -r "
        .[]
        | select(.Service == \"$FOLLOWER_SERVICE\")
        | (([.Publishers[]? | select(.TargetPort == 30303 and .PublishedPort != 0) | .PublishedPort] | unique | first) // \"-\")
    " 2>/dev/null | head -n 1)
    FOLLOWER_P2P_PORT="${FOLLOWER_P2P_PORT:-"-"}"
}

format_ports() {
    local ports=("$@")
    local sorted=()
    local port

    for port in "${ports[@]}"; do
        [[ "$port" =~ ^[0-9]+$ ]] && sorted+=("$port")
    done

    if [[ ${#sorted[@]} -eq 0 ]]; then
        printf "none"
        return
    fi

    mapfile -t sorted < <(printf "%s\n" "${sorted[@]}" | sort -n)

    local start="${sorted[0]}"
    local prev="${sorted[0]}"
    local out=""
    for port in "${sorted[@]:1}"; do
        if [[ "$port" -eq $((prev + 1)) ]]; then
            prev="$port"
            continue
        fi

        if [[ -n "$out" ]]; then
            out+=","
        fi
        if [[ "$start" == "$prev" ]]; then
            out+="$start"
        else
            out+="${start}-${prev}"
        fi
        start="$port"
        prev="$port"
    done

    if [[ -n "$out" ]]; then
        out+=","
    fi
    if [[ "$start" == "$prev" ]]; then
        out+="$start"
    else
        out+="${start}-${prev}"
    fi

    printf "%s" "$out"
}

query_node_status() {
    local port=$1
    local container=$2
    local status=""

    if [[ "$port" =~ ^[0-9]+$ ]]; then
        status=$(curl -s --max-time "$RPC_TIMEOUT" -X POST -H "Content-Type: application/json" \
            -d "$NODE_STATUS_PAYLOAD" "http://127.0.0.1:${port}" 2>/dev/null | \
            jq -c '.result // empty' 2>/dev/null || true)
    fi

    if [[ -z "$status" && -n "$container" && "$container" != "-" ]]; then
        status=$(docker exec "$container" sh -lc \
            "curl -s --max-time $RPC_TIMEOUT -X POST -H 'Content-Type: application/json' -d '$NODE_STATUS_PAYLOAD' http://127.0.0.1:8545" 2>/dev/null | \
            jq -c '.result // empty' 2>/dev/null || true)
    fi

    [[ -n "$status" ]] || status="{}"
    printf "%s\n" "$status"
}

# Fetch all node statuses in parallel
fetch_all_statuses() {
    local tmpdir=$(mktemp -d)
    local row idx port container
    
    # Launch parallel fetches using JSON-RPC POST to get node status.
    for row in "${!NODE_IDS[@]}"; do
        idx="${NODE_IDS[$row]}"
        port="${RPC_PORTS[$row]}"
        container="${CONTAINER_NAMES[$row]}"
        (
            query_node_status "$port" "$container" > "$tmpdir/$idx"
        ) &
    done
    wait
    
    # Read results
    for row in "${!NODE_IDS[@]}"; do
        idx="${NODE_IDS[$row]}"
        if [[ -f "$tmpdir/$idx" ]]; then
            cat "$tmpdir/$idx"
        else
            echo "{}"
        fi
        echo  # newline separator
    done
    
    rm -rf "$tmpdir"
}

fetch_follower_info() {
    compose_ps_json | jq -r ".[] | select(.Service == \"$FOLLOWER_SERVICE\") | [
            .Health // .State // \"unknown\",
            .State // \"unknown\",
            (.RunningFor // \"-\"),
            ([.Publishers[]? | select(.TargetPort == 30303 and .PublishedPort != 0) | .PublishedPort] | unique | join(\",\")),
            .Name // \"$FOLLOWER_SERVICE\"
        ] | @tsv" 2>/dev/null || true
}

render() {
    discover_topology
    tput cup 0 0
    local now=$(date "+%H:%M:%S")
    
    echo -e "${BOLD}${BLUE}╔══════════════════════════════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${BOLD}${BLUE}║${NC}                              ${BOLD}KORA DEVNET MONITOR${NC}                                        ${BOLD}${BLUE}║${NC}"
    echo -e "${BOLD}${BLUE}╚══════════════════════════════════════════════════════════════════════════════════════════╝${NC}"
    echo -e "  ${DIM}$now${NC}  │  ${DIM}Chain:${NC} ${CYAN}$CHAIN_ID${NC}  │  ${DIM}Refresh:${NC} ${REFRESH_INTERVAL}s  │  ${DIM}Ctrl+C to exit${NC}"
    echo ""

    if [[ "$VALIDATOR_COUNT" -eq 0 ]]; then
        echo -e "${YELLOW}No validator containers found for Compose project '${COMPOSE_PROJECT}'.${NC}"
        echo ""
        echo -e "${DIM}Start a devnet with 'just devnet' or 'just trusted-devnet'.${NC}"
        for _ in {1..18}; do
            printf "%-90s\n" ""
        done
        return
    fi
    
    echo -e "${BOLD}${CYAN}Node Status${NC}"
    echo -e "┌───────┬──────────┬────────────┬──────────┬────────────┬────────────┬────────────┬────────────┬────────┐"
    echo -e "│ ${BOLD}Node${NC}  │ ${BOLD}Status${NC}   │ ${BOLD}Uptime${NC}     │ ${BOLD}View${NC}     │ ${BOLD}Finalized${NC}  │ ${BOLD}Nullified${NC}  │ ${BOLD}Proposed${NC}   │ ${BOLD}Blocks/s${NC}   │ ${BOLD}Leader${NC} │"
    echo -e "├───────┼──────────┼────────────┼──────────┼────────────┼────────────┼────────────┼────────────┼────────┤"
    
    local rpc_count=0
    local healthy_count=0
    local stalled_count=0
    local max_uptime=0
    local total_finalized=0
    local max_view=0
    local max_blocks_per_sec=0
    local follower_status="offline"
    local follower_color=$RED
    local follower_state="-"
    local follower_uptime="-"
    local follower_p2p="$FOLLOWER_P2P_PORT"
    local follower_container="$FOLLOWER_SERVICE"
    
    # Fetch all statuses in parallel
    local all_status
    all_status=$(fetch_all_statuses)
    local sample_ms
    sample_ms=$(millis)
    
    local row=0
    while IFS= read -r status; do
        # Skip empty lines (separators between node outputs)
        [[ -z "$status" ]] && continue
        local node_id="${NODE_IDS[$row]:-$row}"
        
        if [[ "$status" != "{}" ]]; then
            # Parse with single jq call
            local parsed
            parsed=$(echo "$status" | jq -r '[.validatorIndex // .validator_index // empty, .uptimeSecs // .uptime_secs // 0, .currentView // .current_view // 0, .finalizedCount // .finalized_count // 0, .nullifiedCount // .nullified_count // 0, .proposedCount // .proposed_count // 0, .isLeader // .is_leader // false] | @tsv' 2>/dev/null)
            
            if [[ -n "$parsed" ]]; then
                read -r validator_index uptime view finalized nullified proposed leader <<< "$parsed"
                
                validator_index="${validator_index:-$node_id}"
                uptime="${uptime:-0}"
                view="${view:-0}"
                finalized="${finalized:-0}"
                nullified="${nullified:-0}"
                proposed="${proposed:-0}"
                
                [[ $uptime -gt $max_uptime ]] && max_uptime=$uptime
                [[ $view -gt $max_view ]] && max_view=$view
                [[ $finalized -gt $total_finalized ]] && total_finalized=$finalized
                ((++rpc_count))
                
                local uptime_str=$(format_uptime "$uptime")
                local leader_str="-"
                [[ "$leader" == "true" ]] && leader_str="${MAGENTA}★${NC}"
                local rpc_status="${GREEN}online${NC} "
                if [[ $view -eq 0 && $finalized -eq 0 && $proposed -eq 0 && $uptime -gt 10 ]]; then
                    rpc_status="${YELLOW}stalled${NC}"
                    ((++stalled_count))
                else
                    ((++healthy_count))
                fi
                
                # Calculate live finalized blocks per second since the previous refresh.
                local blocks_per_sec_str="-"
                if [[ -n "${PREV_FINALIZED[$node_id]:-}" && -n "${PREV_SAMPLE_MS[$node_id]:-}" ]]; then
                    local delta_blocks=$((finalized - PREV_FINALIZED[$node_id]))
                    local delta_ms=$((sample_ms - PREV_SAMPLE_MS[$node_id]))
                    if [[ $delta_blocks -ge 0 && $delta_ms -gt 0 ]]; then
                        local blocks_per_sec
                        blocks_per_sec=$(awk -v blocks="$delta_blocks" -v ms="$delta_ms" 'BEGIN {printf "%.2f", blocks * 1000 / ms}')
                        blocks_per_sec_str="${blocks_per_sec} b/s"
                        local blocks_per_sec_int
                        blocks_per_sec_int=$(awk -v blocks="$delta_blocks" -v ms="$delta_ms" 'BEGIN {printf "%d", blocks * 100000 / ms}')
                        [[ $blocks_per_sec_int -gt $max_blocks_per_sec ]] && max_blocks_per_sec=$blocks_per_sec_int
                    fi
                fi
                PREV_FINALIZED[$node_id]=$finalized
                PREV_SAMPLE_MS[$node_id]=$sample_ms
                
                printf "│ ${CYAN}%-5s${NC} │ %b │ %-10s │ %-8s │ %-10s │ %-10s │ %-10s │ %-10s │   %b    │\n" \
                    "$node_id" "$rpc_status" "$uptime_str" "$view" "$finalized" "$nullified" "$proposed" "$blocks_per_sec_str" "$leader_str"
            else
                unset "PREV_FINALIZED[$node_id]" "PREV_SAMPLE_MS[$node_id]"
                printf "│ ${CYAN}%-5s${NC} │ ${RED}offline${NC}  │ -          │ -        │ -          │ -          │ -          │ -          │   -    │\n" "$node_id"
            fi
        else
            unset "PREV_FINALIZED[$node_id]" "PREV_SAMPLE_MS[$node_id]"
            printf "│ ${CYAN}%-5s${NC} │ ${RED}offline${NC}  │ -          │ -        │ -          │ -          │ -          │ -          │   -    │\n" "$node_id"
        fi
        ((++row))
    done <<< "$all_status"

    local follower_info
    follower_info=$(fetch_follower_info)
    if [[ -n "$follower_info" ]]; then
        local follower_health_value
        IFS=$'\t' read -r follower_health_value follower_state follower_uptime follower_p2p follower_container <<< "$follower_info"
        follower_uptime="${follower_uptime% ago}"
        follower_p2p="${follower_p2p:-$FOLLOWER_P2P_PORT}"

        case "$follower_health_value" in
            healthy)
                follower_status="healthy"
                follower_color=$GREEN
                ;;
            running)
                follower_status="running"
                follower_color=$GREEN
                ;;
            starting)
                follower_status="starting"
                follower_color=$YELLOW
                ;;
            *)
                follower_status="${follower_health_value:-${follower_state:-unknown}}"
                follower_color=$YELLOW
                ;;
        esac
    fi

    local follower_table_uptime="${follower_uptime:0:10}"
    local follower_network="P2P ${follower_p2p:-none}"
    printf "│ ${CYAN}%-5s${NC} │ ${follower_color}%-8s${NC} │ %-10s │ %-8s │ %-10s │ %-10s │ %-10s │ %-10s │   -    │\n" \
        "f0" "$follower_status" "$follower_table_uptime" "follower" "-" "-" "-" "$follower_network"
    
    echo -e "└───────┴──────────┴────────────┴──────────┴────────────┴────────────┴────────────┴────────────┴────────┘"
    
    # Summary
    echo ""
    echo -e "${BOLD}${CYAN}Summary${NC}"
    
    local health_color=$GREEN
    [[ $healthy_count -lt "$VALIDATOR_COUNT" ]] && health_color=$YELLOW
    [[ $healthy_count -lt "$CONSENSUS_THRESHOLD" ]] && health_color=$RED
    
    local threshold="${GREEN}✓ Met${NC}"
    [[ $healthy_count -lt "$CONSENSUS_THRESHOLD" ]] && threshold="${RED}✗ Not met${NC}"
    
    local uptime_str="0s"
    [[ $max_uptime -gt 0 ]] && uptime_str=$(format_uptime "$max_uptime")
    
    # Format live blocks/sec from stored integer (x100)
    local blocks_per_sec_str="0.00 b/s"
    if [[ $max_blocks_per_sec -gt 0 ]]; then
        blocks_per_sec_str=$(awk -v bps="$max_blocks_per_sec" 'BEGIN {printf "%.2f b/s", bps / 100}')
    fi
    
    echo -e "  ${DIM}Consensus:${NC} ${health_color}${healthy_count}/${VALIDATOR_COUNT}${NC}  │  ${DIM}RPC:${NC} ${GREEN}${rpc_count}/${VALIDATOR_COUNT}${NC}  │  ${DIM}Follower:${NC} ${follower_color}${follower_status}${NC}  │  ${DIM}Stalled:${NC} ${YELLOW}${stalled_count}${NC}  │  ${DIM}Threshold:${NC} $threshold ${DIM}(${CONSENSUS_THRESHOLD}/${VALIDATOR_COUNT})${NC}  │  ${DIM}View:${NC} ${CYAN}$max_view${NC}  │  ${DIM}Finalized:${NC} ${GREEN}$total_finalized${NC}  │  ${DIM}Blocks/s:${NC} ${CYAN}$blocks_per_sec_str${NC}  │  ${DIM}Uptime:${NC} $uptime_str"

    echo ""
    echo -e "${BOLD}${CYAN}Follower Node${NC}"
    echo -e "  ${DIM}Node:${NC} ${CYAN}f0${NC}  │  ${DIM}Role:${NC} secondary  │  ${DIM}Service:${NC} $FOLLOWER_SERVICE  │  ${DIM}Container:${NC} $follower_container"
    echo -e "  ${DIM}Health:${NC} ${follower_color}${follower_status}${NC}  │  ${DIM}State:${NC} $follower_state  │  ${DIM}Uptime:${NC} $follower_uptime  │  ${DIM}P2P:${NC} ${follower_p2p:-none}  │  ${DIM}RPC:${NC} none"
    
    # Endpoints
    echo ""
    echo -e "${BOLD}${CYAN}Endpoints${NC}"
    echo -e "  ${DIM}P2P:${NC} $(format_ports "${P2P_PORTS[@]}")    ${DIM}Follower P2P:${NC} $FOLLOWER_P2P_PORT    ${DIM}RPC:${NC} $(format_ports "${RPC_PORTS[@]}")    ${DIM}Metrics:${NC} $(format_ports "${METRICS_PORTS[@]}")"
    
    # Clear extra lines
    for _ in {1..5}; do
        printf "%-90s\n" ""
    done
}

# Main
clear
tput civis

echo -e "${DIM}Connecting to RPC endpoints...${NC}"
sleep 0.2

render

while true; do
    sleep "$REFRESH_INTERVAL"
    render
done
