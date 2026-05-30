# ═══════════════════════════════════════════════════════════════════════════
# Kora Node Docker Image
# Multi-stage build with cargo-chef for optimal layer caching
# ═══════════════════════════════════════════════════════════════════════════

# ───────────────────────────────────────────────────────────────────────────
# Stage 1: Chef - Install cargo-chef for dependency caching
# ───────────────────────────────────────────────────────────────────────────
FROM rustlang/rust:nightly-bookworm AS chef

RUN cargo install cargo-chef --locked
WORKDIR /app

# ───────────────────────────────────────────────────────────────────────────
# Stage 2: Planner - Analyze dependencies and create recipe
# ───────────────────────────────────────────────────────────────────────────
FROM chef AS planner

COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ───────────────────────────────────────────────────────────────────────────
# Stage 3: Builder - Build dependencies (cached) then application
# ───────────────────────────────────────────────────────────────────────────
FROM chef AS builder

# Build profile: "release" (optimized) or "dev" (fast, with debug symbols).
# Passed from docker-bake.hcl targets.
ARG BUILD_PROFILE=release

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy recipe and build dependencies (this layer is cached)
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --profile ${BUILD_PROFILE} --recipe-path recipe.json

# Copy source and build application
COPY . .

# Build only production binaries (kora + keygen).
# loadgen is a load-testing tool and should not ship in production images
# to reduce attack surface (see issue #190).
RUN cargo build --profile ${BUILD_PROFILE} -p kora -p keygen

# Cargo outputs to target/debug for --profile dev, target/release for
# --profile release.  Create a stable symlink so the COPY stage does not
# need conditional logic.
RUN if [ "${BUILD_PROFILE}" = "dev" ]; then \
      ln -s /app/target/debug /app/target/profile-out; \
    else \
      ln -s /app/target/${BUILD_PROFILE} /app/target/profile-out; \
    fi

# ───────────────────────────────────────────────────────────────────────────
# Stage 4: Runtime - Minimal production image
# ───────────────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    netcat-openbsd \
    curl \
    jq \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user for security
RUN useradd -m -u 1000 -s /bin/bash kora && \
    mkdir -p /var/lib/kora /etc/kora /data /shared /runtime && \
    chown -R kora:kora /var/lib/kora /etc/kora /data /shared /runtime

# Copy binaries from builder (profile-out is a symlink to the correct
# target/{release,debug} directory based on BUILD_PROFILE)
COPY --from=builder /app/target/profile-out/kora /usr/local/bin/
COPY --from=builder /app/target/profile-out/keygen /usr/local/bin/

# Copy entrypoint scripts
COPY docker/scripts/ /scripts/
RUN chmod +x /scripts/*.sh

# Switch to non-root user
USER kora
WORKDIR /data

# Expose ports
# 30303 - P2P networking
# 8545  - HTTP JSON-RPC (reserved for Phase 3)
# 8546  - WebSocket JSON-RPC (reserved for Phase 3)
# 9002  - Prometheus metrics
EXPOSE 30303 8545 8546 9002

# Default volumes
VOLUME ["/data", "/shared"]

# Runtime health check shared with Compose. Override HEALTHCHECK_MODE to
# switch the check: dkg (share.key + output.json exist), p2p (port 30303),
# ready (eth_blockNumber + stall detection). Default mode is p2p.
# The compose file overrides these timings; these are conservative defaults.
HEALTHCHECK --interval=30s --timeout=10s --retries=6 --start-period=120s \
    CMD /scripts/healthcheck.sh

# Default entrypoint - can be overridden for different modes
ENTRYPOINT ["/scripts/entrypoint.sh"]
CMD ["validator"]
