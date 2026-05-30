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
/// Supports both IP:port and DNS hostname:port formats for `dialable_addr`
/// via [`TransportParsing::parse_ingress`].
fn parse_network_config(
    config: &NetworkConfig,
) -> Result<(SocketAddr, Ingress, Bootstrappers), TransportError> {
    let listen_addr: SocketAddr = config
        .listen_addr
        .parse()
        .map_err(|_| TransportError::InvalidListenAddr(config.listen_addr.clone()))?;

    let dialable = if let Some(ref dialable_addr) = config.dialable_addr {
        TransportParsing::parse_ingress(dialable_addr)?
    } else {
        Ingress::Socket(listen_addr)
    };

    let bootstrappers = TransportParsing::parse_bootstrappers(&config.bootstrap_peers)?;

    Ok((listen_addr, dialable, bootstrappers))
}

#[cfg(test)]
mod tests {
    use kora_config::NetworkConfig;

    use super::*;

    #[test]
    fn parse_network_config_ip_dialable_addr() {
        let config = NetworkConfig {
            listen_addr: "0.0.0.0:30303".to_string(),
            dialable_addr: Some("1.2.3.4:30303".to_string()),
            bootstrap_peers: vec![],
            tx_gossip: false,
        };
        let (listen, dialable, bootstrappers) = parse_network_config(&config).unwrap();
        assert_eq!(listen.port(), 30303);
        assert!(matches!(dialable, Ingress::Socket(_)));
        assert!(bootstrappers.is_empty());
    }

    #[test]
    fn parse_network_config_dns_dialable_addr() {
        let config = NetworkConfig {
            listen_addr: "0.0.0.0:30303".to_string(),
            dialable_addr: Some("validator-0.example.com:30303".to_string()),
            bootstrap_peers: vec![],
            tx_gossip: false,
        };
        let (_listen, dialable, _bootstrappers) = parse_network_config(&config).unwrap();
        assert!(
            matches!(dialable, Ingress::Dns { .. }),
            "DNS hostnames must be parsed as Ingress::Dns, got {dialable:?}",
        );
    }

    #[test]
    fn parse_network_config_no_dialable_addr_falls_back_to_listen() {
        let config = NetworkConfig {
            listen_addr: "0.0.0.0:30303".to_string(),
            dialable_addr: None,
            bootstrap_peers: vec![],
            tx_gossip: false,
        };
        let (listen, dialable, _) = parse_network_config(&config).unwrap();
        assert!(matches!(dialable, Ingress::Socket(addr) if addr == listen));
    }

    #[test]
    fn parse_network_config_invalid_listen_addr() {
        let config = NetworkConfig {
            listen_addr: "not-a-socket-addr".to_string(),
            dialable_addr: None,
            bootstrap_peers: vec![],
            tx_gossip: false,
        };
        let result = parse_network_config(&config);
        assert!(matches!(result, Err(TransportError::InvalidListenAddr(_))));
    }
}
