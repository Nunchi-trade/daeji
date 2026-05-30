# `eth_subscribe` over HTTP returns -32603 (Internal error) instead of -32601 (Method not found)

## Status

**Candidate for folding into a broader RPC compliance issue.** This is a cosmetic error-code mismatch with no functional impact. It could be addressed as a one-line fix or merged into Issue 07 (RPC compliance) rather than tracked separately.

## Summary

When a client calls `eth_subscribe` over plain HTTP (not WebSocket), jsonrpsee returns error code `-32603` ("Internal error") instead of `-32601` ("Method not found"). This is misleading because `-32603` implies a transient server-side failure, which may cause well-behaved clients to retry. The correct code is `-32601`, signaling that retrying is pointless on this transport.

**Important:** `eth_subscribe` IS fully implemented and functional over WebSocket. The subscription module in `subscription.rs` (lines 62-129) registers a working `eth_subscribe` handler via `register_subscription()`, and it is merged into the server in `server.rs` (lines 493-530). The issue is ONLY about the error code returned when this method is called over the HTTP transport.

## Root Cause

The jsonrpsee `Server::builder()` supports both HTTP and WebSocket on the same port. When `register_subscription()` is used, jsonrpsee adds `eth_subscribe` to the method registry as a subscription-only method. Over WebSocket, this works correctly. Over HTTP, jsonrpsee recognizes that the method exists but cannot establish a subscription on a stateless HTTP connection, so it produces a generic `-32603` ("Internal error") instead of `-32601` ("Method not found").

This is a jsonrpsee framework behavior, not a Kora application bug. The method is technically registered, so jsonrpsee does not consider it "not found" -- it just cannot serve it over HTTP, and the framework's fallback error code for this situation is `-32603`.

## Observed Behavior

From the RPC consistency test (all requests sent over HTTP):

```
eth_subscribe  | -32603 Internal error  | Should be -32601
```

For comparison, truly unregistered methods return the correct error:

```
eth_getBlockReceipts    | -32601 Method not found  | Correct
debug_traceTransaction  | -32601 Method not found  | Correct
```

The full error response for `eth_subscribe` over HTTP:
```json
{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"Internal error"}}
```

Expected:
```json
{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"eth_subscribe is not available over HTTP; use a WebSocket connection"}}
```

## Impact

Very low. This only affects clients that (a) call `eth_subscribe` over HTTP rather than WebSocket, AND (b) interpret error codes programmatically to decide on retry behavior. In practice, any client using `eth_subscribe` should already know to use a WebSocket connection.

- **Retry storms (theoretical)**: Clients interpreting `-32603` as transient may retry. `-32601` signals permanent unavailability.
- **Misleading diagnostics**: Operators seeing `-32603` may investigate server-side issues when the problem is simply wrong transport.

## Proposed Fix

This does NOT require changing the subscription module or how `eth_subscribe` is registered. The subscription module is correct and must remain as-is for WebSocket support to work.

The simplest fix is to add an RPC middleware layer (or a jsonrpsee `method_guard`) that intercepts `eth_subscribe` and `eth_unsubscribe` calls arriving over HTTP and rewrites the error code from `-32603` to `-32601` with a descriptive message. Alternatively, this could be handled upstream if jsonrpsee adds transport-aware error codes for subscription methods.

A practical approach in `server.rs` is to wrap the RPC service layer to detect HTTP-transport calls to subscription methods and override the response error code:

```rust
// In the RateLimitedRpcService (or a new middleware layer), check if the
// request method is "eth_subscribe" or "eth_unsubscribe" and the transport
// is HTTP, then return -32601 instead of letting jsonrpsee produce -32603.
```

This is a small change to the existing RPC middleware pipeline and does not affect the WebSocket subscription path at all.

**WARNING:** Do NOT use the "conditional registration" approach (only registering the subscription module when WebSocket is enabled, and registering HTTP fallback methods otherwise). The jsonrpsee `Server` already serves BOTH HTTP and WebSocket on the same port. Removing the subscription module registration would BREAK existing WebSocket subscription support. The subscription module MUST always be registered.

## Affected Files

| File | Line(s) | Role |
|------|---------|------|
| `crates/node/rpc/src/server.rs` | 460-537 | RPC middleware where the error code override would go |
| `crates/node/rpc/src/error.rs` | 7-33 | Error code definitions (already has `METHOD_NOT_FOUND = -32601`) |

Files that should NOT be changed:

| File | Line(s) | Why |
|------|---------|-----|
| `crates/node/rpc/src/subscription.rs` | 62-129 | Working WebSocket subscription handler; must not be altered |

## Severity

Very Low -- cosmetic error code mismatch on a rare edge case (HTTP caller using a WebSocket-only method). No impact on consensus, state, execution, or any WebSocket clients. Consider folding into a broader RPC spec-compliance effort rather than tracking as a standalone issue.
