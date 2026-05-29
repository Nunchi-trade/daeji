//! Execution configuration.

use alloy_primitives::Address;
use serde::{Deserialize, Serialize};

/// Default gas limit per block.
pub const DEFAULT_GAS_LIMIT: u64 = 250_000_000;

/// Initial base fee per gas (1 gwei).
///
/// EIP-1559 base-fee accounting requires a non-zero seed value; starting
/// from zero means `calculate_base_fee` can never increase the fee because
/// `0 * anything = 0`. One gwei is the Ethereum-mainnet genesis value and
/// a reasonable default for devnets.
pub const INITIAL_BASE_FEE: u64 = 1_000_000_000;

/// Execution layer configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionConfig {
    /// Maximum gas per block.
    #[serde(default = "default_gas_limit")]
    pub gas_limit: u64,

    /// Address that receives priority fees (EIP-1559 tips) and is returned
    /// by the `COINBASE` opcode.  Defaults to `Address::ZERO`, which
    /// effectively burns all priority fees.
    ///
    /// Set this to the operator's fee-collection address so that validators
    /// earn revenue from transaction inclusion.
    #[serde(default = "default_fee_recipient")]
    pub fee_recipient: Address,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self { gas_limit: DEFAULT_GAS_LIMIT, fee_recipient: Address::ZERO }
    }
}

const fn default_gas_limit() -> u64 {
    DEFAULT_GAS_LIMIT
}

const fn default_fee_recipient() -> Address {
    Address::ZERO
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_execution_config() {
        let config = ExecutionConfig::default();
        assert_eq!(config.gas_limit, DEFAULT_GAS_LIMIT);
        assert_eq!(config.fee_recipient, Address::ZERO);
    }

    #[test]
    fn test_execution_config_serde_roundtrip() {
        let config = ExecutionConfig { gas_limit: 300_000_000, ..Default::default() };
        let serialized = serde_json::to_string(&config).expect("serialize");
        let deserialized: ExecutionConfig = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_execution_config_toml_roundtrip() {
        let config = ExecutionConfig { gas_limit: 150_000_000, ..Default::default() };
        let serialized = toml::to_string(&config).expect("serialize toml");
        let deserialized: ExecutionConfig = toml::from_str(&serialized).expect("deserialize toml");
        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_execution_config_serde_defaults() {
        let config: ExecutionConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.gas_limit, DEFAULT_GAS_LIMIT);
        assert_eq!(config.fee_recipient, Address::ZERO);
    }

    #[test]
    fn test_execution_config_partial_defaults() {
        let config: ExecutionConfig =
            serde_json::from_str(r#"{"gas_limit": 10000000}"#).expect("deserialize");
        assert_eq!(config.gas_limit, 10_000_000);
        assert_eq!(config.fee_recipient, Address::ZERO);
    }

    #[test]
    fn test_execution_config_fee_recipient_roundtrip() {
        let recipient = Address::repeat_byte(0xAB);
        let config = ExecutionConfig { gas_limit: DEFAULT_GAS_LIMIT, fee_recipient: recipient };
        let serialized = serde_json::to_string(&config).expect("serialize");
        let deserialized: ExecutionConfig = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized.fee_recipient, recipient);
    }

    #[test]
    fn initial_base_fee_is_one_gwei() {
        assert_eq!(INITIAL_BASE_FEE, 1_000_000_000);
    }

    #[test]
    fn test_execution_config_clone_and_eq() {
        let config = ExecutionConfig { gas_limit: 999, ..Default::default() };
        assert_eq!(config, config.clone());
        assert_ne!(config, ExecutionConfig::default());
    }
}
