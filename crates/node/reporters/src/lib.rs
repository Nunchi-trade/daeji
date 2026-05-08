//! Consensus reporters for Kora nodes.
#![doc = include_str!("../README.md")]
#![doc(issue_tracker_base_url = "https://github.com/refcell/kora/issues/")]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use std::{fmt, marker::PhantomData, sync::Arc};

use kora_hdc_chain::OnChainHdcIndex;

type HdcIndex = Arc<parking_lot::RwLock<OnChainHdcIndex>>;

use alloy_consensus::{Transaction as _, TxEnvelope, transaction::SignerRecoverable as _};
use alloy_eips::eip2718::Decodable2718 as _;
use alloy_primitives::{B256, Bytes, U256, U64, keccak256};
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
use kora_rpc::{ConsensusEvent, NodeState, SubscriptionEvent};
use tokio_crate::sync::broadcast;
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
    hdc_index: Option<HdcIndex>,
    block_broadcast: Option<broadcast::Sender<SubscriptionEvent>>,
    update: Update<Block>,
) where
    E: BlockExecutor<OverlayState<QmdbState>, Tx = Bytes>,
    P: BlockContextProvider,
{
    match update {
        Update::Tip(..) => {}
        Update::Block(block, ack) => {
            let digest = block.commitment();
            let snapshot_exists = state.query_state_root(digest).await.is_some();
            let mut execution_outcome = None;
            let mut execution_context = None;

            if !snapshot_exists || block_index.is_some() {
                if snapshot_exists {
                    trace!(?digest, "re-executing finalized block for RPC indexing");
                } else {
                    trace!(?digest, "missing snapshot for finalized block; re-executing");
                }
                let parent_digest = block.parent();
                if let Some(parent_snapshot) = state.parent_snapshot(parent_digest).await {
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

                    if !snapshot_exists {
                        let merged_changes =
                            parent_snapshot.state.merge_changes(execution.outcome.changes.clone());
                        let next_state =
                            OverlayState::new(parent_snapshot.state.base(), merged_changes);
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

                    execution_outcome = Some(execution.outcome);
                    execution_context = Some(block_context);
                } else if snapshot_exists {
                    warn!(
                        ?digest,
                        ?parent_digest,
                        "missing parent snapshot for cached finalized block; skipping RPC indexing replay"
                    );
                } else {
                    error!(?digest, ?parent_digest, "missing parent snapshot for finalized block");
                    ack.acknowledge();
                    return;
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
            if let (Some(index), Some(outcome), Some(block_context)) =
                (block_index.as_ref(), execution_outcome.as_ref(), execution_context.as_ref())
            {
                index_finalized_block(index, &block, block_context, outcome);
            }

            // Broadcast block events to WebSocket subscribers.
            if let (Some(tx), Some(outcome), Some(block_context)) =
                (&block_broadcast, execution_outcome.as_ref(), execution_context.as_ref())
            {
                broadcast_block_events(tx, &block, block_context, outcome);
            }

            // Process HDC events from finalized receipt logs in canonical order.
            if let (Some(hdc_idx), Some(outcome)) = (hdc_index.as_ref(), execution_outcome.as_ref())
            {
                process_hdc_events(hdc_idx, outcome);
            }
            state.prune_mempool(&block.txs).await;
            // Marshal waits for the application to acknowledge processing before advancing the
            // delivery floor. Without this, the node can stall on finalized block delivery.
            ack.acknowledge();
        }
    }
}

#[derive(Clone, Debug)]
struct TxMetadata {
    from: alloy_primitives::Address,
    to: Option<alloy_primitives::Address>,
    value: alloy_primitives::U256,
    gas_limit: u64,
    gas_price: u128,
    input: Bytes,
    nonce: u64,
}

fn index_finalized_block(
    index: &BlockIndex,
    block: &Block,
    block_context: &BlockContext,
    outcome: &ExecutionOutcome,
) {
    let block_hash = block.id().0;
    let transaction_hashes = block.txs.iter().map(|tx| keccak256(&tx.bytes)).collect::<Vec<_>>();
    let tx_metadata = block.txs.iter().map(|tx| decode_tx_metadata(&tx.bytes)).collect::<Vec<_>>();

    let indexed_block = IndexedBlock {
        hash: block_hash,
        number: block.height,
        parent_hash: block.parent.0,
        state_root: block.state_root.0,
        timestamp: block_context.header.timestamp,
        gas_limit: block_context.header.gas_limit,
        gas_used: outcome.gas_used,
        base_fee_per_gas: block_context.header.base_fee_per_gas,
        transaction_hashes,
    };

    let indexed_txs = tx_metadata
        .iter()
        .enumerate()
        .filter_map(|(idx, metadata)| {
            let metadata = metadata.as_ref()?;
            let hash = keccak256(&block.txs[idx].bytes);
            Some(IndexedTransaction {
                hash,
                block_hash,
                block_number: block.height,
                index: idx as u64,
                from: metadata.from,
                to: metadata.to,
                value: metadata.value,
                gas_limit: metadata.gas_limit,
                gas_price: metadata.gas_price,
                input: metadata.input.clone(),
                nonce: metadata.nonce,
            })
        })
        .collect();

    let mut next_log_index = 0u64;
    let indexed_receipts = outcome
        .receipts
        .iter()
        .enumerate()
        .filter_map(|(idx, receipt)| {
            let metadata = tx_metadata.get(idx)?.as_ref()?;
            let logs = receipt
                .logs()
                .iter()
                .map(|log| {
                    let (topics, data) = log.data.clone().split();
                    let log_index = next_log_index;
                    next_log_index += 1;
                    IndexedLog { address: log.address, topics, data, log_index }
                })
                .collect();

            Some(IndexedReceipt {
                transaction_hash: receipt.tx_hash,
                block_hash,
                block_number: block.height,
                transaction_index: idx as u64,
                from: metadata.from,
                to: metadata.to,
                cumulative_gas_used: receipt.cumulative_gas_used(),
                gas_used: receipt.gas_used,
                contract_address: receipt.contract_address,
                logs,
                status: receipt.success(),
            })
        })
        .collect();

    index.insert_block(indexed_block, indexed_txs, indexed_receipts);
}

/// Broadcast finalized block events to WebSocket subscribers.
///
/// Constructs an [`RpcBlock`] and [`RpcLog`] entries from the execution outcome
/// and sends them over the broadcast channel.  Receivers that are too slow will
/// miss messages (lagged), but sending never blocks or errors fatally.
fn broadcast_block_events(
    tx: &broadcast::Sender<SubscriptionEvent>,
    block: &Block,
    block_context: &BlockContext,
    outcome: &ExecutionOutcome,
) {
    use kora_rpc::{BlockTransactions, RpcBlock, RpcLog};

    let block_hash = block.id().0;
    let transaction_hashes: Vec<B256> =
        block.txs.iter().map(|t| keccak256(&t.bytes)).collect();

    let rpc_block = RpcBlock {
        hash: block_hash,
        parent_hash: block.parent.0,
        number: U64::from(block.height),
        state_root: block.state_root.0,
        timestamp: U64::from(block_context.header.timestamp),
        gas_limit: U64::from(block_context.header.gas_limit),
        gas_used: U64::from(outcome.gas_used),
        base_fee_per_gas: block_context.header.base_fee_per_gas.map(U256::from),
        transactions: BlockTransactions::Hashes(transaction_hashes),
        ..Default::default()
    };

    // Best-effort: ignore send errors (no subscribers).
    let _ = tx.send(SubscriptionEvent::NewHead(rpc_block));

    // Broadcast individual log events.
    let mut next_log_index = 0u64;
    for (tx_idx, receipt) in outcome.receipts.iter().enumerate() {
        let tx_hash = block
            .txs
            .get(tx_idx)
            .map(|t| keccak256(&t.bytes))
            .unwrap_or_default();
        for log in receipt.logs() {
            let (topics, data) = log.data.clone().split();
            let rpc_log = RpcLog {
                address: log.address,
                topics,
                data,
                block_number: U64::from(block.height),
                transaction_hash: tx_hash,
                transaction_index: U64::from(tx_idx as u64),
                block_hash,
                log_index: U64::from(next_log_index),
                removed: false,
            };
            let _ = tx.send(SubscriptionEvent::Log(rpc_log));
            next_log_index += 1;
        }
    }
}

/// Process HDC events from a finalized block's receipt logs.
///
/// Iterates receipts and their logs in canonical order, calling
/// [`kora_hdc_chain::event::process_log`] for each log entry. Errors from
/// individual logs are logged and skipped -- they must not block finalization.
fn process_hdc_events(hdc_index: &HdcIndex, outcome: &ExecutionOutcome) {
    let mut index = hdc_index.write();
    for receipt in &outcome.receipts {
        for log in receipt.logs() {
            let (topics, data) = log.data.clone().split();
            if let Err(e) =
                kora_hdc_chain::event::process_log(&mut index, &topics, &data, &log.address)
            {
                // UnknownTopic is expected for non-HDC logs; only warn on real errors.
                match e {
                    kora_hdc_chain::event::EventError::UnknownTopic(_) => {}
                    _ => {
                        warn!(
                            error = %e,
                            address = %log.address,
                            "HDC event processing error"
                        );
                    }
                }
            }
        }
    }
}

fn decode_tx_metadata(tx_bytes: &Bytes) -> Option<TxMetadata> {
    let envelope = match TxEnvelope::decode_2718(&mut tx_bytes.as_ref()) {
        Ok(envelope) => envelope,
        Err(err) => {
            warn!(error = %err, "failed to decode finalized transaction for indexing");
            return None;
        }
    };
    let from = match envelope.recover_signer() {
        Ok(from) => from,
        Err(err) => {
            warn!(error = %err, "failed to recover finalized transaction sender for indexing");
            return None;
        }
    };

    Some(TxMetadata {
        from,
        to: envelope.to(),
        value: envelope.value(),
        gas_limit: envelope.gas_limit(),
        gas_price: effective_gas_price(&envelope),
        input: envelope.input().clone(),
        nonce: envelope.nonce(),
    })
}

const fn effective_gas_price(envelope: &TxEnvelope) -> u128 {
    match envelope {
        TxEnvelope::Legacy(tx) => tx.tx().gas_price,
        TxEnvelope::Eip2930(tx) => tx.tx().gas_price,
        TxEnvelope::Eip1559(tx) => tx.tx().max_fee_per_gas,
        TxEnvelope::Eip4844(tx) => tx.tx().tx().max_fee_per_gas,
        TxEnvelope::Eip7702(tx) => tx.tx().max_fee_per_gas,
    }
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
    /// Optional RPC block index updated after finalized blocks are persisted.
    block_index: Option<Arc<BlockIndex>>,
    /// Optional HDC on-chain index updated from finalized receipt logs.
    hdc_index: Option<HdcIndex>,
    /// Broadcast channel for WebSocket subscription events (newHeads, logs).
    block_broadcast: Option<broadcast::Sender<SubscriptionEvent>>,
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
    /// Create a new finalized reporter.
    pub const fn new(
        state: LedgerService,
        context: tokio::Context,
        executor: E,
        provider: P,
    ) -> Self {
        Self {
            state,
            context,
            executor,
            provider,
            block_index: None,
            hdc_index: None,
            block_broadcast: None,
        }
    }

    /// Attach the RPC-visible block index to update when blocks finalize.
    #[must_use]
    pub fn with_block_index(mut self, block_index: Arc<BlockIndex>) -> Self {
        self.block_index = Some(block_index);
        self
    }

    /// Attach the HDC on-chain index to update from finalized receipt logs.
    #[must_use]
    pub fn with_hdc_index(mut self, hdc_index: HdcIndex) -> Self {
        self.hdc_index = Some(hdc_index);
        self
    }

    /// Attach a broadcast sender for WebSocket subscription events.
    #[must_use]
    pub fn with_block_broadcast(
        mut self,
        tx: broadcast::Sender<SubscriptionEvent>,
    ) -> Self {
        self.block_broadcast = Some(tx);
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
        let hdc_index = self.hdc_index.clone();
        let block_broadcast = self.block_broadcast.clone();
        async move {
            handle_finalized_update(
                state,
                context,
                executor,
                provider,
                block_index,
                hdc_index,
                block_broadcast,
                update,
            )
            .await;
        }
    }
}

/// Reporter that updates RPC-visible node state from consensus activity.
///
/// This reporter tracks:
/// - Current view number (from notarizations)
/// - Finalized block count
/// - Nullified round count
///
/// When a consensus broadcast sender is attached, it also emits
/// [`ConsensusEvent`] values for WebSocket subscribers.
#[derive(Clone)]
pub struct NodeStateReporter<S> {
    /// RPC node state to update.
    state: NodeState,
    /// Optional broadcast for consensus events to WS subscribers.
    consensus_broadcast: Option<broadcast::Sender<ConsensusEvent>>,
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
        Self { state, consensus_broadcast: None, _scheme: PhantomData }
    }

    /// Attach a broadcast sender for consensus events.
    #[must_use]
    pub fn with_consensus_broadcast(
        mut self,
        tx: broadcast::Sender<ConsensusEvent>,
    ) -> Self {
        self.consensus_broadcast = Some(tx);
        self
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
                let view = n.proposal.round.view().get();
                self.state.set_view(view);
                if let Some(tx) = &self.consensus_broadcast {
                    let _ = tx.send(ConsensusEvent::Notarization { view });
                }
            }
            Activity::Finalization(f) => {
                let view = f.proposal.round.view().get();
                self.state.set_view(view);
                self.state.inc_finalized();
                if let Some(tx) = &self.consensus_broadcast {
                    let _ = tx.send(ConsensusEvent::Finalization { view });
                }
            }
            Activity::Nullification(n) => {
                self.state.inc_nullified();
                if let Some(tx) = &self.consensus_broadcast {
                    let view = n.round.view().get();
                    let _ = tx.send(ConsensusEvent::Nullification { view });
                }
            }
            _ => {}
        }
        async {}
    }
}
