//! Supervisor primitives for the chat service per canonical-plan §19 (B2).
//!
//! Splits the validator-isolation hardening into reusable helpers:
//! - [`disabled_via_env`] — runtime kill switch (DAEJI_CHAT_DISABLED env var).
//!   Per §19 B2.3: an operator can disable chat without restarting kora by
//!   setting the env var, sending SIGUSR1, and letting the kora signal
//!   handler re-check the var. This decouples the disable mechanism from
//!   the (yet to be re-established) kora-binary integration.
//! - [`next_backoff`] — exponential backoff for the supervised-restart loop.
//!   Per §19 B2.4: chat panic loops trigger backoff with a 60s cap; after
//!   3 failures within 60s the supervisor logs an alert + stops respawning.
//! - [`PanicTracker`] — sliding window over recent failures used by a future
//!   in-process supervisor to decide when to give up.
//!
//! The full supervisor wrapper around [`run_chat`](crate::service::run_chat)
//! lands in PR-Daeji-G once the kora-side integration that consumed
//! `run_chat` is re-established (it was displaced during a recent main
//! rebase and needs a fresh PR).

use std::time::Duration;

/// Environment variable that disables the chat service at runtime.
/// When set to a truthy value (`1`, `true`, `yes`, case-insensitive),
/// [`run_chat`](crate::service::run_chat) returns early before opening any
/// network sockets.
pub const DISABLE_ENV_VAR: &str = "DAEJI_CHAT_DISABLED";

/// Initial backoff between supervised restarts. After a chat panic, the
/// supervisor sleeps this long before re-spawning, then doubles per attempt
/// up to [`MAX_BACKOFF`].
pub const INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// Maximum backoff between supervised restarts. Past this, repeated failures
/// stay capped — preventing pathologically long sleep windows while still
/// limiting CPU pressure from rapid respawn loops.
pub const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Number of failures within [`PANIC_WINDOW`] before the supervisor stops
/// respawning. Past this threshold the chat service is effectively disabled
/// until the operator intervenes (manual restart or env-var flip).
pub const PANIC_THRESHOLD: usize = 3;

/// Sliding-window duration over which [`PANIC_THRESHOLD`] is enforced.
pub const PANIC_WINDOW: Duration = Duration::from_secs(60);

/// Returns `true` when the [`DISABLE_ENV_VAR`] is set to a truthy value.
/// Truthy values: `1`, `true`, `yes` (case-insensitive). Anything else
/// (including unset) is `false`.
///
/// Operators set this when chat is misbehaving in production and they
/// want the service shut down on the next kora restart without recompiling
/// or editing the chat config file. Kora's signal handler can also re-check
/// this after a SIGUSR1 to support runtime disable without process restart.
pub fn disabled_via_env() -> bool {
    matches!(
        std::env::var(DISABLE_ENV_VAR).ok().as_deref(),
        Some("1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes")
    )
}

/// Compute the next backoff duration given the previous one. Doubles each
/// step up to [`MAX_BACKOFF`]. Pass [`Duration::ZERO`] (or anything less
/// than [`INITIAL_BACKOFF`]) for the first call.
pub fn next_backoff(previous: Duration) -> Duration {
    if previous < INITIAL_BACKOFF {
        return INITIAL_BACKOFF;
    }
    let doubled = previous.saturating_mul(2);
    if doubled > MAX_BACKOFF {
        MAX_BACKOFF
    } else {
        doubled
    }
}

/// Sliding window over recent failure timestamps. The supervisor pushes a
/// failure each time `run_chat` exits with an error or panics; once the
/// window exceeds [`PANIC_THRESHOLD`] failures within [`PANIC_WINDOW`], the
/// supervisor stops respawning and surfaces an alert.
///
/// Lightweight `Vec`-backed; we don't expect thousands of entries.
#[derive(Debug, Default)]
pub struct PanicTracker {
    /// Monotonic timestamps (e.g. from `Instant::elapsed()`-style monotonic
    /// counter) of recent failures, oldest first.
    failures: Vec<Duration>,
}

impl PanicTracker {
    /// New empty tracker.
    pub const fn new() -> Self {
        Self { failures: Vec::new() }
    }

    /// Record a failure at the given monotonic time `now`. Old entries
    /// outside the [`PANIC_WINDOW`] are pruned.
    pub fn record(&mut self, now: Duration) {
        // Prune old entries — anything older than `now - PANIC_WINDOW`.
        let cutoff = now.saturating_sub(PANIC_WINDOW);
        self.failures.retain(|&t| t >= cutoff);
        self.failures.push(now);
    }

