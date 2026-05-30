# RPC Health Endpoint Always Returns HTTP 200 Regardless of Node State

## Category
bug/ops -- rpc

## Severity
medium

## Summary
The `/health` HTTP endpoint on the Kora RPC server is a static handler that always returns HTTP 200 with the body `"ok"`. It does not check whether the node has peers, whether consensus is progressing, or whether the node is catching up after recovery. Load balancers and orchestrators that rely on this endpoint will never detect an unhealthy node.

## Problem
Kora exposes two HTTP endpoints alongside the JSON-RPC server: `/status` (which returns a detailed `NodeStatus` JSON object including peer count, partition status, and sync state) and `/health` (which is intended as a binary healthy/unhealthy probe for load balancers and Kubernetes liveness checks).

The `/health` handler is defined as a no-argument function that unconditionally returns `StatusCode::OK`:

**File:** `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs`, lines 766-768

```rust
async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}
```

The handler does not receive the `NodeState` (which is available to the route as shared Axum state) and therefore cannot inspect any runtime conditions. The `NodeState` type (defined in `crates/node/rpc/src/state.rs`) already exposes methods such as `is_catching_up()`, `partition_status()`, `peer_count()`, and `finalized_height()` that would be suitable for health determination.

The route is wired into the Axum router at line 366:

```rust
Router::new()
    .route("/status", get(status_handler))
    .route("/health", get(health_handler))
    // ...
    .with_state(node_state)
```

Note that `node_state` is passed as `.with_state()` but the `health_handler` function signature does not extract it.

## Code Reference
`/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs`, lines 364-368 (route wiring):
```rust
Router::new()
    .route("/status", get(status_handler))
    .route("/health", get(health_handler))
    .layer(middleware::from_fn_with_state(rate_limiter, enforce_http_rate_limit))
    .layer(cors_layer)
```

`/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs`, lines 766-768 (handler):
```rust
async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}
```

For comparison, the `status_handler` at lines 761-764 does use node state:
```rust
async fn status_handler(State(state): State<Arc<NodeState>>) -> impl IntoResponse {
    let status = state.status();
    (StatusCode::OK, axum::Json(status))
}
```

## Impact
- **Load balancer routing:** A load balancer (HAProxy, nginx, ALB) will continue routing user traffic to a node that is network-partitioned, stalled, or catching up. Users will receive stale or incorrect state responses.
- **Kubernetes/Docker orchestration:** Liveness and readiness probes will never trigger container restarts or traffic rerouting, even when a node is completely non-functional. The 10-node devnet uses Docker Compose, and any health-check-based restart policy would be ineffective.
- **Monitoring blind spots:** Operators cannot distinguish a healthy node from a partitioned one using the health endpoint alone. They must parse the full JSON from `/status`, which is not standard for health check tooling.

## Root Cause
The health handler was implemented as a static response placeholder. It was never wired to the `NodeState` that is already available as Axum shared state on the same router.

## Suggested Fix
Accept the `NodeState` in the health handler and return HTTP 503 (Service Unavailable) when the node is in an unhealthy state:

```rust
async fn health_handler(State(state): State<Arc<NodeState>>) -> impl IntoResponse {
    let status = state.status();
    if status.partition_status == PartitionStatus::Partitioned {
        return (StatusCode::SERVICE_UNAVAILABLE, "partitioned");
    }
    if state.is_catching_up() {
        return (StatusCode::SERVICE_UNAVAILABLE, "syncing");
    }
    (StatusCode::OK, "ok")
}
```

No signature changes are needed for the route wiring since `NodeState` is already the Axum state type.

## Files to Modify
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs` -- change `health_handler` to accept and inspect `NodeState`

## Related Issues
- `133-rpc-net-peer-count-snapshot-never-updated.md` -- `net_peerCount` also returns stale data (always 0), making it another blind spot for connectivity monitoring

## Labels
bug, reliability, rpc, metrics, good first issue
