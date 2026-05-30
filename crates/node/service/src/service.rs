//! Kora node service implementation.

use std::sync::Arc;

use ::tokio::signal::unix::{SignalKind, signal as unix_signal};
use commonware_cryptography::Signer;
use commonware_p2p::Manager;
use commonware_runtime::{
    Runner,
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
#[derive(Debug)]
pub struct LegacyNodeService {
    config: NodeConfig,
}

impl LegacyNodeService {
    /// Create a new legacy node service.
    pub const fn new(config: NodeConfig) -> Self {
        Self { config }
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
            .build_local_transport(validator_key, context)
            .map_err(|e| eyre::eyre!("failed to build transport: {}", e))?;
        tracing::info!("network transport started");

        let validators = self.config.consensus.build_validator_set()?;
        if !validators.is_empty() {
            let validator_set: commonware_utils::ordered::Set<_> = validators
                .try_into()
                .map_err(|_| eyre::eyre!("failed to convert validator set"))?;
            transport.oracle.track(0, validator_set);
            tracing::info!("registered validators with oracle");
        }

        tracing::info!(chain_id = self.config.chain_id, "kora node initialized");

        // Race the transport join against a shutdown signal so that
        // `docker stop` / `systemctl stop` / `kill` (SIGTERM) causes a clean
        // exit instead of being silently ignored until the 30-second
        // SIGKILL timeout fires.
        let mut sigterm =
            unix_signal(SignalKind::terminate()).expect("failed to register SIGTERM handler");
        ::tokio::select! {
            result = try_join_all(vec![transport.handle]) => {
                if let Err(e) = result {
                    tracing::error!(?e, "service task failed");
                    return Err(eyre::eyre!("service task failed: {:?}", e));
                }
            }
            _ = ::tokio::signal::ctrl_c() => {
                tracing::info!("Received SIGINT, initiating graceful shutdown...");
            }
            _ = sigterm.recv() => {
                tracing::info!("Received SIGTERM, initiating graceful shutdown...");
            }
        }

        tracing::info!("kora node shutdown");
        Ok(())
    }
}
