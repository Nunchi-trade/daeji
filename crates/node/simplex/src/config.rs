//! Provides a default simplex configuration.

use std::time::Duration;

use commonware_consensus::{
    CertifiableAutomaton, Relay, Reporter,
    simplex::{self, ForwardingPolicy, Plan, elector::Random, types::Activity},
    types::{Epoch, ViewDelta},
};
use commonware_cryptography::{Digest, certificate::Scheme};
use commonware_p2p::Blocker;
use commonware_parallel::Sequential;
use commonware_runtime::buffer::paged::CacheRef;
use commonware_utils::NZUsize;

/// Default mailbox size for internal consensus channels.
pub const DEFAULT_MAILBOX_SIZE: usize = 1024;

/// Default replay buffer size (1 MiB).
pub const DEFAULT_REPLAY_BUFFER: usize = 1024 * 1024;

/// Default write buffer size (1 MiB).
pub const DEFAULT_WRITE_BUFFER: usize = 1024 * 1024;

/// Default leader timeout (1 second).
///
/// Synchronized with [`kora_config::DEFAULT_SIMPLEX_LEADER_TIMEOUT_SECS`].
/// Healthy views complete in ~7 ms; 1 s provides a 143x safety margin.
pub const DEFAULT_LEADER_TIMEOUT: Duration = Duration::from_secs(1);

/// Default notarization timeout (2 seconds).
///
/// Synchronized with [`kora_config::DEFAULT_SIMPLEX_CERTIFICATION_TIMEOUT_SECS`].
pub const DEFAULT_NOTARIZATION_TIMEOUT: Duration = Duration::from_secs(2);

/// Default nullify retry interval (1 second).
///
/// Synchronized with [`kora_config::DEFAULT_SIMPLEX_TIMEOUT_RETRY_SECS`].
/// Reduced from 5 s to match the production runner default, limiting the
/// compounding penalty (`leader_timeout + N * timeout_retry`) when a
/// nullification quorum requires multiple retry rounds.
pub const DEFAULT_NULLIFY_RETRY: Duration = Duration::from_secs(1);

/// Default fetch timeout (2 seconds).
///
/// Synchronized with [`kora_config::DEFAULT_SIMPLEX_FETCH_TIMEOUT_SECS`].
pub const DEFAULT_FETCH_TIMEOUT: Duration = Duration::from_secs(2);

/// Default activity timeout (256 views).
///
/// Synchronized with [`kora_config::DEFAULT_SIMPLEX_ACTIVITY_TIMEOUT_VIEWS`].
/// At ~135 views/sec, 20 views (~148 ms) was too aggressive; 256 views
/// (~1.9 s) is realistic for detecting genuinely inactive peers.
pub const DEFAULT_ACTIVITY_TIMEOUT: ViewDelta = ViewDelta::new(256);

/// Default skip timeout (32 views).
///
/// Synchronized with [`kora_config::DEFAULT_SIMPLEX_SKIP_TIMEOUT_VIEWS`].
/// At ~135 views/sec, 10 views (~74 ms) was too fast; 32 views (~237 ms)
/// provides adequate time for a healthy peer to respond.
pub const DEFAULT_SKIP_TIMEOUT: ViewDelta = ViewDelta::new(32);

/// Default number of concurrent fetch requests.
pub const DEFAULT_FETCH_CONCURRENT: usize = 8;

/// The default simplex configuration constructor.
///
/// Creates a [`simplex::Config`] with sensible defaults using:
/// - [`Random`] leader election
/// - [`Sequential`] execution strategy
/// - Default buffer pool from [`DefaultPool`]
/// - Default timing parameters
#[derive(Debug, Clone, Copy)]
pub struct DefaultConfig;

