//! Shutdown signal handling for Kora processes.
//!
//! Provides a shared helper that waits for either SIGINT (Ctrl+C) or SIGTERM,
//! the two standard shutdown signals for Kora processes:
//!
//! - SIGINT is sent by Ctrl+C during interactive terminal use.
//! - SIGTERM is sent by `docker stop`, `docker compose stop`, `systemctl stop`,
//!   Kubernetes pod termination, and `kill <pid>` (the default kill signal).
//!
//! All Kora code paths that need to block until shutdown should call
//! [`wait_for_shutdown_signal`] rather than rolling their own signal handling.

use tokio::signal::unix::{SignalKind, signal};

/// Wait for either SIGINT (Ctrl+C) or SIGTERM, then return.
///
/// This is the canonical shutdown signal helper for Kora processes running in
/// Docker containers or under process supervisors. It handles both signals so
/// that interactive users (Ctrl+C → SIGINT) and orchestration systems
/// (`docker stop` → SIGTERM) both trigger a clean exit.
///
/// # Panics
///
/// Panics if the SIGTERM handler cannot be registered. This should not happen
/// on any supported Unix platform when tokio is compiled with the `signal`
/// feature (included in `features = ["full"]`).
pub async fn wait_for_shutdown_signal() {
    let mut sigterm = signal(SignalKind::terminate()).expect("failed to register SIGTERM handler");

    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(e) = result {
                tracing::warn!(error = %e, "ctrl_c signal handler error");
            } else {
                tracing::info!("Received SIGINT (Ctrl+C), initiating graceful shutdown...");
            }
        }
        _ = sigterm.recv() => {
            tracing::info!("Received SIGTERM, initiating graceful shutdown...");
        }
    }
}
