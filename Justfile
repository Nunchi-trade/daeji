# List available recipes
default:
    @just --list

# Run the full CI suite
ci: fmt clippy test deny

# Run all checks
check: fmt clippy test

# Run tests
test:
    cargo nextest run --workspace --all-features

# Build in release mode
build:
    cargo build --release

# Build all targets
build-all:
    cargo build --all-targets

# Check formatting
fmt:
    cargo +nightly fmt --all -- --check

# Fix formatting
fmt-fix:
    cargo +nightly fmt --all

# Run clippy
clippy:
    cargo clippy --all-targets --all-features -- -D warnings

# Run cargo deny
deny:
    .github/ensure-cargo-deny.sh
    cargo deny check

# Clean build artifacts
clean:
    cargo clean

# Start the devnet with interactive DKG (production-like)
devnet:
    cd docker && just devnet

# Start the devnet with trusted dealer DKG (fast, insecure, for local dev)
trusted-devnet:
    cd docker && just trusted-devnet

# Stop the devnet
devnet-down:
    cd docker && just down

# Reset devnet (clears all state, requires fresh DKG)
devnet-reset:
    cd docker && just reset

# View devnet logs
devnet-logs:
    cd docker && just logs

# View devnet status
devnet-status:
    cd docker && just status

# Live devnet monitoring dashboard
devnet-stats:
    cd docker && just stats

# Build docker images
docker-build:
    cd docker && just build

# Run load generator against devnet
loadgen *args:
    cargo run --release --bin loadgen -- {{args}}

# Quick load test (1000 txs)
loadtest:
    cargo run --release --bin loadgen -- --total-txs 1000

# Stress test (10000 txs with 50 accounts)
stresstest:
    cargo run --release --bin loadgen -- --total-txs 10000 --accounts 50

# Build the released commonware-chat example (vendored from commonwarexyz/monorepo@v2026.4.0
# at crates/network/commonware-chat-upstream).
chat-build:
    cargo build --release -p commonware-chat-upstream

# Released commonware-chat example — friend 1 (bootstrapper).
# Open four terminals and run chat-1 .. chat-4 to spin up the 4-node demo.
chat-1:
    cargo run --release -p commonware-chat-upstream --bin commonware-chat -- \
        --me=1@3001 --friends=1,2,3,4

# Released commonware-chat example — friend 2.
chat-2:
    cargo run --release -p commonware-chat-upstream --bin commonware-chat -- \
        --me=2@3002 --friends=1,2,3,4 --bootstrappers=1@127.0.0.1:3001

# Released commonware-chat example — friend 3.
chat-3:
    cargo run --release -p commonware-chat-upstream --bin commonware-chat -- \
        --me=3@3003 --friends=1,2,3,4 --bootstrappers=1@127.0.0.1:3001

# Released commonware-chat example — friend 4.
chat-4:
    cargo run --release -p commonware-chat-upstream --bin commonware-chat -- \
        --me=4@3004 --friends=1,2,3,4 --bootstrappers=3@127.0.0.1:3003
