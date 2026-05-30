//! In-memory block index storage.

use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

use alloy_primitives::B256;
use parking_lot::RwLock;
use tracing::debug;

use crate::{
    filter::LogFilter,
    types::{IndexStats, IndexedBlock, IndexedLog, IndexedReceipt, IndexedTransaction},
};

/// Internal state guarded by a single [`RwLock`].
///
/// Grouping every map under one lock ensures that [`BlockIndex::insert_block`]
/// publishes blocks, transactions, receipts **and** logs atomically -- readers
/// can never observe a partially-indexed block.  It also eliminates the
/// five-lock convoy that previously serialised writers against concurrent RPC
/// readers (see issue #138).
#[derive(Debug, Default)]
struct IndexState {
    blocks_by_hash: HashMap<B256, IndexedBlock>,
    blocks_by_number: HashMap<u64, B256>,
    transactions: HashMap<B256, IndexedTransaction>,
    receipts: HashMap<B256, IndexedReceipt>,
    logs_by_block: HashMap<B256, Vec<IndexedLog>>,
}

/// In-memory storage for indexed blocks, transactions, receipts, and logs.
#[derive(Debug)]
pub struct BlockIndex {
    state: RwLock<IndexState>,
    head_block: AtomicU64,
}

impl Default for BlockIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockIndex {
    /// Maximum number of blocks to retain in the index.
    ///
    /// 10,000 blocks at 33 blocks/s is roughly 5 minutes of history.
    /// This must exceed 256 so the EVM `BLOCKHASH` opcode (served by
    /// [`Self::recent_block_hashes`]) always has a full window available.
    pub const MAX_RETAINED_BLOCKS: u64 = 10_000;

    /// Maximum number of log entries returned by a single [`Self::get_logs`]
    /// call.
    ///
    /// Queries matching more logs than this limit are truncated. This
    /// prevents memory exhaustion when a broad filter matches millions of
    /// events within the allowed block range (e.g. ERC-20 `Transfer`
    /// across 10,000 blocks). The value matches Geth's default cap.
    pub const MAX_LOG_RESULTS: usize = 10_000;

    /// Creates a new empty block index.
    #[must_use]
    pub fn new() -> Self {
        Self { state: RwLock::new(IndexState::default()), head_block: AtomicU64::new(0) }
    }

    /// Inserts a block with its transactions and receipts into the index.
    ///
    /// All five maps are updated under a **single** write-lock acquisition,
    /// guaranteeing atomic visibility to concurrent readers and eliminating
    /// the previous 5-lock convoy (issue #138).
    pub fn insert_block(
        &self,
        block: IndexedBlock,
        txs: Vec<IndexedTransaction>,
        receipts: Vec<IndexedReceipt>,
    ) {
        let block_hash = block.hash;
        let block_number = block.number;

        debug!(number = block_number, hash = %block_hash, txs = txs.len(), "indexing block");

        // Extract logs from receipts before consuming them.
        let all_logs: Vec<IndexedLog> = receipts.iter().flat_map(|r| r.logs.clone()).collect();

        // Single write-lock for all maps.
        {
            let mut st = self.state.write();
            st.blocks_by_hash.insert(block_hash, block);
            st.blocks_by_number.insert(block_number, block_hash);
            for tx in txs {
                st.transactions.insert(tx.hash, tx);
            }
            for receipt in receipts {
                st.receipts.insert(receipt.transaction_hash, receipt);
            }
            st.logs_by_block.insert(block_hash, all_logs);
        }

        // Update head atomically (lock-free).
        let mut current = self.head_block.load(Ordering::Acquire);
        while block_number > current {
            match self.head_block.compare_exchange_weak(
                current,
                block_number,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(c) => current = c,
            }
        }
    }

