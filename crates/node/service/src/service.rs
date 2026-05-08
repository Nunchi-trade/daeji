//! Kora node service implementation.

use std::sync::Arc;

use commonware_cryptography::Signer;
use commonware_p2p::Manager;
use commonware_runtime::{
    Metrics, Runner, Spawner,
    tokio::{self, Context},
};
use futures::future::try_join_all;
use kora_config::NodeConfig;
use kora_transport::NetworkConfigExt;

use crate::{NodeRunContext, NodeRunner, TransportProvider};

/// Generic kora node service that delegates to a runner.
///
/// This is the primary way to run a kora node with custom execution logic.
/// The service handles transport creation via the `TransportProvider`,
/// then delegates node wiring to the `NodeRunner`.
pub struct KoraNodeService<R, T>
where
    R: NodeRunner<Transport = T::Transport>,
    T: TransportProvider,
{
    runner: R,
    transport_provider: T,
    config: NodeConfig,
}

impl<R, T> std::fmt::Debug for KoraNodeService<R, T>
where
    R: NodeRunner<Transport = T::Transport>,
    T: TransportProvider,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KoraNodeService").finish_non_exhaustive()
    }
}

impl<R, T> KoraNodeService<R, T>
where
    R: NodeRunner<Transport = T::Transport>,
    T: TransportProvider,
{
    /// Create a new generic node service.
    pub const fn new(runner: R, transport_provider: T, config: NodeConfig) -> Self {
        Self { runner, transport_provider, config }
    }

    /// Run the node service using the default tokio runtime.
    pub fn run(self) -> Result<R::Handle, eyre::Error>
    where
        R::Error: Into<eyre::Error>,
        T::Error: Into<eyre::Error>,
    {
        let executor = tokio::Runner::new(
            tokio::Config::default().with_storage_directory(self.config.data_dir.join("runtime")),
        );
        executor.start(|context| async move { self.run_with_context(context).await })
    }

    /// Run the node service with a provided context.
    pub async fn run_with_context(mut self, context: Context) -> Result<R::Handle, eyre::Error>
    where
        R::Error: Into<eyre::Error>,
        T::Error: Into<eyre::Error>,
    {
        let transport = self
            .transport_provider
            .build_transport(&context, &self.config)
            .await
            .map_err(Into::into)?;

        let run_ctx = NodeRunContext::new(context, Arc::new(self.config), transport);

        self.runner.run(run_ctx).await.map_err(Into::into)
    }
}

/// Legacy kora node service for production use.
///
/// This maintains backward compatibility with the existing production binary.
/// For new implementations, prefer [`KoraNodeService`] with custom runner/provider.
///
/// Optionally bolts the chat layer ([`daeji_chat::service::run_chat`]) into the
/// same kora process when [`Self::with_chat`] is set. The chat service runs as
/// a spawned tokio task on its own commonware-p2p network instance — separate
/// from kora's consensus mesh in v1, so a chat-side bug cannot impact consensus.
/// Sharing the consensus mesh is a future PR (requires modifying kora-transport).
#[derive(Debug)]
pub struct LegacyNodeService {
    config: NodeConfig,
    chat: Option<daeji_chat::service::ChatConfig>,
}

impl LegacyNodeService {
    /// Create a new legacy node service.
    pub const fn new(config: NodeConfig) -> Self {
        Self { config, chat: None }
    }

    /// Attach a chat configuration. When set and `enabled = true`, the chat
    /// service is spawned alongside the consensus runtime in `run_with_context`.
    #[must_use]
    pub fn with_chat(mut self, chat: daeji_chat::service::ChatConfig) -> Self {
        self.chat = Some(chat);
        self
    }

    /// Run the legacy node service.
    pub fn run(self) -> eyre::Result<()> {
        let executor = tokio::Runner::new(
            tokio::Config::default().with_storage_directory(self.config.data_dir.join("runtime")),
        );
        executor.start(|context| async move { self.run_with_context(context).await })
    }

    /// Runs the legacy node service with context.
    pub async fn run_with_context(self, context: Context) -> eyre::Result<()> {
        let validator_key = self.config.validator_key()?;
        let validator = validator_key.public_key();
        tracing::info!(?validator, "loaded validator key");

        let mut transport = self
            .config
            .network
            .build_local_transport(validator_key, context.clone())
            .map_err(|e| eyre::eyre!("failed to build transport: {}", e))?;
        tracing::info!("network transport started");

        let validators = self.config.consensus.build_validator_set()?;
        if !validators.is_empty() {
            let validator_set: commonware_utils::ordered::Set<_> = validators
                .try_into()
                .map_err(|_| eyre::eyre!("failed to convert validator set"))?;
            transport.oracle.track(0, validator_set).await;
            tracing::info!("registered validators with oracle");
        }

        tracing::info!(chain_id = self.config.chain_id, "kora node initialized");

        // Optionally spawn the chat service supervised. Runs as a spawned task on
        // the same kora process; uses its own commonware network instance
        // (separate port from consensus). Bug-isolation: a chat panic / network
        // error doesn't touch consensus.
        //
        // Supervisor (per canonical-plan §19 B2.4): `run_chat_supervised` retries
        // on error with exponential backoff up to 60s, gives up after 3 failures
        // within 60s. `Disabled` errors (config or `DAEJI_CHAT_DISABLED` env)
        // exit cleanly without retry.
        if let Some(chat_cfg) = self.chat.clone() {
            if chat_cfg.enabled {
                let chat_ctx = context.clone();
                tracing::info!(
                    bind_port = chat_cfg.bind_port,
                    seeded_jobs = chat_cfg.seed_jobs.len(),
                    "starting chat service (supervised)"
                );
                context.with_label("chat").spawn(move |_| async move {
                    match daeji_chat::supervisor::run_chat_supervised(chat_ctx, chat_cfg).await {
                        Ok(()) => {
                            tracing::info!("chat supervisor exited cleanly");
                        }
                        Err(err) => {
                            tracing::error!(?err, "chat supervisor gave up after threshold");
                        }
                    }
                });
            } else {
                tracing::info!("chat config present but disabled; skipping");
            }
        }

        if let Err(e) = try_join_all(vec![transport.handle]).await {
            tracing::error!(?e, "service task failed");
            return Err(eyre::eyre!("service task failed: {:?}", e));
        }

        tracing::info!("kora node shutdown");
        Ok(())
    }
}