impl DefaultConfig {
    /// Initializes a default [`simplex::Config`].
    ///
    /// # Parameters
    ///
    /// - `partition`: Unique partition name for the consensus engine's journal
    /// - `scheme`: Signing scheme (e.g., BLS12-381 threshold VRF)
    /// - `blocker`: Network blocker for peer management
    /// - `automaton`: Application interface for block production/verification
    /// - `relay`: Relay for broadcasting payloads
    /// - `reporter`: Activity reporter for observability
    #[allow(clippy::type_complexity)]
    pub fn init<S, B, D, A, R, F>(
        partition: impl Into<String>,
        page_cache: CacheRef,
        scheme: S,
        blocker: B,
        automaton: A,
        relay: R,
        reporter: F,
    ) -> simplex::Config<S, Random, B, D, A, R, F, Sequential>
    where
        S: Scheme,
        Random: simplex::elector::Config<S>,
        B: Blocker<PublicKey = S::PublicKey>,
        D: Digest,
        A: CertifiableAutomaton<Context = simplex::types::Context<D, S::PublicKey>, Digest = D>,
        R: Relay<Digest = D, PublicKey = S::PublicKey, Plan = Plan<S::PublicKey>>,
        F: Reporter<Activity = Activity<S, D>>,
    {
        simplex::Config {
            scheme,
            elector: Random,
            blocker,
            automaton,
            relay,
            reporter,
            strategy: Sequential,
            partition: partition.into(),
            mailbox_size: NZUsize!(DEFAULT_MAILBOX_SIZE),
            epoch: Epoch::zero(),
            floor: simplex::Floor::Genesis(D::EMPTY),
            replay_buffer: NZUsize!(DEFAULT_REPLAY_BUFFER),
            write_buffer: NZUsize!(DEFAULT_WRITE_BUFFER),
            leader_timeout: DEFAULT_LEADER_TIMEOUT,
            certification_timeout: DEFAULT_NOTARIZATION_TIMEOUT,
            timeout_retry: DEFAULT_NULLIFY_RETRY,
            fetch_timeout: DEFAULT_FETCH_TIMEOUT,
            activity_timeout: DEFAULT_ACTIVITY_TIMEOUT,
            skip_timeout: DEFAULT_SKIP_TIMEOUT,
            fetch_concurrent: NZUsize!(DEFAULT_FETCH_CONCURRENT),
            page_cache,
            forwarding: ForwardingPolicy::Disabled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mailbox_size_is_1024() {
        assert_eq!(DEFAULT_MAILBOX_SIZE, 1024);
    }

    #[test]
    fn default_replay_buffer_is_1mib() {
        assert_eq!(DEFAULT_REPLAY_BUFFER, 1024 * 1024);
    }

    #[test]
    fn default_write_buffer_is_1mib() {
        assert_eq!(DEFAULT_WRITE_BUFFER, 1024 * 1024);
    }

    #[test]
    fn default_leader_timeout_is_1_second() {
        assert_eq!(DEFAULT_LEADER_TIMEOUT, Duration::from_secs(1));
    }

    #[test]
    fn default_notarization_timeout_is_2_seconds() {
        assert_eq!(DEFAULT_NOTARIZATION_TIMEOUT, Duration::from_secs(2));
    }

    #[test]
    fn default_nullify_retry_is_1_second() {
        assert_eq!(DEFAULT_NULLIFY_RETRY, Duration::from_secs(1));
    }

    #[test]
    fn default_fetch_timeout_is_2_seconds() {
        assert_eq!(DEFAULT_FETCH_TIMEOUT, Duration::from_secs(2));
    }

    #[test]
    fn default_activity_timeout_is_256_views() {
        assert_eq!(DEFAULT_ACTIVITY_TIMEOUT, ViewDelta::new(256));
    }

    #[test]
    fn default_skip_timeout_is_32_views() {
        assert_eq!(DEFAULT_SKIP_TIMEOUT, ViewDelta::new(32));
    }

    #[test]
    fn default_fetch_concurrent_is_8() {
        assert_eq!(DEFAULT_FETCH_CONCURRENT, 8);
    }

    #[test]
    fn default_config_has_debug_impl() {
        let config = DefaultConfig;
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("DefaultConfig"));
    }

    #[test]
    fn default_config_is_copy() {
        let config = DefaultConfig;
        let config2 = config;
        let _ = config;
        let _ = config2;
    }
}