    /// Removes all index entries for blocks with `number < min_block_number`.
    ///
    /// This bounds memory by evicting blocks, transactions, receipts, and logs
    /// that are older than the retention window.
    ///
    /// After removal, each map is shrunk via [`HashMap::shrink_to_fit`] so that
    /// the backing allocation tracks the actual entry count instead of
    /// retaining its peak capacity indefinitely (issue #140).
    pub fn prune_before(&self, min_block_number: u64) {
        let mut st = self.state.write();

        // Collect block hashes and their tx hashes to prune.
        let blocks_to_remove: Vec<(u64, B256)> = st
            .blocks_by_number
            .iter()
            .filter(|(num, _)| **num < min_block_number)
            .map(|(num, hash)| (*num, *hash))
            .collect();

        if blocks_to_remove.is_empty() {
            return;
        }

        let tx_hashes: Vec<B256> = blocks_to_remove
            .iter()
            .filter_map(|(_, h)| st.blocks_by_hash.get(h))
            .flat_map(|b| b.transaction_hashes.iter().copied())
            .collect();

        // Remove block-level entries.
        for &(num, hash) in &blocks_to_remove {
            st.blocks_by_number.remove(&num);
            st.blocks_by_hash.remove(&hash);
            st.logs_by_block.remove(&hash);
        }

        // Remove transaction-level entries.
        for h in &tx_hashes {
            st.transactions.remove(h);
            st.receipts.remove(h);
        }

        // Shrink all maps so the bucket arrays track retained size
        // instead of keeping peak capacity allocated (issue #140).
        st.blocks_by_number.shrink_to_fit();
        st.blocks_by_hash.shrink_to_fit();
        st.logs_by_block.shrink_to_fit();
        st.transactions.shrink_to_fit();
        st.receipts.shrink_to_fit();

        debug!(
            min_block_number,
            pruned_blocks = blocks_to_remove.len(),
            pruned_txs = tx_hashes.len(),
            "pruned old index entries",
        );
    }

    /// Gets a block by its hash.
    pub fn get_block_by_hash(&self, hash: &B256) -> Option<IndexedBlock> {
        self.state.read().blocks_by_hash.get(hash).cloned()
    }

    /// Gets a block by its number.
    pub fn get_block_by_number(&self, number: u64) -> Option<IndexedBlock> {
        let st = self.state.read();
        let hash = st.blocks_by_number.get(&number)?;
        st.blocks_by_hash.get(hash).cloned()
    }

    /// Gets a transaction by its hash.
    pub fn get_transaction(&self, hash: &B256) -> Option<IndexedTransaction> {
        self.state.read().transactions.get(hash).cloned()
    }

    /// Gets all indexed transactions for a block in insertion order.
    ///
    /// Uses the block's `transaction_hashes` list for O(txs_in_block) targeted
    /// lookups instead of scanning the entire transaction map (issue #037).
    pub fn get_transactions_for_block(&self, block_hash: &B256) -> Vec<IndexedTransaction> {
        let st = self.state.read();
        let Some(block) = st.blocks_by_hash.get(block_hash) else {
            return Vec::new();
        };
        block
            .transaction_hashes
            .iter()
            .filter_map(|hash| st.transactions.get(hash).cloned())
            .collect()
    }

    /// Gets a receipt by its transaction hash.
    pub fn get_receipt(&self, hash: &B256) -> Option<IndexedReceipt> {
        self.state.read().receipts.get(hash).cloned()
    }

    /// Returns the current head block number.
    #[must_use]
    pub fn head_block_number(&self) -> u64 {
        self.head_block.load(Ordering::Acquire)
    }

    /// Gets logs matching the given filter.
    pub fn get_logs(&self, filter: &LogFilter) -> Vec<IndexedLog> {
        let head = self.head_block_number();
        let from_block = filter.from_block.unwrap_or(0);
        let to_block = filter.to_block.unwrap_or(head).min(head);

        let mut result = Vec::new();

        let st = self.state.read();

        for block_num in from_block..=to_block {
            let Some(block_hash) = st.blocks_by_number.get(&block_num) else {
                continue;
            };

            let Some(logs) = st.logs_by_block.get(block_hash) else {
                continue;
            };

            for log in logs {
                if !Self::matches_filter(log, filter) {
                    continue;
                }
                result.push(log.clone());
            }
        }

        result
    }

    /// Returns the total number of indexed blocks.
    #[must_use]
    pub fn block_count(&self) -> usize {
        self.state.read().blocks_by_hash.len()
    }

    /// Returns the total number of indexed transactions.
    #[must_use]
    pub fn transaction_count(&self) -> usize {
        self.state.read().transactions.len()
    }

