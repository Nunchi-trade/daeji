//! Consensus reporters for Kora nodes.
#![doc = include_str!("../README.md")]
#![doc(issue_tracker_base_url = "https://github.com/refcell/kora/issues/")]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use std::{fmt, marker::PhantomData, sync::Arc};

use alloy_consensus::{Transaction as _, TxEnvelope};
use alloy_eips::eip2718::Decodable2718;
use alloy_primitives::{B256, Bytes, keccak256};
use commonware_consensus::{
    Block as _, Reporter,
    marshal::Update,
    simplex::{
        scheme::bls12381_threshold::vrf::{Scheme, Seedable as _},
        types::Activity,
    },
};
use commonware_cryptography::{Committable as _, bls12381::primitives::variant::Variant};
use commonware_runtime::{Spawner as _, tokio};
use commonware_utils::acknowledgement::Acknowledgement as _;
use kora_consensus::BlockExecution;
use kora_domain::{Block, ConsensusDigest, PublicKey};
use kora_executor::{BlockContext, BlockExecutor, ExecutionOutcome};
use kora_indexer::{BlockIndex, IndexedBlock, IndexedLog, IndexedReceipt, IndexedTransaction};
use kora_ledger::LedgerService;
use kora_overlay::OverlayState;
use kora_qmdb_ledger::QmdbState;
use kora_rpc::NodeState;
use kora_txpool::recover_sender_from_envelope;
use tracing::{error, trace, warn};

/// Provides block execution context for finalized block verification.
pub trait BlockContextProvider: Clone + Send + Sync + 'static {
    /// Build a block execution context for the provided block.
    fn context(&self, block: &Block) -> BlockContext;
}

/// Helper function for SeedReporter::report that owns all its inputs.
async fn seed_report_inner<V: Variant>(
    state: LedgerService,
    activity: Activity<Scheme<PublicKey, V>, ConsensusDigest>,
) {
    match activity {
        Activity::Notarization(notarization) => {
            state
                .set_seed(
                    notarization.proposal.payload,
                    SeedReporter::<V>::hash_seed(notarization.seed()),
                )
                .await;
        }
        Activity::Finalization(finalization) => {
            state
                .set_seed(
                    finalization.proposal.payload,
                    SeedReporter::<V>::hash_seed(finalization.seed()),
                )
                .await;
        }
        _ => {}
    }
}

#[derive(Clone)]
/// Tracks simplex activity to store seed hashes for future proposals.
pub struct SeedReporter<V> {
    /// Ledger service that keeps per-digest seeds and snapshots.
    state: LedgerService,
    /// Marker indicating the variant for the threshold scheme in use.
    _variant: PhantomData<V>,
}

impl<V> fmt::Debug for SeedReporter<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SeedReporter").finish_non_exhaustive()
    }
}

impl<V> SeedReporter<V> {
    /// Create a new seed reporter for the provided ledger service.
    pub const fn new(state: LedgerService) -> Self {
        Self { state, _variant: PhantomData }
    }

    fn hash_seed(seed: impl commonware_codec::Encode) -> B256 {
        keccak256(seed.encode())
    }
}

impl<V> Reporter for SeedReporter<V>
where
    V: Variant,
{
    type Activity = Activity<Scheme<PublicKey, V>, ConsensusDigest>;

    fn report(&mut self, activity: Self::Activity) -> impl std::future::Future<Output = ()> + Send {
        let state = self.state.clone();
        async move {
            seed_report_inner(state, activity).await;
        }
    }
}

