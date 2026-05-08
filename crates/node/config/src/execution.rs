//! Execution configuration.

use serde::{Deserialize, Serialize};

/// Default gas limit per block.
pub const DEFAULT_GAS_LIMIT: u64 = 250_000_000;

/// Default block time in milliseconds.
pub const DEFAULT_BLOCK_TIME_MS: u64 = 2000;

/// Execution layer configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionConfig {
    /// Maximum gas per block.
    #[serde(default = "default_gas_limit")]
    pub gas_limit: u64,

    /// Target block time in milliseconds.
    #[serde(default = "default_block_time_ms", alias = "block_time")]
    pub block_time_ms: u64,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self { gas_limit: DEFAULT_GAS_LIMIT, block_time_ms: DEFAULT_BLOCK_TIME_MS }
    }
}

const fn default_gas_limit() -> u64 {
    DEFAULT_GAS_LIMIT
}

const fn default_block_time_ms() -> u64 {
    DEFAULT_BLOCK_TIME_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_execution_config() {
        let config = ExecutionConfig::default();
        assert_eq!(config.gas_limit, DEFAULT_GAS_LIMIT);
        assert_eq!(config.block_time_ms, DEFAULT_BLOCK_TIME_MS);
    }

    #[test]
    fn test_execution_config_serde_roundtrip() {
        let config = ExecutionConfig { gas_limit: 300_000_000, block_time_ms: 5000 };
        let serialized = serde_json::to_string(&config).expect("serialize");
        let deserialized: ExecutionConfig = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_execution_config_toml_roundtrip() {
        let config = ExecutionConfig { gas_limit: 150_000_000, block_time_ms: 1000 };
        let serialized = toml::to_string(&config).expect("serialize toml");
        let deserialized: ExecutionConfig = toml::from_str(&serialized).expect("deserialize toml");
        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_execution_config_serde_defaults() {
        let config: ExecutionConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.gas_limit, DEFAULT_GAS_LIMIT);
        assert_eq!(config.block_time_ms, DEFAULT_BLOCK_TIME_MS);
    }

    #[test]
    fn test_execution_config_partial_defaults() {
        let config: ExecutionConfig =
            serde_json::from_str(r#"{"gas_limit": 10000000}"#).expect("deserialize");
        assert_eq!(config.gas_limit, 10_000_000);
        assert_eq!(config.block_time_ms, DEFAULT_BLOCK_TIME_MS);

        let config: ExecutionConfig =
            serde_json::from_str(r#"{"block_time_ms": 10000}"#).expect("deserialize");
        assert_eq!(config.gas_limit, DEFAULT_GAS_LIMIT);
        assert_eq!(config.block_time_ms, 10000);
    }

    #[test]
    fn test_execution_config_clone_and_eq() {
        let config = ExecutionConfig { gas_limit: 999, block_time_ms: 42000 };
        assert_eq!(config, config.clone());
        assert_ne!(config, ExecutionConfig::default());
    }

    #[test]
    fn test_execution_config_old_block_time_alias() {
        // Old configs use "block_time" (seconds). The alias should still deserialize.
        let config: ExecutionConfig =
            serde_json::from_str(r#"{"block_time": 5}"#).expect("alias deserialize");
        assert_eq!(config.block_time_ms, 5);
    }
}
