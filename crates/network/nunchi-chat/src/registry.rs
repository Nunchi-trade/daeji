//! Authorized peer registry — the on-chain state-of-the-world that drives `oracle.track`.
//!
//! Two record kinds (untagged enum, both round-trip through the same JSON):
//! - `Seed { seed: u64 }` — POC shortcut. Pubkey is derived deterministically via
//!   `ed25519::PrivateKey::from_seed(seed)`. Used by Phase 1/2 demos.
//! - `Chain { controller, transport_pubkey, capabilities, endpoint }` — the production shape
//!   per spec D1 (corrected). Sourced from `AgentRegistry.AgentRegistered` events on
//!   `~/contracts-core/packages/agents/src/AgentRegistry.sol`, with the off-chain status.json
//!   resolved + verified via `card.rs`.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use commonware_cryptography::{Signer as _, ed25519};
use commonware_utils::ordered::Set;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum AgentRecord {
    Seed {
        seed: u64,
    },
    Chain {
        /// EVM controller address (0x-prefixed hex), the agent's `msg.sender` to AgentRegistry.
        controller: String,
        /// 32-byte ed25519 transport pubkey, 0x-prefixed hex.
        transport_pubkey: String,
        /// Pipe-delimited capabilities string from the on-chain registration.
        capabilities: String,
        /// URL extracted from `capabilities[endpoint=…]` for ops debugging.
        endpoint: String,
    },
}

impl AgentRecord {
    /// Derive (Seed) or parse (Chain) the commonware ed25519 pubkey for this agent.
    pub fn pubkey(&self) -> Result<ed25519::PublicKey, RegistryError> {
        match self {
            Self::Seed { seed } => Ok(ed25519::PrivateKey::from_seed(*seed).public_key()),
            Self::Chain { transport_pubkey, .. } => {
                let stripped =
                    transport_pubkey.trim().strip_prefix("0x").unwrap_or(transport_pubkey);
                let raw = hex::decode(stripped).map_err(|_| RegistryError::BadPubkey)?;
                if raw.len() != 32 {
                    return Err(RegistryError::BadPubkey);
                }
                let mut buf = [0u8; 32];
                buf.copy_from_slice(&raw);
                use commonware_codec::extensions::DecodeExt;
                ed25519::PublicKey::decode(buf.as_slice()).map_err(|_| RegistryError::BadPubkey)
            }
        }
    }