async fn handle_finalized_update<E, P>(
    state: LedgerService,
    context: tokio::Context,
    executor: E,
    provider: P,
    block_index: Option<Arc<BlockIndex>>,
    update: Update<Block>,
) where
    E: BlockExecutor<OverlayState<QmdbState>, Tx = Bytes>,
    P: BlockContextProvider,
{
    let Update::Block(block, ack) = update else { return };

    let digest = block.commitment();
    let parent_digest = block.parent();
    let snapshot_exists = state.query_state_root(digest).await.is_some();
    let want_index = block_index.is_some();

    // Execute when we have no cached snapshot, OR when an index is attached and we need
    // receipts to populate it. The cached path is hit on the proposing/verifying validator
    // for blocks they have just produced; in that case we re-execute purely to recover
    // receipts.
    //
    // TODO: cache receipts in the snapshot store so we don't pay 2x execution on the
    // proposer/verifier paths.
    let mut indexable: Option<(BlockContext, ExecutionOutcome)> = None;
    if !snapshot_exists || want_index {
        let Some(parent_snapshot) = state.parent_snapshot(parent_digest).await else {
            error!(?digest, ?parent_digest, "missing parent snapshot for finalized block");
            ack.acknowledge();
            return;
        };
        let block_context = provider.context(&block);
        let execution = match BlockExecution::execute(
            &parent_snapshot,
            &executor,
            &block_context,
            &block.txs,
        )
        .await
        {
            Ok(result) => result,
            Err(err) => {
                error!(?digest, error = ?err, "failed to execute finalized block");
                ack.acknowledge();
                return;
            }
        };

        if !snapshot_exists {
            trace!(?digest, "missing snapshot for finalized block; re-executing");
            let merged_changes =
                parent_snapshot.state.merge_changes(execution.outcome.changes.clone());
            let state_root = match state
                .compute_root_from_store(parent_digest, execution.outcome.changes.clone())
                .await
            {
                Ok(root) => root,
                Err(err) => {
                    error!(?digest, error = ?err, "failed to compute qmdb root");
                    ack.acknowledge();
                    return;
                }
            };
            if state_root != block.state_root {
                warn!(
                    ?digest,
                    expected = ?block.state_root,
                    computed = ?state_root,
                    "state root mismatch for finalized block"
                );
                ack.acknowledge();
                return;
            }
            let next_state = OverlayState::new(parent_snapshot.state.base(), merged_changes);
            state
                .insert_snapshot(
                    digest,
                    parent_digest,
                    next_state,
                    state_root,
                    execution.outcome.changes.clone(),
                    &block.txs,
                )
                .await;
        }

        if want_index {
            indexable = Some((block_context, execution.outcome));
        }
    } else {
        trace!(?digest, "using cached snapshot for finalized block");
    }

    let persist_state = state.clone();
    let persist_handle = context
        .shared(true)
        .spawn(move |_| async move { persist_state.persist_snapshot(digest).await });
    let persist_result = match persist_handle.await {
        Ok(result) => result,
        Err(err) => {
            error!(?digest, error = ?err, "persist task failed");
            ack.acknowledge();
            return;
        }
    };
    if let Err(err) = persist_result {
        error!(?digest, error = ?err, "failed to persist finalized block");
        ack.acknowledge();
        return;
    }
    state.prune_mempool(&block.txs).await;

    // Index the block once the ledger snapshot is durably persisted. The index is a
    // derived view; tx-decode failures are warnings and the rest of the block is still
    // indexed.
    if let (Some(index), Some((block_context, outcome))) = (block_index, indexable) {
        index_finalized_block(&index, &block, &block_context, &outcome);
    }

    // Marshal waits for the application to acknowledge processing before advancing the
    // delivery floor. Without this, the node can stall on finalized block delivery.
    ack.acknowledge();
}

