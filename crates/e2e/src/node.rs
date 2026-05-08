//! Test node wrapper for e2e testing.

use alloy_primitives::{Address, B256, U256};
use kora_domain::{ConsensusDigest, StateRoot, Tx};
use kora_ledger::LedgerService;

/// Handle for interacting with a test node.
#[derive(Clone)]
pub struct TestNode {
    /// Node index in the test cluster.
    pub index: usize,
    /// Ledger service for state queries.
    ledger: LedgerService,
}

impl TestNode {
    /// Create a new test node handle.
    pub const fn new(index: usize, ledger: LedgerService) -> Self {
        Self { index, ledger }
    }

    /// Submit a transaction to this node's mempool.
    pub async fn submit_tx(&self, tx: Tx) -> bool {
        self.ledger.submit_tx(tx).await
    }

    /// Query the balance of an address at a specific block digest.
    pub async fn query_balance(&self, digest: ConsensusDigest, address: Address) -> Option<U256> {
        self.ledger.query_balance(digest, address).await
    }

    /// Query a storage slot at a specific block digest.
    pub async fn query_storage(
        &self,
        digest: ConsensusDigest,
        address: Address,
        slot: U256,
    ) -> Option<U256> {
        self.ledger.query_storage(digest, address, slot).await
    }

    /// Query the state root at a specific block digest.
    pub async fn query_state_root(&self, digest: ConsensusDigest) -> Option<StateRoot> {
        self.ledger.query_state_root(digest).await
    }

    /// Query the seed (prevrandao source) at a specific block digest.
    pub async fn query_seed(&self, digest: ConsensusDigest) -> Option<B256> {
        self.ledger.query_seed(digest).await
    }

    /// Get the genesis block from this node.
    pub fn genesis_block(&self) -> kora_domain::Block {
        self.ledger.genesis_block()
    }
}

impl std::fmt::Debug for TestNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestNode").field("index", &self.index).finish_non_exhaustive()
    }
}