    /// Returns the total number of indexed receipts.
    #[must_use]
    pub fn receipt_count(&self) -> usize {
        self.state.read().receipts.len()
    }

    /// Returns true if the index is empty (no blocks indexed).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.state.read().blocks_by_hash.is_empty()
    }

    /// Returns statistics about the index.
    #[must_use]
    pub fn stats(&self) -> IndexStats {
        IndexStats {
            block_count: self.block_count(),
            transaction_count: self.transaction_count(),
            receipt_count: self.receipt_count(),
            head_block_number: self.head_block_number(),
        }
    }

    /// Returns up to 256 recent block hashes keyed by block number, looking
    /// backwards from `head` (exclusive). Used to populate the BLOCKHASH opcode
    /// context.
    #[must_use]
    pub fn recent_block_hashes(&self, head: u64) -> HashMap<u64, B256> {
        let st = self.state.read();
        let depth = head.min(256);
        let mut hashes = HashMap::with_capacity(depth as usize);
        for num in head.saturating_sub(depth)..head {
            if let Some(hash) = st.blocks_by_number.get(&num) {
                hashes.insert(num, *hash);
            }
        }
        hashes
    }

    fn matches_filter(log: &IndexedLog, filter: &LogFilter) -> bool {
        if let Some(addresses) = &filter.address
            && !addresses.contains(&log.address)
        {
            return false;
        }

        for (i, topic_filter) in filter.topics.iter().enumerate() {
            if let Some(allowed_topics) = topic_filter {
                match log.topics.get(i) {
                    Some(log_topic) if allowed_topics.contains(log_topic) => {}
                    _ => return false,
                }
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, B256, Bloom, Bytes, U256};

    use super::*;

    fn create_test_block(number: u64, hash: B256) -> IndexedBlock {
        IndexedBlock {
            hash,
            number,
            parent_hash: B256::ZERO,
            state_root: B256::ZERO,
            transactions_root: B256::ZERO,
            receipts_root: B256::ZERO,
            timestamp: 1000 + number,
            gas_limit: 30_000_000,
            gas_used: 21_000,
            base_fee_per_gas: Some(1_000_000_000),
            mix_hash: B256::ZERO,
            logs_bloom: Bloom::ZERO,
            size: 508,
            transaction_hashes: vec![],
        }
    }

    fn create_test_tx(hash: B256, block_hash: B256, block_number: u64) -> IndexedTransaction {
        IndexedTransaction {
            hash,
            block_hash,
            block_number,
            index: 0,
            from: Address::ZERO,
            to: Some(Address::ZERO),
            value: U256::ZERO,
            gas_limit: 21_000,
            gas_price: 1_000_000_000,
            tx_type: 0,
            chain_id: Some(1337),
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            v: 27,
            r: U256::from(1),
            s: U256::from(2),
            input: Bytes::new(),
            nonce: 0,
        }
    }

    fn create_test_receipt(tx_hash: B256, block_hash: B256, block_number: u64) -> IndexedReceipt {
        IndexedReceipt {
            transaction_hash: tx_hash,
            block_hash,
            block_number,
            transaction_index: 0,
            from: Address::ZERO,
            to: Some(Address::ZERO),
            cumulative_gas_used: 21_000,
            gas_used: 21_000,
            contract_address: None,
            logs: vec![],
            logs_bloom: Bloom::ZERO,
            tx_type: 0,
            effective_gas_price: 1_000_000_000,
            status: true,
        }
    }

    #[test]
    fn test_insert_and_get_block() {
        let index = BlockIndex::new();
        let block_hash = B256::repeat_byte(1);
        let block = create_test_block(1, block_hash);

        index.insert_block(block, vec![], vec![]);

        let retrieved = index.get_block_by_hash(&block_hash).unwrap();
        assert_eq!(retrieved.number, 1);
        assert_eq!(retrieved.hash, block_hash);

        let by_number = index.get_block_by_number(1).unwrap();
        assert_eq!(by_number.hash, block_hash);
    }

    #[test]
    fn test_insert_and_get_transaction() {
        let index = BlockIndex::new();
        let block_hash = B256::repeat_byte(1);
        let tx_hash = B256::repeat_byte(2);
        let block = create_test_block(1, block_hash);
        let tx = create_test_tx(tx_hash, block_hash, 1);
        let receipt = create_test_receipt(tx_hash, block_hash, 1);

        index.insert_block(block, vec![tx], vec![receipt]);

        let retrieved_tx = index.get_transaction(&tx_hash).unwrap();
        assert_eq!(retrieved_tx.hash, tx_hash);

        let retrieved_receipt = index.get_receipt(&tx_hash).unwrap();
        assert_eq!(retrieved_receipt.transaction_hash, tx_hash);
    }

    #[test]
    fn test_head_block_number() {
        let index = BlockIndex::new();
        assert_eq!(index.head_block_number(), 0);

        index.insert_block(create_test_block(5, B256::repeat_byte(5)), vec![], vec![]);
        assert_eq!(index.head_block_number(), 5);

        index.insert_block(create_test_block(3, B256::repeat_byte(3)), vec![], vec![]);
        assert_eq!(index.head_block_number(), 5);

        index.insert_block(create_test_block(10, B256::repeat_byte(10)), vec![], vec![]);
        assert_eq!(index.head_block_number(), 10);
    }

    #[test]
    fn test_get_logs_with_filter() {
        let index = BlockIndex::new();
        let block_hash = B256::repeat_byte(1);
        let contract_addr = Address::repeat_byte(0xAB);
        let topic = B256::repeat_byte(0xCD);

        let log = IndexedLog {
            address: contract_addr,
            topics: vec![topic],
            data: Bytes::new(),
            log_index: 0,
            block_number: 1,
            block_hash,
            transaction_hash: B256::repeat_byte(2),
            transaction_index: 0,
        };

        let receipt = IndexedReceipt {
            transaction_hash: B256::repeat_byte(2),
            block_hash,
            block_number: 1,
            transaction_index: 0,
            from: Address::ZERO,
            to: None,
            cumulative_gas_used: 21_000,
            gas_used: 21_000,
            contract_address: None,
            logs: vec![log],
            logs_bloom: Bloom::ZERO,
            tx_type: 0,
            effective_gas_price: 1_000_000_000,
            status: true,
        };

        index.insert_block(create_test_block(1, block_hash), vec![], vec![receipt]);

        let filter = LogFilter::new().address(vec![contract_addr]);
        let logs = index.get_logs(&filter);
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].address, contract_addr);

        let filter = LogFilter::new().topic(0, vec![topic]);
        let logs = index.get_logs(&filter);
        assert_eq!(logs.len(), 1);

        let filter = LogFilter::new().address(vec![Address::repeat_byte(0xFF)]);
        let logs = index.get_logs(&filter);
        assert!(logs.is_empty());
    }

    #[test]
    fn test_is_empty() {
        let index = BlockIndex::new();
        assert!(index.is_empty());

        index.insert_block(create_test_block(1, B256::repeat_byte(1)), vec![], vec![]);
        assert!(!index.is_empty());
    }

    #[test]
    fn test_block_count() {
        let index = BlockIndex::new();
        assert_eq!(index.block_count(), 0);

        index.insert_block(create_test_block(1, B256::repeat_byte(1)), vec![], vec![]);
        assert_eq!(index.block_count(), 1);

        index.insert_block(create_test_block(2, B256::repeat_byte(2)), vec![], vec![]);
        assert_eq!(index.block_count(), 2);
    }

    #[test]
    fn test_transaction_count() {
        let index = BlockIndex::new();
        assert_eq!(index.transaction_count(), 0);

        let block_hash = B256::repeat_byte(1);
        let tx1 = create_test_tx(B256::repeat_byte(2), block_hash, 1);
        let tx2 = create_test_tx(B256::repeat_byte(3), block_hash, 1);

        index.insert_block(create_test_block(1, block_hash), vec![tx1, tx2], vec![]);
        assert_eq!(index.transaction_count(), 2);
    }

    #[test]
    fn test_receipt_count() {
        let index = BlockIndex::new();
        assert_eq!(index.receipt_count(), 0);

        let block_hash = B256::repeat_byte(1);
        let tx_hash = B256::repeat_byte(2);
        let receipt = create_test_receipt(tx_hash, block_hash, 1);

        index.insert_block(create_test_block(1, block_hash), vec![], vec![receipt]);
        assert_eq!(index.receipt_count(), 1);
    }

    #[test]
    fn test_stats() {
        let index = BlockIndex::new();

        let stats = index.stats();
        assert_eq!(stats.block_count, 0);
        assert_eq!(stats.transaction_count, 0);
        assert_eq!(stats.receipt_count, 0);
        assert_eq!(stats.head_block_number, 0);

        let block_hash = B256::repeat_byte(1);
        let tx_hash = B256::repeat_byte(2);
        let tx = create_test_tx(tx_hash, block_hash, 5);
        let receipt = create_test_receipt(tx_hash, block_hash, 5);

        index.insert_block(create_test_block(5, block_hash), vec![tx], vec![receipt]);

        let stats = index.stats();
        assert_eq!(stats.block_count, 1);
        assert_eq!(stats.transaction_count, 1);
        assert_eq!(stats.receipt_count, 1);
        assert_eq!(stats.head_block_number, 5);
    }

    #[test]
    fn test_recent_block_hashes() {
        let index = BlockIndex::new();

        // Insert blocks 0..5
        for i in 0..5 {
            index.insert_block(create_test_block(i, B256::repeat_byte(i as u8)), vec![], vec![]);
        }

        // Head=5 should return hashes for blocks 0..5
        let hashes = index.recent_block_hashes(5);
        assert_eq!(hashes.len(), 5);
        for i in 0..5 {
            assert_eq!(hashes[&i], B256::repeat_byte(i as u8));
        }

        // Head=0 should return empty
        let hashes = index.recent_block_hashes(0);
        assert!(hashes.is_empty());

        // Head=3 should return blocks 0..3
        let hashes = index.recent_block_hashes(3);
        assert_eq!(hashes.len(), 3);
        assert!(hashes.contains_key(&0));
        assert!(hashes.contains_key(&1));
        assert!(hashes.contains_key(&2));
        assert!(!hashes.contains_key(&3));
    }

    #[test]
    fn test_prune_before_removes_old_blocks() {
        let index = BlockIndex::new();

        // Insert blocks 1..=5, each with one tx and one receipt.
        for i in 1..=5u64 {
            let block_hash = B256::repeat_byte(i as u8);
            let tx_hash = B256::repeat_byte((100 + i) as u8);
            let mut block = create_test_block(i, block_hash);
            block.transaction_hashes = vec![tx_hash];
            let tx = create_test_tx(tx_hash, block_hash, i);
            let receipt = create_test_receipt(tx_hash, block_hash, i);
            index.insert_block(block, vec![tx], vec![receipt]);
        }

        assert_eq!(index.block_count(), 5);
        assert_eq!(index.transaction_count(), 5);
        assert_eq!(index.receipt_count(), 5);

        // Prune everything below block 3 (removes blocks 1, 2).
        index.prune_before(3);

        assert_eq!(index.block_count(), 3);
        assert_eq!(index.transaction_count(), 3);
        assert_eq!(index.receipt_count(), 3);

        // Blocks 1 and 2 are gone.
        assert!(index.get_block_by_number(1).is_none());
        assert!(index.get_block_by_number(2).is_none());

        // Block 3, 4, 5 remain.
        assert!(index.get_block_by_number(3).is_some());
        assert!(index.get_block_by_number(4).is_some());
        assert!(index.get_block_by_number(5).is_some());

        // Head block unchanged.
        assert_eq!(index.head_block_number(), 5);

        // Pruned tx hashes are gone.
        assert!(index.get_transaction(&B256::repeat_byte(101)).is_none());
        assert!(index.get_transaction(&B256::repeat_byte(102)).is_none());

        // Retained tx hashes still present.
        assert!(index.get_transaction(&B256::repeat_byte(103)).is_some());
    }

    #[test]
    fn test_prune_before_noop_when_nothing_to_prune() {
        let index = BlockIndex::new();

        index.insert_block(create_test_block(5, B256::repeat_byte(5)), vec![], vec![]);

        // min_block_number <= all stored blocks: should be a no-op.
        index.prune_before(1);
        assert_eq!(index.block_count(), 1);

        // min_block_number = 0: also a no-op.
        index.prune_before(0);
        assert_eq!(index.block_count(), 1);
    }

    #[test]
    fn test_prune_preserves_recent_block_hashes_window() {
        let index = BlockIndex::new();

        // Insert 300 blocks (more than the 256 BLOCKHASH window).
        for i in 0..300u64 {
            index.insert_block(
                create_test_block(i, B256::repeat_byte((i % 256) as u8)),
                vec![],
                vec![],
            );
        }

        // Prune old blocks, keeping only 270+ (simulates a retention window).
        index.prune_before(270);

        // recent_block_hashes(300) looks back 256 blocks (44..300).
        // Only blocks 270..300 remain, so we should get exactly 30 entries.
        let hashes = index.recent_block_hashes(300);
        assert_eq!(hashes.len(), 30);
    }

    #[test]
    fn test_get_transactions_for_block_uses_per_block_index() {
        let index = BlockIndex::new();

        let block_hash_a = B256::repeat_byte(0xAA);
        let block_hash_b = B256::repeat_byte(0xBB);

        let tx1 = B256::repeat_byte(1);
        let tx2 = B256::repeat_byte(2);
        let tx3 = B256::repeat_byte(3);

        // Block A has tx1 and tx2 (in that order).
        let mut block_a = create_test_block(1, block_hash_a);
        block_a.transaction_hashes = vec![tx1, tx2];

        let mut t1 = create_test_tx(tx1, block_hash_a, 1);
        t1.index = 0;
        let mut t2 = create_test_tx(tx2, block_hash_a, 1);
        t2.index = 1;

        index.insert_block(block_a, vec![t1, t2], vec![]);

        // Block B has tx3.
        let mut block_b = create_test_block(2, block_hash_b);
        block_b.transaction_hashes = vec![tx3];
        let mut t3 = create_test_tx(tx3, block_hash_b, 2);
        t3.index = 0;
        index.insert_block(block_b, vec![t3], vec![]);

        // Query for block A should return only tx1 and tx2, in order.
        let txs_a = index.get_transactions_for_block(&block_hash_a);
        assert_eq!(txs_a.len(), 2);
        assert_eq!(txs_a[0].hash, tx1);
        assert_eq!(txs_a[1].hash, tx2);

        // Query for block B should return only tx3.
        let txs_b = index.get_transactions_for_block(&block_hash_b);
        assert_eq!(txs_b.len(), 1);
        assert_eq!(txs_b[0].hash, tx3);

        // Query for unknown block returns empty.
        let txs_none = index.get_transactions_for_block(&B256::repeat_byte(0xFF));
        assert!(txs_none.is_empty());
    }

    #[test]
    fn test_prune_shrinks_maps() {
        let index = BlockIndex::new();

        // Insert 100 blocks with one tx each.
        for i in 0..100u64 {
            let block_hash = B256::from([i as u8; 32]);
            let tx_hash = B256::from([(i + 100) as u8; 32]);
            let mut block = create_test_block(i, block_hash);
            block.transaction_hashes = vec![tx_hash];
            let tx = create_test_tx(tx_hash, block_hash, i);
            let receipt = create_test_receipt(tx_hash, block_hash, i);
            index.insert_block(block, vec![tx], vec![receipt]);
        }

        assert_eq!(index.block_count(), 100);

        // Prune down to 10 blocks.
        index.prune_before(90);

        assert_eq!(index.block_count(), 10);
        assert_eq!(index.transaction_count(), 10);
        assert_eq!(index.receipt_count(), 10);

        // Verify the maps' capacities shrunk. After shrink_to_fit() the
        // capacity should be <= some reasonable multiple of the length.
        // HashMap typically rounds up to a power of 2, so capacity <= 4*len
        // is a generous bound.
        let st = index.state.read();
        assert!(
            st.blocks_by_hash.capacity() <= 4 * st.blocks_by_hash.len(),
            "blocks_by_hash capacity {} should be <= 4 * len {}",
            st.blocks_by_hash.capacity(),
            st.blocks_by_hash.len(),
        );
        assert!(
            st.transactions.capacity() <= 4 * st.transactions.len(),
            "transactions capacity {} should be <= 4 * len {}",
            st.transactions.capacity(),
            st.transactions.len(),
        );
    }
}