/// Convert a finalized block plus its execution outcome into indexer entries and insert
/// them. Per-tx decode failures are logged and skipped so a single malformed tx doesn't
/// block indexing of the rest of the block.
fn index_finalized_block(
    index: &BlockIndex,
    block: &Block,
    block_context: &BlockContext,
    outcome: &ExecutionOutcome,
) {
    let block_hash = block.id().0;
    let block_number = block.height;

    let mut indexed_txs = Vec::with_capacity(block.txs.len());
    let mut indexed_receipts = Vec::with_capacity(outcome.receipts.len());
    let mut transaction_hashes = Vec::with_capacity(block.txs.len());
    let mut log_index: u64 = 0;

    for (idx, (tx, exec_receipt)) in block.txs.iter().zip(outcome.receipts.iter()).enumerate() {
        let envelope = match TxEnvelope::decode_2718(&mut tx.bytes.as_ref()) {
            Ok(env) => env,
            Err(err) => {
                warn!(?block_hash, idx, error = ?err, "failed to decode tx for indexing");
                continue;
            }
        };
        let sender = match recover_sender_from_envelope(&envelope) {
            Ok(addr) => addr,
            Err(err) => {
                warn!(?block_hash, idx, error = ?err, "failed to recover sender for indexing");
                continue;
            }
        };

        let tx_hash = exec_receipt.tx_hash;
        let to = envelope.to();
        let gas_price = match &envelope {
            TxEnvelope::Legacy(t) => t.tx().gas_price,
            TxEnvelope::Eip2930(t) => t.tx().gas_price,
            TxEnvelope::Eip1559(t) => t.tx().max_fee_per_gas,
            TxEnvelope::Eip4844(t) => t.tx().tx().max_fee_per_gas,
            TxEnvelope::Eip7702(t) => t.tx().max_fee_per_gas,
        };

        transaction_hashes.push(tx_hash);
        indexed_txs.push(IndexedTransaction {
            hash: tx_hash,
            block_hash,
            block_number,
            index: idx as u64,
            from: sender,
            to,
            value: envelope.value(),
            gas_limit: envelope.gas_limit(),
            gas_price,
            input: envelope.input().clone(),
            nonce: envelope.nonce(),
        });

        let mut receipt_logs = Vec::with_capacity(exec_receipt.receipt.logs.len());
        for log in &exec_receipt.receipt.logs {
            receipt_logs.push(IndexedLog {
                address: log.address,
                topics: log.data.topics().to_vec(),
                data: log.data.data.clone(),
                log_index,
            });
            log_index += 1;
        }

        indexed_receipts.push(IndexedReceipt {
            transaction_hash: tx_hash,
            block_hash,
            block_number,
            transaction_index: idx as u64,
            from: sender,
            to,
            cumulative_gas_used: exec_receipt.cumulative_gas_used(),
            gas_used: exec_receipt.gas_used,
            contract_address: exec_receipt.contract_address,
            logs: receipt_logs,
            status: exec_receipt.success(),
        });
    }

    let indexed_block = IndexedBlock {
        hash: block_hash,
        number: block_number,
        parent_hash: block.parent.0,
        state_root: block.state_root.0,
        timestamp: block_context.header.timestamp,
        gas_limit: block_context.header.gas_limit,
        gas_used: outcome.gas_used,
        base_fee_per_gas: block_context.header.base_fee_per_gas,
        transaction_hashes,
    };

    index.insert_block(indexed_block, indexed_txs, indexed_receipts);
}

#[derive(Clone)]
/// Persists finalized blocks.
pub struct FinalizedReporter<E, P> {
    /// Ledger service used to verify blocks and persist snapshots.
    state: LedgerService,
    /// Tokio context used to schedule blocking work.
    context: tokio::Context,
    /// Block executor used to replay finalized blocks.
    executor: E,
    /// Provider that builds block execution context.
    provider: P,
    /// Optional in-memory block index populated as finalized blocks land. When set, this
    /// reporter will execute the block (re-executing if a snapshot was already cached) so
    /// receipts can be fed to the index for `eth_getBlockByNumber` / `eth_getTransactionReceipt`.
    block_index: Option<Arc<BlockIndex>>,
}

impl<E, P> fmt::Debug for FinalizedReporter<E, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FinalizedReporter").finish_non_exhaustive()
    }
}

impl<E, P> FinalizedReporter<E, P>
where
    E: BlockExecutor<OverlayState<QmdbState>, Tx = Bytes>,
    P: BlockContextProvider,
{
    /// Create a new finalized reporter without a block index attached.
    pub const fn new(
        state: LedgerService,
        context: tokio::Context,
        executor: E,
        provider: P,
    ) -> Self {
        Self { state, context, executor, provider, block_index: None }
    }

    /// Attach a block index that this reporter will populate as blocks finalize.
    #[must_use]
    pub fn with_block_index(mut self, block_index: Arc<BlockIndex>) -> Self {
        self.block_index = Some(block_index);
        self
    }
}

impl<E, P> Reporter for FinalizedReporter<E, P>
where
    E: BlockExecutor<OverlayState<QmdbState>, Tx = Bytes>,
    P: BlockContextProvider,
{
    type Activity = Update<Block>;

    fn report(&mut self, update: Self::Activity) -> impl std::future::Future<Output = ()> + Send {
        let state = self.state.clone();
        let context = self.context.clone();
        let executor = self.executor.clone();
        let provider = self.provider.clone();
        let block_index = self.block_index.clone();
        async move {
            handle_finalized_update(state, context, executor, provider, block_index, update).await;
        }
    }
}

