#!/usr/bin/env bash
set -euo pipefail

NUM_VALIDATORS="${1:?usage: generate-devnet-compose.sh <num_validators>}"

if ! [[ "$NUM_VALIDATORS" =~ ^[0-9]+$ ]] || [[ "$NUM_VALIDATORS" -lt 1 ]]; then
    echo "num_validators must be a positive integer" >&2
    exit 1
fi

THRESHOLD=$((NUM_VALIDATORS - (NUM_VALIDATORS - 1) / 3))
SECONDARY_RPC_PORT=$((8545 + NUM_VALIDATORS))
SECONDARY_METRICS_PORT=$((9000 + NUM_VALIDATORS))

peer_nodes=""
chown_paths=""
init_data_volumes=""
init_setup_mounts=""
init_config_mounts=""
runtime_volumes=""

for ((i = 0; i < NUM_VALIDATORS; i++)); do
    [[ -n "$peer_nodes" ]] && peer_nodes+=","
    peer_nodes+="node${i}"
    chown_paths+=" /shared/node${i}"
    init_data_volumes+="  data_node${i}:"$'\n'
    runtime_volumes+="  runtime_node${i}:"$'\n'
    init_setup_mounts+="      - data_node${i}:/shared/node${i}"$'\n'
    init_config_mounts+="      - data_node${i}:/shared/node${i}"$'\n'
done

cat <<EOF
name: kora-devnet

networks:
  kora-net:
    driver: bridge

volumes:
${init_data_volumes}  data_secondary0:
${runtime_volumes}  runtime_secondary0:
  shared_config:
  startup_barrier:
  prometheus_data:
  grafana_data:
  loki_data:

x-node-common: &node-common
  image: kora:local
  networks:
    - kora-net
  logging:
    driver: json-file
    options:
      max-size: "50m"
      max-file: "5"
  environment:
    - RUST_LOG=\${RUST_LOG:-info}
    - CHAIN_ID=\${CHAIN_ID:-1337}

x-validator-common: &validator-common
  <<: *node-common
  restart: unless-stopped
  init: true
  stop_grace_period: 5s
  stop_signal: SIGTERM
  read_only: true
  security_opt:
    - no-new-privileges:true
  cap_drop:
    - ALL
  ulimits:
    nofile:
      soft: 65536
      hard: 65536
    core: 0
  deploy:
    resources:
      limits:
        memory: 4G
        cpus: "2"
        pids: 4096
  tmpfs:
    - /tmp:size=64m,mode=0700
  healthcheck:
    test: ["CMD", "/scripts/healthcheck.sh"]
    interval: 10s
    timeout: 5s
    retries: 3
    start_period: 30s
  environment:
    - RUST_LOG=\${RUST_LOG:-info}
    - CHAIN_ID=\${CHAIN_ID:-1337}
    - KORA_RUNTIME_DIR=\${KORA_RUNTIME_DIR:-/runtime}
    - KORA_CHECKPOINT_INTERVAL=\${KORA_CHECKPOINT_INTERVAL:-256}
    - TX_GOSSIP=\${TX_GOSSIP:-true}
    - HEALTHCHECK_MODE=ready

services:
  init-setup:
    <<: *node-common
    user: root
    entrypoint: ["/bin/bash", "-c"]
    command:
      - |
        echo "[init] Running keygen setup..." && \\
        /usr/local/bin/keygen setup \\
          --validators=${NUM_VALIDATORS} \\
          --secondary-peers=1 \\
          --threshold=${THRESHOLD} \\
          --chain-id=\${CHAIN_ID:-1337} \\
          --output-dir=/shared && \\
        echo "[init] Setting permissions..." && \\
        chown -R 1000:1000${chown_paths} /shared/secondary0 && \\
        echo "[init] Setup complete (run DKG ceremony next)"
    volumes:
      - shared_config:/shared