    /// Stable identity key for change detection / dedup.
    pub fn ident(&self) -> String {
        match self {
            Self::Seed { seed } => format!("seed:{seed}"),
            Self::Chain { controller, .. } => format!("chain:{controller}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Registry {
    pub epoch: u64,
    pub agents: Vec<AgentRecord>,
}

impl Registry {
    pub const fn empty() -> Self {
        Self { epoch: 0, agents: Vec::new() }
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let raw = fs::read_to_string(path)?;
        serde_json::from_str(&raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut tmp: PathBuf = path.into();
        tmp.set_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self).unwrap())?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Insert a seed shortcut record if absent. Bumps epoch on change.
    pub fn add(&mut self, seed: u64) -> bool {
        let ident = format!("seed:{seed}");
        if self.agents.iter().any(|a| a.ident() == ident) {
            return false;
        }
        self.agents.push(AgentRecord::Seed { seed });
        self.epoch += 1;
        true
    }

    /// Insert a chain-mode record if absent. Bumps epoch on change. Used by the chain-watcher
    /// path that consumes `AgentRegistered` events from contracts-core's AgentRegistry.
    pub fn add_chain(
        &mut self,
        controller: String,
        transport_pubkey: String,
        capabilities: String,
        endpoint: String,
    ) -> bool {
        let ident = format!("chain:{controller}");
        if self.agents.iter().any(|a| a.ident() == ident) {
            return false;
        }
        self.agents.push(AgentRecord::Chain {
            controller,
            transport_pubkey,
            capabilities,
            endpoint,
        });
        self.epoch += 1;
        true
    }

    /// Remove by seed (matches the legacy daeji-indexer remove path).
    pub fn remove(&mut self, seed: u64) -> bool {
        let ident = format!("seed:{seed}");
        self.remove_by_ident(&ident)
    }

    pub fn remove_chain(&mut self, controller: &str) -> bool {
        let ident = format!("chain:{controller}");
        self.remove_by_ident(&ident)
    }

    fn remove_by_ident(&mut self, ident: &str) -> bool {
        let before = self.agents.len();
        self.agents.retain(|a| a.ident() != ident);
        let removed = self.agents.len() != before;
        if removed {
            self.epoch += 1;
        }
        removed
    }

    /// Materialize the authorized pubkey set as commonware expects it. Mixed seed + chain
    /// records both work; commonware just sees the union of public keys.
    pub fn pubkey_set(&self) -> Set<ed25519::PublicKey> {
        let pubs: Vec<ed25519::PublicKey> =
            self.agents.iter().filter_map(|a| a.pubkey().ok()).collect();
        Set::try_from(pubs).expect("registry pubkeys are unique")
    }

    /// Legacy: list seed shortcuts only. Kept for the existing `daeji-indexer show` output.
    pub fn seeds(&self) -> Vec<u64> {
        self.agents
            .iter()
            .filter_map(|a| match a {
                AgentRecord::Seed { seed } => Some(*seed),
                _ => None,
            })
            .collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("transport pubkey is not 32 bytes of hex")]
    BadPubkey,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_bump_epoch() {
        let mut r = Registry::empty();
        assert!(r.add(1));
        assert_eq!(r.epoch, 1);
        assert!(r.add(2));
        assert_eq!(r.epoch, 2);
        assert!(!r.add(1));
        assert_eq!(r.epoch, 2);
    }

    #[test]
    fn remove_and_bump_epoch() {
        let mut r = Registry {
            epoch: 5,
            agents: vec![AgentRecord::Seed { seed: 1 }, AgentRecord::Seed { seed: 2 }],
        };
        assert!(r.remove(1));
        assert_eq!(r.epoch, 6);
        assert!(!r.remove(99));
        assert_eq!(r.epoch, 6);
    }

    #[test]
    fn save_load_round_trip_mixed() {
        let dir = tempdir();
        let path = dir.join("registry.json");
        let r = Registry {
            epoch: 3,
            agents: vec![
                AgentRecord::Seed { seed: 1 },
                AgentRecord::Chain {
                    controller: "0xBcd4042DE499D14e55001CcbB24a551F3b954096".into(),
                    transport_pubkey: "0x".to_string() + &"77".repeat(32),
                    capabilities: "perps_liquidator|endpoint=https://example/status.json".into(),
                    endpoint: "https://example/status.json".into(),
                },
            ],
        };
        r.save(&path).unwrap();
        let r2 = Registry::load(&path).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn chain_record_pubkey_parses() {
        let r = AgentRecord::Chain {
            controller: "0xabc".into(),
            transport_pubkey: "0x".to_string() + &"77".repeat(32),
            capabilities: "x".into(),
            endpoint: "u".into(),
        };
        let pk = r.pubkey().unwrap();
        assert_eq!(pk.as_ref(), &[0x77u8; 32]);
    }

    #[test]
    fn add_chain_dedup_by_controller() {
        let mut r = Registry::empty();
        let added = r.add_chain(
            "0xABCD".into(),
            "0x".to_string() + &"11".repeat(32),
            "endpoint=u".into(),
            "u".into(),
        );
        assert!(added);
        assert_eq!(r.epoch, 1);
        let dup = r.add_chain(
            "0xABCD".into(),
            "0x".to_string() + &"22".repeat(32),
            "endpoint=u".into(),
            "u".into(),
        );
        assert!(!dup);
        assert_eq!(r.epoch, 1);
    }

    #[test]
    fn pubkey_set_unions_seed_and_chain() {
        let mut r = Registry::empty();
        r.add(1);
        r.add_chain(
            "0xC".into(),
            "0x".to_string() + &"33".repeat(32),
            "endpoint=u".into(),
            "u".into(),
        );
        let set = r.pubkey_set();
        assert_eq!(set.len(), 2);
    }

    fn tempdir() -> PathBuf {
        let pid = std::process::id();
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("nunchi-chat-poc-{pid}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
