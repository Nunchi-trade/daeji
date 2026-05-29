//! Core trait abstractions for consensus components.

use std::collections::BTreeSet;

use alloy_primitives::B256;
use kora_domain::{ConsensusDigest, StateRoot, Tx, TxId as DomainTxId};
use kora_qmdb::ChangeSet;
use kora_traits::StateDb;

use crate::ConsensusError;

/// Transaction identifier type.
pub type TxId = DomainTxId;

/// Consensus digest type.
pub type Digest = ConsensusDigest;

/// A snapshot of execution state at a specific block.
#[derive(Clone, Debug)]
pub struct Snapshot<S> {
    /// Parent block digest.
    pub parent: Option<Digest>,
    /// State database at this point.
    pub state: S,
    /// Computed state root.
    pub state_root: StateRoot,
    /// Pending state changes not yet persisted.
    pub changes: ChangeSet,
    /// Transaction IDs included in this snapshot's block.
    pub tx_ids: BTreeSet<TxId>,
    /// Total gas consumed by the block that produced this snapshot.
    ///
    /// Used by the EIP-1559 base fee calculation for the *next* block:
    /// `calculate_base_fee(parent_base_fee, parent_gas_used, parent_gas_limit, …)`.
    /// Defaults to `0` (genesis / catch-up snapshots where gas accounting is
    /// not available).
    pub gas_used: u64,
    /// Base fee per gas that was in effect for the block that produced this
    /// snapshot.
    ///
    /// Used together with [`Self::gas_used`] by the EIP-1559 algorithm to
    /// derive the next block's base fee.  Defaults to
    /// [`kora_config::INITIAL_BASE_FEE`] for genesis and catch-up snapshots.
    pub base_fee_per_gas: u64,
}

impl<S> Snapshot<S> {
    /// Create a new snapshot.
    ///
    /// `gas_used` and `base_fee_per_gas` default to genesis values (`0` and
    /// `INITIAL_BASE_FEE` respectively).  Call [`Self::with_execution_fees`]
    /// after construction to set them for production blocks.
    pub const fn new(
        parent: Option<Digest>,
        state: S,
        state_root: StateRoot,
        changes: ChangeSet,
        tx_ids: BTreeSet<TxId>,
    ) -> Self {
        Self {
            parent,
            state,
            state_root,
            changes,
            tx_ids,
            gas_used: 0,
            base_fee_per_gas: kora_config::INITIAL_BASE_FEE,
        }
    }

    /// Attach execution fee metadata to the snapshot.
    ///
    /// This records the block's total gas usage and the base fee that was in
    /// effect, so that the *next* block can compute its EIP-1559 base fee
    /// from the parent snapshot.
    #[must_use]
    pub const fn with_execution_fees(mut self, gas_used: u64, base_fee_per_gas: u64) -> Self {
        self.gas_used = gas_used;
        self.base_fee_per_gas = base_fee_per_gas;
        self
    }
}

/// Mempool provides access to pending transactions for block building.
///
/// Implementations may use different ordering strategies (FIFO, priority, etc).
pub trait Mempool: Clone + Send + Sync + 'static {
    /// Insert a transaction into the mempool.
    ///
    /// Returns `true` if the transaction was newly inserted.
    fn insert(&self, tx: Tx) -> bool;

    /// Build a batch of transactions for inclusion in a block.
    ///
    /// `excluded` contains transaction IDs already included in pending ancestor blocks.
    /// `max_txs` limits the number of transactions returned.
    fn build(&self, max_txs: usize, excluded: &BTreeSet<TxId>) -> Vec<Tx>;

    /// Remove finalized transactions from the mempool.
    fn prune(&self, tx_ids: &[TxId]);

    /// Get the current number of pending transactions.
    fn len(&self) -> usize;

    /// Check if the mempool is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Manages execution snapshots keyed by block digest.
///
/// Snapshots allow replaying execution from any ancestor and computing
/// speculative state roots before finalization.
pub trait SnapshotStore<S: StateDb>: Clone + Send + Sync + 'static {
    /// Get a snapshot by digest.
    fn get(&self, digest: &Digest) -> Option<Snapshot<S>>;

    /// Insert a new snapshot.
    fn insert(&self, digest: Digest, snapshot: Snapshot<S>);

    /// Check if a digest has been persisted to the underlying state db.
    fn is_persisted(&self, digest: &Digest) -> bool;

    /// Mark a chain of digests as persisted.
    fn mark_persisted(&self, digests: &[Digest]);

    /// Get merged changes from the last persisted ancestor up to and including
    /// the given parent, then merge with the provided new changes.
    fn merged_changes(
        &self,
        parent: Digest,
        new_changes: ChangeSet,
    ) -> Result<ChangeSet, ConsensusError>;

    /// Get the chain of unpersisted digests and merged changes for persistence.
    fn changes_for_persist(
        &self,
        digest: Digest,
    ) -> Result<(Vec<Digest>, ChangeSet), ConsensusError>;
}

/// Tracks VRF seeds for prevrandao computation.
///
/// Seeds are derived from threshold VRF signatures during consensus and
/// used to populate the `prevrandao` field in subsequent blocks.
pub trait SeedTracker: Clone + Send + Sync + 'static {
    /// Get the seed for a given digest.
    fn get(&self, digest: &Digest) -> Option<B256>;

    /// Insert a seed for a digest.
    fn insert(&self, digest: Digest, seed: B256);
}

#[cfg(test)]
mod tests {
    use alloy_primitives::B256;
    use kora_domain::StateRoot;

    use super::*;

    #[test]
    fn snapshot_new() {
        let snapshot: Snapshot<()> =
            Snapshot::new(None, (), StateRoot(B256::ZERO), ChangeSet::new(), BTreeSet::new());
        assert!(snapshot.parent.is_none());
        assert_eq!(snapshot.state_root, StateRoot(B256::ZERO));
        assert_eq!(snapshot.gas_used, 0);
        assert_eq!(snapshot.base_fee_per_gas, kora_config::INITIAL_BASE_FEE);
    }

    #[test]
    fn snapshot_with_execution_fees() {
        let snapshot: Snapshot<()> =
            Snapshot::new(None, (), StateRoot(B256::ZERO), ChangeSet::new(), BTreeSet::new())
                .with_execution_fees(21_000, 2_000_000_000);
        assert_eq!(snapshot.gas_used, 21_000);
        assert_eq!(snapshot.base_fee_per_gas, 2_000_000_000);
    }
}