${init_setup_mounts}      - data_secondary0:/shared/secondary0

  init-config:
    <<: *node-common
    user: root
    entrypoint: ["/bin/bash", "-c"]
    command:
      - |
        if [ -f /shared/node0/share.key ] && [ -f /shared/node0/output.json ]; then
            echo "[init] DKG already completed, skipping"
            echo "[init] Preparing startup barrier..." && \\
            rm -f /barrier/*.ready && \\
            chown -R 1000:1000 /barrier && \\
            exit 0
        fi
        echo "[init] Clearing startup barrier from previous runs..." && \\
        rm -f /barrier/*.ready && \\
        echo "[init] Running keygen setup..." && \\
        /usr/local/bin/keygen setup \\
          --validators=${NUM_VALIDATORS} \\
          --secondary-peers=1 \\
          --threshold=${THRESHOLD} \\
          --chain-id=\${CHAIN_ID:-1337} \\
          --output-dir=/shared && \\
        echo "[init] Running trusted dealer DKG..." && \\
        /usr/local/bin/keygen dkg-deal \\
          --validators=${NUM_VALIDATORS} \\
          --threshold=${THRESHOLD} \\
          --output-dir=/shared && \\
        echo "[init] Setting permissions..." && \\
        chown -R 1000:1000${chown_paths} /shared/secondary0 /barrier && \\
        echo "[init] Init complete"
    volumes:
      - shared_config:/shared
${init_config_mounts}      - data_secondary0:/shared/secondary0
      - startup_barrier:/barrier
EOF

for ((i = 0; i < NUM_VALIDATORS; i++)); do
    dkg_port=$((30300 + i))
    cat <<EOF

  dkg-node${i}:
    <<: *node-common
    profiles: ["interactive-dkg"]
    hostname: node${i}
    depends_on:
      init-setup:
        condition: service_completed_successfully
EOF
    if [[ "$i" -gt 0 ]]; then
        cat <<EOF
      dkg-node0:
        condition: service_started
EOF
    fi
    cat <<EOF
    entrypoint: ["/scripts/entrypoint.sh", "dkg"]
    volumes:
      - shared_config:/shared:ro
      - data_node${i}:/data
    environment:
      - RUST_LOG=\${RUST_LOG:-info}
      - CHAIN_ID=\${CHAIN_ID:-1337}
      - VALIDATOR_INDEX=${i}
EOF
    if [[ "$i" -eq 0 ]]; then
        cat <<EOF
      - IS_BOOTSTRAP=true
      - IS_LEADER=true
EOF
    else
        cat <<EOF
      - IS_BOOTSTRAP=false
      - BOOTSTRAP_PEERS=node0:30303
EOF
    fi
    cat <<EOF
    ports:
      - "${dkg_port}:30303"
EOF
done

for ((i = 0; i < NUM_VALIDATORS; i++)); do
    p2p_port=$((30400 + i))
    rpc_port=$((8545 + i))
    metrics_port=$((9000 + i))
    cat <<EOF

  validator-node${i}:
    <<: *validator-common
    hostname: node${i}
    depends_on:
      init-config:
        condition: service_completed_successfully
    entrypoint: ["/scripts/entrypoint.sh", "validator"]
    volumes:
      - shared_config:/shared:ro
      - data_node${i}:/data
      - runtime_node${i}:/runtime
      - startup_barrier:/barrier
    environment:
      - RUST_LOG=\${RUST_LOG:-info}
      - CHAIN_ID=\${CHAIN_ID:-1337}
      - KORA_RUNTIME_DIR=\${KORA_RUNTIME_DIR:-/runtime}
      - KORA_CHECKPOINT_INTERVAL=\${KORA_CHECKPOINT_INTERVAL:-256}
      - KORA_BOOTSTRAP_TOPOLOGY=\${KORA_BOOTSTRAP_TOPOLOGY:-lower}
      - VALIDATOR_INDEX=${i}
      - VALIDATOR_COUNT=${NUM_VALIDATORS}
EOF
    if [[ "$i" -eq 0 ]]; then
        cat <<EOF
      - IS_BOOTSTRAP=true
      - PEER_NODES=${peer_nodes}
      - HEALTHCHECK_MODE=ready
EOF
    else
        cat <<EOF
      - IS_BOOTSTRAP=false
      - BOOTSTRAP_PEERS=node0:30303
      - PEER_NODES=${peer_nodes}
      - HEALTHCHECK_MODE=ready
EOF
    fi
    cat <<EOF
    ports:
      - "${p2p_port}:30303"
      - "127.0.0.1:${rpc_port}:8545"
      - "127.0.0.1:${metrics_port}:9002"
EOF
done

cat <<EOF

  secondary-node0:
    <<: *validator-common
    hostname: secondary0
    depends_on:
      validator-node0:
        condition: service_healthy
    entrypoint: ["/scripts/entrypoint.sh", "secondary"]
    volumes:
      - shared_config:/shared:ro
      - data_secondary0:/data
      - runtime_secondary0:/runtime
    environment:
      - RUST_LOG=\${RUST_LOG:-info}
      - CHAIN_ID=\${CHAIN_ID:-1337}
      - KORA_RUNTIME_DIR=\${KORA_RUNTIME_DIR:-/runtime}
      - KORA_CHECKPOINT_INTERVAL=\${KORA_CHECKPOINT_INTERVAL:-256}
      - IS_BOOTSTRAP=false
      - BOOTSTRAP_PEERS=node0:30303
      - HEALTHCHECK_MODE=p2p
    ports:
      - "30500:30303"
      - "127.0.0.1:${SECONDARY_RPC_PORT}:8545"
      - "127.0.0.1:${SECONDARY_METRICS_PORT}:9002"
EOF

cat <<'EOF'

  prometheus:
    image: prom/prometheus:latest
    profiles: ["observability"]
    restart: unless-stopped
    read_only: true
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    tmpfs:
      - /tmp:size=64m,mode=0700
    volumes:
      - prometheus_data:/prometheus
      - ../config/prometheus.yml:/etc/prometheus/prometheus.yml:ro
      - ../config/alerts.yml:/etc/prometheus/alerts.yml:ro
      - ../config/recording-rules.yml:/etc/prometheus/recording-rules.yml:ro
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'
      - '--storage.tsdb.path=/prometheus'
      - '--web.enable-lifecycle'
    ports:
      - "127.0.0.1:9090:9090"
    networks:
      - kora-net

  loki:
    image: grafana/loki:3.4.2
    profiles: ["observability"]
    restart: unless-stopped
    read_only: true
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    tmpfs:
      - /tmp:size=64m,mode=0700
    volumes:
      - loki_data:/loki
      - ../config/loki.yml:/etc/loki/local-config.yaml:ro
    command: -config.file=/etc/loki/local-config.yaml
    ports:
      - "127.0.0.1:3100:3100"
    networks:
      - kora-net

  promtail:
    image: grafana/promtail:3.4.2
    profiles: ["observability"]
    restart: unless-stopped
    read_only: true
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    tmpfs:
      - /tmp:size=64m,mode=0700
    depends_on:
      - loki
    volumes:
      - ../config/promtail.yml:/etc/promtail/config.yml:ro
      - /var/run/docker.sock:/var/run/docker.sock:ro
    command: -config.file=/etc/promtail/config.yml
    networks:
      - kora-net

  grafana:
    image: grafana/grafana:latest
    profiles: ["observability"]
    restart: unless-stopped
    read_only: true
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    tmpfs:
      - /tmp:size=64m,mode=0700
    depends_on:
      - prometheus
      - loki
    volumes:
      - grafana_data:/var/lib/grafana
      - ../grafana/provisioning:/etc/grafana/provisioning:ro
      - ../grafana/dashboards:/var/lib/grafana/dashboards:ro
    environment:
      - GF_SECURITY_ADMIN_USER=admin
      - GF_SECURITY_ADMIN_PASSWORD=${GF_SECURITY_ADMIN_PASSWORD:-admin}
      - GF_AUTH_ANONYMOUS_ENABLED=true
      - GF_AUTH_ANONYMOUS_ORG_ROLE=Viewer
      - GF_LOG_MODE=console
    ports:
      - "127.0.0.1:3000:3000"
    networks:
      - kora-net
EOF
