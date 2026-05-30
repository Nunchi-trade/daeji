//! Mempool trait for transaction pool compatibility.

use std::collections::BTreeSet;

use kora_domain::{Tx, TxId};

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
    ///
    /// Accepts the full raw transactions from the finalized block so that
    /// implementations can advance sender nonces even for transactions that
    /// were not in the local pool (e.g., submitted to a different validator).
    /// This prevents stale nonce entries that could otherwise allow a
    /// same-nonce transaction to be accepted and re-proposed.
    fn prune(&self, txs: &[Tx]);

    /// Get the current number of pending transactions.
    fn len(&self) -> usize;

    /// Check if the mempool is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
