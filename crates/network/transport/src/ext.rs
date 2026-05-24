//! Extension trait for NetworkConfig to build transport.

use std::net::SocketAddr;

use commonware_cryptography::ed25519;
use commonware_p2p::Ingress;
use commonware_runtime::{BufferPooler, Clock, Metrics, Network as RNetwork, Resolver, Spawner};
use kora_config::NetworkConfig;
use rand_core::CryptoRngCore;

use crate::{
    NetworkTransport, TransportConfig, TransportError, TransportParsing,
    config::{DEFAULT_MAX_MESSAGE_SIZE, DEFAULT_NAMESPACE},
};

/// Extension trait for building transport from network configuration.
pub trait NetworkConfigExt {
    /// Build a local development transport.
    ///
    /// Uses faster discovery and more lenient settings for local testing.
    fn build_local_transport<E>(
        &self,
        crypto: ed25519::PrivateKey,
        context: E,
    ) -> Result<NetworkTransport<ed25519::PublicKey, E>, TransportError>
    where
        E: Spawner + BufferPooler + Clock + CryptoRngCore + RNetwork + Resolver + Metrics;

    /// Build a production transport.
    ///
    /// Uses conservative settings suitable for production deployments.
    fn build_transport<E>(
        &self,
        crypto: ed25519::PrivateKey,
        context: E,
    ) -> Result<NetworkTransport<ed25519::PublicKey, E>, TransportError>
    where
        E: Spawner + BufferPooler + Clock + CryptoRngCore + RNetwork + Resolver + Metrics;
}

impl NetworkConfigExt for NetworkConfig {
    fn build_local_transport<E>(
        &self,
        crypto: ed25519::PrivateKey,
        context: E,
    ) -> Result<NetworkTransport<ed25519::PublicKey, E>, TransportError>
    where
        E: Spawner + BufferPooler + Clock + CryptoRngCore + RNetwork + Resolver + Metrics,
    {
        let (listen_addr, dialable, bootstrappers) = parse_network_config(self)?;

        let transport_config = TransportConfig::local(
            crypto,
            DEFAULT_NAMESPACE,
            listen_addr,
            dialable,
            bootstrappers,
            DEFAULT_MAX_MESSAGE_SIZE,
        )
        .with_allow_private_ips(true);

        Ok(transport_config.build(context))
    }

    fn build_transport<E>(
        &self,
        crypto: ed25519::PrivateKey,
        context: E,
    ) -> Result<NetworkTransport<ed25519::PublicKey, E>, TransportError>
    where
        E: Spawner + BufferPooler + Clock + CryptoRngCore + RNetwork + Resolver + Metrics,
    {
        let (listen_addr, dialable, bootstrappers) = parse_network_config(self)?;

        let transport_config = TransportConfig::recommended(
            crypto,
            DEFAULT_NAMESPACE,
            listen_addr,
            dialable,
            bootstrappers,
            DEFAULT_MAX_MESSAGE_SIZE,
        );

        Ok(transport_config.build(context))
    }
}

/// Bootstrapper list type alias.
type Bootstrappers = Vec<(ed25519::PublicKey, Ingress)>;

/// Parse network config into transport construction parameters.
///
/// When `dialable_addr` is not set and `listen_addr` binds to `0.0.0.0`
/// (the wildcard), the function resolves the system hostname and advertises
/// `hostname:port` as the dialable address.  This is essential inside Docker
/// containers where `0.0.0.0` is not reachable from other containers but
/// the container hostname (set via `hostname:` in compose) resolves to the
/// correct bridge-network IP.
fn parse_network_config(
    config: &NetworkConfig,
) -> Result<(SocketAddr, Ingress, Bootstrappers), TransportError> {
    let listen_addr: SocketAddr = config
        .listen_addr
        .parse()
        .map_err(|_| TransportError::InvalidListenAddr(config.listen_addr.clone()))?;

    let dialable = if let Some(ref dialable_addr) = config.dialable_addr {
        TransportParsing::parse_ingress(dialable_addr)?
    } else if listen_addr.ip().is_unspecified() {
        // 0.0.0.0 is not dialable from other hosts.  Resolve the system
        // hostname so peers can reach us via DNS (works in Docker where
        // the compose `hostname:` directive sets a resolvable name).
        match std::env::var("HOSTNAME") {
            Ok(name) if !name.is_empty() => {
                let dialable_str = format!("{}:{}", name, listen_addr.port());
                tracing::info!(
                    dialable_addr = %dialable_str,
                    "listen_addr is 0.0.0.0; using HOSTNAME as dialable address"
                );
                TransportParsing::parse_ingress(&dialable_str)
                    .unwrap_or_else(|_| Ingress::Socket(listen_addr))
            }
            _ => Ingress::Socket(listen_addr),
        }
    } else {
        Ingress::Socket(listen_addr)
    };

    let bootstrappers = TransportParsing::parse_bootstrappers(&config.bootstrap_peers)?;

    Ok((listen_addr, dialable, bootstrappers))
}