    /// `true` when the failure count within [`PANIC_WINDOW`] exceeds
    /// [`PANIC_THRESHOLD`]. Caller stops respawning and alerts.
    pub fn should_give_up(&self, now: Duration) -> bool {
        let cutoff = now.saturating_sub(PANIC_WINDOW);
        self.failures.iter().filter(|&&t| t >= cutoff).count() >= PANIC_THRESHOLD
    }

    /// Number of failures in the current sliding window. Useful for metrics.
    pub fn count_in_window(&self, now: Duration) -> usize {
        let cutoff = now.saturating_sub(PANIC_WINDOW);
        self.failures.iter().filter(|&&t| t >= cutoff).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_via_env_truthy_values() {
        // SAFETY: `set_var` / `remove_var` are safe in single-threaded tests.
        // We don't run these tests under `--test-threads=N>1` where they'd race.
        for v in ["1", "true", "TRUE", "True", "yes", "YES", "Yes"] {
            unsafe { std::env::set_var(DISABLE_ENV_VAR, v) };
            assert!(disabled_via_env(), "{v:?} should be truthy");
        }
        unsafe { std::env::remove_var(DISABLE_ENV_VAR) };
    }

    #[test]
    fn disabled_via_env_falsy_values() {
        for v in ["0", "false", "no", "off", "", "definitely_not"] {
            unsafe { std::env::set_var(DISABLE_ENV_VAR, v) };
            assert!(!disabled_via_env(), "{v:?} should be falsy");
        }
        unsafe { std::env::remove_var(DISABLE_ENV_VAR) };
    }

    #[test]
    fn disabled_via_env_unset_is_false() {
        unsafe { std::env::remove_var(DISABLE_ENV_VAR) };
        assert!(!disabled_via_env());
    }

    #[test]
    fn next_backoff_starts_at_initial() {
        assert_eq!(next_backoff(Duration::ZERO), INITIAL_BACKOFF);
        assert_eq!(next_backoff(Duration::from_millis(500)), INITIAL_BACKOFF);
    }

    #[test]
    fn next_backoff_doubles() {
        assert_eq!(next_backoff(Duration::from_secs(1)), Duration::from_secs(2));
        assert_eq!(next_backoff(Duration::from_secs(2)), Duration::from_secs(4));
        assert_eq!(next_backoff(Duration::from_secs(4)), Duration::from_secs(8));
    }

    #[test]
    fn next_backoff_caps_at_max() {
        assert_eq!(next_backoff(Duration::from_secs(30)), Duration::from_secs(60));
        assert_eq!(next_backoff(Duration::from_secs(60)), Duration::from_secs(60));
        assert_eq!(next_backoff(Duration::from_secs(120)), Duration::from_secs(60));
    }

    #[test]
    fn panic_tracker_records_and_counts() {
        let mut t = PanicTracker::new();
        t.record(Duration::from_secs(5));
        t.record(Duration::from_secs(10));
        assert_eq!(t.count_in_window(Duration::from_secs(20)), 2);
    }

    #[test]
    fn panic_tracker_prunes_outside_window() {
        let mut t = PanicTracker::new();
        t.record(Duration::from_secs(5));
        // 70s later, the 5s entry is outside the 60s window.
        t.record(Duration::from_secs(75));
        assert_eq!(t.count_in_window(Duration::from_secs(75)), 1);
    }

    #[test]
    fn panic_tracker_should_give_up_at_threshold() {
        let mut t = PanicTracker::new();
        t.record(Duration::from_secs(1));
        t.record(Duration::from_secs(10));
        assert!(!t.should_give_up(Duration::from_secs(15)));
        t.record(Duration::from_secs(15));
        // 3 failures in 60s window → give up.
        assert!(t.should_give_up(Duration::from_secs(15)));
    }

    #[test]
    fn panic_tracker_does_not_give_up_when_failures_spread() {
        let mut t = PanicTracker::new();
        t.record(Duration::from_secs(0));
        t.record(Duration::from_secs(70));
        t.record(Duration::from_secs(140));
        // Each failure ages out before the next; window count never reaches 3.
        assert!(!t.should_give_up(Duration::from_secs(140)));
        assert_eq!(t.count_in_window(Duration::from_secs(140)), 1);
    }
}