/// Reporter that updates RPC-visible node state from consensus activity.
///
/// This reporter tracks:
/// - Current view number (from notarizations)
/// - Finalized block count
/// - Nullified round count
#[derive(Clone)]
pub struct NodeStateReporter<S> {
    /// RPC node state to update.
    state: NodeState,
    /// Marker for the signing scheme.
    _scheme: PhantomData<S>,
}

impl<S> fmt::Debug for NodeStateReporter<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeStateReporter").finish_non_exhaustive()
    }
}

impl<S> NodeStateReporter<S> {
    /// Create a new node state reporter.
    pub const fn new(state: NodeState) -> Self {
        Self { state, _scheme: PhantomData }
    }
}

impl<S> Reporter for NodeStateReporter<S>
where
    S: commonware_cryptography::certificate::Scheme + Clone + Send + 'static,
{
    type Activity = Activity<S, ConsensusDigest>;

    fn report(&mut self, activity: Self::Activity) -> impl std::future::Future<Output = ()> + Send {
        match &activity {
            Activity::Notarization(n) => {
                self.state.set_view(n.proposal.round.view().get());
            }
            Activity::Finalization(f) => {
                self.state.set_view(f.proposal.round.view().get());
                self.state.inc_finalized();
            }
            Activity::Nullification(_) => {
                self.state.inc_nullified();
            }
            _ => {}
        }
        async {}
    }
}

#[cfg(test)]
mod indexer_tests {
    use alloy_consensus::Header;
    use alloy_primitives::B256;
    use kora_domain::{BlockId, StateRoot};
    use kora_executor::{BlockContext, ExecutionOutcome};
    use kora_indexer::BlockIndex;

    use super::{Block, index_finalized_block};

    fn make_block(height: u64, parent: B256) -> Block {
        Block {
            parent: BlockId(parent),
            height,
            prevrandao: B256::ZERO,
            state_root: StateRoot(B256::repeat_byte(0xab)),
            txs: vec![],
        }
    }

    fn make_context(timestamp: u64, gas_limit: u64, base_fee: Option<u64>) -> BlockContext {
        let header =
            Header { timestamp, gas_limit, base_fee_per_gas: base_fee, ..Header::default() };
        BlockContext::new(header, B256::ZERO, B256::ZERO)
    }

    #[test]
    fn empty_block_is_indexed() {
        let index = BlockIndex::new();
        let block = make_block(7, B256::repeat_byte(0x01));
        let ctx = make_context(1_700_000_000, 30_000_000, Some(1_000_000_000));
        let outcome = ExecutionOutcome::new();

        index_finalized_block(&index, &block, &ctx, &outcome);

        let stats = index.stats();
        assert_eq!(stats.head_block_number, 7);
        assert_eq!(stats.block_count, 1);
        assert_eq!(stats.transaction_count, 0);
        assert_eq!(stats.receipt_count, 0);

        let indexed = index.get_block_by_number(7).expect("block at height 7");
        assert_eq!(indexed.number, 7);
        assert_eq!(indexed.parent_hash, B256::repeat_byte(0x01));
        assert_eq!(indexed.timestamp, 1_700_000_000);
        assert_eq!(indexed.gas_limit, 30_000_000);
        assert_eq!(indexed.gas_used, 0);
        assert_eq!(indexed.base_fee_per_gas, Some(1_000_000_000));
        assert!(indexed.transaction_hashes.is_empty());
    }

    #[test]
    fn malformed_tx_is_skipped_but_block_indexed() {
        use alloy_primitives::Bytes;
        use kora_domain::Tx;

        let index = BlockIndex::new();
        let mut block = make_block(3, B256::ZERO);
        // A clearly-malformed tx envelope — will fail decode_2718.
        block.txs = vec![Tx::new(Bytes::from_static(&[0xff, 0xff, 0xff]))];
        let ctx = make_context(0, 30_000_000, None);
        let outcome = ExecutionOutcome::new();

        index_finalized_block(&index, &block, &ctx, &outcome);

        let stats = index.stats();
        assert_eq!(stats.head_block_number, 3);
        assert_eq!(stats.block_count, 1);
        // Tx decode failed → no tx/receipt entries, but block is still in the index.
        assert_eq!(stats.transaction_count, 0);
        assert_eq!(stats.receipt_count, 0);
    }
}
