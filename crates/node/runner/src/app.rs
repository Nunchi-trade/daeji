//! REVM-based consensus application implementation.

use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, UNIX_EPOCH},
};

use alloy_consensus::Header;
use alloy_primitives::{Address, B256, Bytes};
use commonware_consensus::{
    Application, Block as _, marshal::ancestry::Ancestry, simplex::types::Context,
};
use commonware_cryptography::{Committable as _, certificate::Scheme as CertScheme};
use commonware_runtime::{Clock, Metrics, Spawner};
use futures::StreamExt;
use kora_consensus::{BlockExecution, SnapshotStore, components::InMemorySnapshotStore};
use kora_domain::{Block, ConsensusDigest};
use kora_executor::{BaseFeeParams, BlockContext, BlockExecutor, calculate_base_fee};
use kora_ledger::LedgerService;
use kora_metrics::AppMetrics;
use kora_overlay::OverlayState;
use kora_qmdb_ledger::QmdbState;
use kora_rpc::NodeState;
use parking_lot::RwLock;
use rand::Rng;
use tracing::{debug, error, info, trace, warn};

/// Maximum time to wait for a parent snapshot to become available before
/// giving up and nullifying the view.  Uses event-driven notification
/// (via [`LedgerService::wait_for_snapshot`]) so the wake-up is immediate
/// once the snapshot is inserted, with this timeout as the upper bound.
///
/// Under CPU contention (e.g. 23 threads on 0.75 cores), the finalization
/// reporter may need more time to produce the parent snapshot.  100 ms
/// provides ample budget; in the common case the Notify fires within the
/// first few milliseconds.
const SNAPSHOT_WAIT_TIMEOUT: Duration = Duration::from_millis(100);

/// Maximum number of seconds a block timestamp may be ahead of the
/// validator's wall-clock time.  Blocks with timestamps further in the
/// future are rejected during verification.  15 seconds is generous enough
/// to tolerate clock skew between validators while preventing malicious
/// leaders from pushing timestamps arbitrarily far forward.
const MAX_FUTURE_TIMESTAMP_DRIFT: u64 = 15;

/// Maximum number of unfinalized blocks a leader may be ahead of the last
/// finalized height before it voluntarily skips its proposal turn.  This
/// prevents a single fast leader from racing too far ahead of finalization,
/// which can cascade into snapshot-miss failures for other validators.
///
/// The previous value of 8 was too tight under CPU contention and after node
/// restarts: transient finalization stalls (or the finalization pipeline
/// lagging during re-sync) would trip the guard and force every leader to
/// skip, producing a cascade of nullifications that could stall the entire
/// network.  A value of 64 gives finalization plenty of room to drain
/// without stalling proposals on healthy nodes.  At the current throughput
/// ceiling of ~30 blocks/s, a gap of 64 represents roughly 2 seconds of
/// blocks.
const MAX_PROPOSAL_LAG: u64 = 64;

/// Maximum number of entries retained in the `block_fees` cache.
///
/// Only the parent block's fee data is needed for base fee computation, so
/// entries older than `MAX_PROPOSAL_LAG * 2` are never queried.  Capping the
/// cache prevents unbounded memory growth (~132 MB/day at 33 blocks/s).
const MAX_BLOCK_FEES_CACHED: usize = 128;

fn unix_timestamp_secs<Env: Clock>(env: &Env) -> u64 {
    env.current().duration_since(UNIX_EPOCH).map(|duration| duration.as_secs()).unwrap_or(0)
}

/// Number of blocks the network must advance PAST the recovered height
/// (as measured by full-execution verification, not certificate trust)
/// before the node exits catch-up mode and starts requiring full
/// re-execution for verification.
///
/// During catch-up, blocks whose parent snapshot is missing are trusted
/// based on their finality certificate (the resolver already verified the
/// certificate before delivering the block to the application layer).
///
/// Previously this was set to 2, which meant the node exited catch-up mode
/// almost immediately -- each trusted block advanced `recovered_height`,
/// so the *next* block was only 1 ahead, below the threshold of 2.  The
/// catch-up window collapsed after a single block.
///
/// Now the catch-up window is anchored to the *original* `recovered_height`
/// and only closes when `last_verified_height` (advanced only by full
/// execution, NOT by certificate trust) reaches `recovered_height + 64`.
const CATCH_UP_THRESHOLD: u64 = 64;

/// REVM-based consensus application.
#[derive(Clone)]
pub struct RevmApplication<S, E> {
    ledger: LedgerService,
    executor: E,
    max_txs: usize,
    gas_limit: u64,
    fee_recipient: Address,
    node_state: Option<NodeState>,
    metrics: Option<AppMetrics>,
    /// Height of the HEAD block that was restored from the archive during
    /// startup recovery.  This value is set once at startup and never
    /// changes; it anchors the catch-up window.
    recovered_height: Arc<AtomicU64>,
    /// The highest block height that has been processed by `verify_block`.
    last_verified_height: Arc<AtomicU64>,
    /// Per-block `(gas_used, base_fee_per_gas)` cache, keyed by consensus
    /// digest.  Bounded to [`MAX_BLOCK_FEES_CACHED`] entries; oldest
    /// entries are evicted on insertion to prevent unbounded memory growth.
    block_fees: Arc<RwLock<HashMap<ConsensusDigest, (u64, u64)>>>,
    /// Insertion-ordered queue of block fee cache keys for oldest-first eviction.
    block_fees_order: Arc<RwLock<VecDeque<ConsensusDigest>>>,
    _scheme: std::marker::PhantomData<S>,
}

impl<S, E> std::fmt::Debug for RevmApplication<S, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RevmApplication")
            .field("max_txs", &self.max_txs)
            .field("gas_limit", &self.gas_limit)
            .field("fee_recipient", &self.fee_recipient)
            .field("metrics", &self.metrics.is_some())
            .field("recovered_height", &self.recovered_height.load(Ordering::Relaxed))
            .field("last_verified_height", &self.last_verified_height.load(Ordering::Relaxed))
            .field("block_fees_cached", &self.block_fees.read().len())
            .field("block_fees_capacity", &MAX_BLOCK_FEES_CACHED)
            .finish_non_exhaustive()
    }
}

impl<S, E> RevmApplication<S, E>
where
    E: BlockExecutor<OverlayState<QmdbState>, Tx = Bytes> + Clone,
{
    /// Create a new REVM application.
    pub fn new(
        ledger: LedgerService,
        executor: E,
        max_txs: usize,
        gas_limit: u64,
        fee_recipient: Address,
    ) -> Self {
        let genesis_digest = ledger.genesis_block().commitment();
        let mut block_fees = HashMap::new();
        block_fees.insert(genesis_digest, (0, kora_config::INITIAL_BASE_FEE));
        let mut block_fees_order = VecDeque::new();
        block_fees_order.push_back(genesis_digest);

        Self {
            ledger,
            executor,
            max_txs,
            gas_limit,
            fee_recipient,
            node_state: None,
            metrics: None,
            recovered_height: Arc::new(AtomicU64::new(0)),
            last_verified_height: Arc::new(AtomicU64::new(0)),
            block_fees: Arc::new(RwLock::new(block_fees)),
            block_fees_order: Arc::new(RwLock::new(block_fees_order)),
            _scheme: std::marker::PhantomData,
        }
    }

    /// Set the node state for tracking proposal metrics.
    #[must_use]
    pub fn with_node_state(mut self, state: NodeState) -> Self {
        self.node_state = Some(state);
        self
    }

    /// Attach application-level metrics.
    #[must_use]
    pub fn with_metrics(mut self, metrics: AppMetrics) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Set the height of the HEAD block that was recovered from the archive.
    #[must_use]
    pub fn with_recovered_height(self, height: u64) -> Self {
        self.recovered_height.store(height, Ordering::Relaxed);
        self.last_verified_height.store(height, Ordering::Relaxed);
        self
    }

    /// Seed the block-fee cache with entries from the block index so that
    /// the first blocks after a restart can derive a correct EIP-1559 base
    /// fee.
    pub fn seed_block_fees(&self, entries: &[(ConsensusDigest, u64, u64)]) {
        let mut fees = self.block_fees.write();
        let mut order = self.block_fees_order.write();
        for &(digest, gas_used, base_fee) in entries {
            if fees.insert(digest, (gas_used, base_fee)).is_none() {
                order.push_back(digest);
            }
        }
    }

    /// Compute the base fee for a new block from the parent's gas usage
    /// (EIP-1559).
    fn compute_base_fee(&self, parent_digest: ConsensusDigest) -> u64 {
        let fees = self.block_fees.read();
        match fees.get(&parent_digest) {
            Some(&(parent_gas_used, parent_base_fee)) => calculate_base_fee(
                parent_base_fee,
                parent_gas_used,
                self.gas_limit,
                &BaseFeeParams::DEFAULT,
            ),
            None => kora_config::INITIAL_BASE_FEE,
        }
    }

    /// Record a block's gas usage and base fee so that the next block can
    /// derive its own base fee via [`Self::compute_base_fee`].
    ///
    /// Evicts the oldest entries when the cache exceeds
    /// [`MAX_BLOCK_FEES_CACHED`] to prevent unbounded memory growth.
    fn record_block_fees(&self, digest: ConsensusDigest, gas_used: u64, base_fee: u64) {
        let mut fees = self.block_fees.write();
        let mut order = self.block_fees_order.write();

        if fees.insert(digest, (gas_used, base_fee)).is_none() {
            order.push_back(digest);
        }

        // Evict oldest entries beyond the capacity limit.
        while order.len() > MAX_BLOCK_FEES_CACHED {
            if let Some(oldest) = order.pop_front() {
                fees.remove(&oldest);
            }
        }
    }

    fn block_context(
        &self,
        height: u64,
        timestamp: u64,
        prevrandao: B256,
        parent_digest: ConsensusDigest,
    ) -> BlockContext {
        let base_fee = self.compute_base_fee(parent_digest);
        let header = Header {
            number: height,
            timestamp,
            gas_limit: self.gas_limit,
            beneficiary: self.fee_recipient,
            base_fee_per_gas: Some(base_fee),
            ..Default::default()
        };
        BlockContext::new(header, B256::ZERO, prevrandao)
    }

    async fn get_prevrandao(&self, parent_digest: ConsensusDigest) -> B256 {
        self.ledger.seed_for_parent(parent_digest).await.unwrap_or(B256::ZERO)
    }

    async fn build_block(&self, parent: &Block, timestamp: u64) -> Option<Block> {
        use kora_consensus::Mempool as _;

        let start = Instant::now();
        let parent_digest = parent.commitment();

        let parent_snapshot = {
            let wait_start = Instant::now();
            match self.ledger.wait_for_snapshot(parent_digest, SNAPSHOT_WAIT_TIMEOUT).await {
                Some(s) => {
                    let wait_elapsed = wait_start.elapsed();
                    if wait_elapsed.as_millis() > 1 {
                        if let Some(ref m) = self.metrics {
                            m.snapshot_poll_wait.observe(wait_elapsed.as_secs_f64());
                        }
                        debug!(
                            parent_height = parent.height,
                            ?parent_digest,
                            wait_ms = wait_elapsed.as_millis(),
                            "build_block: parent snapshot arrived after waiting"
                        );
                    }
                    s
                }
                None => {
                    if let Some(ref m) = self.metrics {
                        m.proposal_snapshot_misses.inc();
                    }
                    warn!(
                        parent_height = parent.height,
                        ?parent_digest,
                        wait_ms = wait_start.elapsed().as_millis(),
                        "build_block: parent snapshot not found after waiting \
                         -- node has not yet processed this parent block"
                    );
                    return None;
                }
            }
        };
        let snapshot_elapsed = start.elapsed();

        let (_, mempool, snapshots) = self.ledger.proposal_components().await;
        let excluded = match self.collect_pending_tx_ids(&snapshots, parent_digest) {
            Some(ids) => ids,
            None => {
                return None;
            }
        };
        let mempool_len = mempool.len();
        let excluded_len = excluded.len();
        let txs = mempool.build(self.max_txs, &excluded);

        if txs.is_empty() && mempool_len > excluded_len {
            warn!(
                mempool_len,
                excluded_len,
                max_txs = self.max_txs,
                "build_block: mempool has unincluded txs but produced empty block"
            );
        } else {
            trace!(
                mempool_len,
                excluded_len,
                drained = txs.len(),
                max_txs = self.max_txs,
                "build_block: mempool drain"
            );
        }

        let prevrandao = self.get_prevrandao(parent_digest).await;
        let height = parent.height + 1;
        let context = self.block_context(height, timestamp, prevrandao, parent_digest);
        let base_fee = context.header.base_fee_per_gas.unwrap_or(kora_config::INITIAL_BASE_FEE);
        let txs_bytes: Vec<Bytes> = txs.iter().map(|tx| tx.bytes.clone()).collect();

        let exec_start = Instant::now();
        let outcome = {
            let executor = self.executor.clone();
            let state = parent_snapshot.state.clone();
            match tokio::task::spawn_blocking(move || {
                executor.execute(&state, &context, &txs_bytes)
            })
            .await
            {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(err)) => {
                    error!(
                        parent = ?parent_digest,
                        height,
                        txs = txs.len(),
                        gas_limit = self.gas_limit,
                        error = %err,
                        error_debug = ?err,
                        "build_block: block execution failed -- \
                         this may indicate a bad transaction, OOM, or state corruption"
                    );
                    return None;
                }
                Err(join_err) => {
                    error!(
                        parent = ?parent_digest,
                        height,
                        error = %join_err,
                        "build_block: spawn_blocking join error"
                    );
                    return None;
                }
            }
        };
        let exec_elapsed = exec_start.elapsed();

        let root_start = Instant::now();
        let state_root =
            match self.ledger.compute_root_from_store(parent_digest, &outcome.changes).await {
                Ok(root) => root,
                Err(err) => {
                    error!(
                        parent = ?parent_digest,
                        height,
                        error = %err,
                        error_debug = ?err,
                        "build_block: QMDB state root computation failed -- \
                         this may indicate a storage I/O error or inconsistent state"
                    );
                    return None;
                }
            };
        let root_elapsed = root_start.elapsed();

        let block = Block::new(parent.id(), height, timestamp, prevrandao, state_root, txs);
        let block_digest = block.commitment();

        self.record_block_fees(block_digest, outcome.gas_used, base_fee);

        let total_elapsed = start.elapsed();

        if let Some(ref m) = self.metrics {
            m.block_build_time.observe(total_elapsed.as_secs_f64());
            m.evm_execution_seconds.observe(exec_elapsed.as_secs_f64());
            m.block_txs_included.set(block.txs.len() as i64);
        }

        debug!(
            ?block_digest,
            height,
            timestamp,
            txs = block.txs.len(),
            snapshot_ms = snapshot_elapsed.as_millis(),
            exec_ms = exec_elapsed.as_millis(),
            root_ms = root_elapsed.as_millis(),
            total_ms = total_elapsed.as_millis(),
            "built block"
        );
        Some(block)
    }

    /// Check whether the node is in catch-up mode.
    fn is_catching_up(&self, block_height: u64) -> bool {
        let recovered = self.recovered_height.load(Ordering::Relaxed);
        if recovered == 0 {
            return false;
        }
        if block_height <= recovered {
            return false;
        }
        let verified = self.last_verified_height.load(Ordering::Relaxed);
        verified < recovered.saturating_add(CATCH_UP_THRESHOLD)
    }

    async fn verify_block(
        &self,
        block: &Block,
        parent_timestamp: Option<u64>,
        now_secs: u64,
    ) -> bool {
        let start = Instant::now();
        let digest = block.commitment();
        let parent_digest = block.parent();

        if self.ledger.query_state_root(digest).await.is_some() {
            self.last_verified_height.fetch_max(block.height, Ordering::Relaxed);
            if let Some(ref state) = self.node_state {
                state.set_last_verified_height(block.height);
            }
            trace!(?digest, height = block.height, "block already verified");
            return true;
        }

        if !self.is_catching_up(block.height) {
            if let Some(parent_ts) = parent_timestamp
                && block.timestamp < parent_ts
            {
                warn!(
                    ?digest,
                    height = block.height,
                    block_timestamp = block.timestamp,
                    parent_timestamp = parent_ts,
                    "verify_block: timestamp moved backwards"
                );
                return false;
            }

            let max_allowed = now_secs.saturating_add(MAX_FUTURE_TIMESTAMP_DRIFT);
            if block.timestamp > max_allowed {
                warn!(
                    ?digest,
                    height = block.height,
                    block_timestamp = block.timestamp,
                    now_secs,
                    max_allowed,
                    "verify_block: timestamp too far in the future"
                );
                return false;
            }
        }

        let parent_snapshot = match self.ledger.parent_snapshot(parent_digest).await {
            Some(snap) => snap,
            None => {
                if self.is_catching_up(block.height) {
                    debug!(
                        ?digest,
                        ?parent_digest,
                        height = block.height,
                        recovered_height = self.recovered_height.load(Ordering::Relaxed),
                        last_verified = self.last_verified_height.load(Ordering::Relaxed),
                        "verify_block: parent snapshot missing during catch-up; \
                         trusting finality certificate"
                    );
                    self.ledger.restore_persisted_snapshot(block).await;
                    return true;
                }

                warn!(
                    ?digest,
                    ?parent_digest,
                    height = block.height,
                    recovered_height = self.recovered_height.load(Ordering::Relaxed),
                    last_verified = self.last_verified_height.load(Ordering::Relaxed),
                    "verify_block: missing parent snapshot (not in catch-up mode)"
                );
                return false;
            }
        };
        let snapshot_elapsed = start.elapsed();

        let context =
            self.block_context(block.height, block.timestamp, block.prevrandao, parent_digest);
        let base_fee = context.header.base_fee_per_gas.unwrap_or(kora_config::INITIAL_BASE_FEE);
        let exec_start = Instant::now();
        let execution =
            match BlockExecution::execute(&parent_snapshot, &self.executor, &context, &block.txs)
                .await
            {
                Ok(result) => result,
                Err(err) => {
                    if self.is_catching_up(block.height) {
                        warn!(
                            ?digest,
                            height = block.height,
                            error = ?err,
                            "verify_block: execution failed during catch-up; \
                             falling back to certificate trust"
                        );
                        self.ledger.restore_persisted_snapshot(block).await;
                        return true;
                    }
                    warn!(?digest, error = ?err, "execution failed");
                    return false;
                }
            };
        let exec_elapsed = exec_start.elapsed();

        let root_start = Instant::now();
        let state_root = match self
            .ledger
            .compute_root_from_store(parent_digest, &execution.outcome.changes)
            .await
        {
            Ok(root) => root,
            Err(err) => {
                if self.is_catching_up(block.height) {
                    warn!(
                        ?digest,
                        height = block.height,
                        error = ?err,
                        "verify_block: compute root failed during catch-up; \
                         falling back to certificate trust"
                    );
                    self.ledger.restore_persisted_snapshot(block).await;
                    return true;
                }
                warn!(?digest, error = ?err, "compute root failed");
                return false;
            }
        };
        let root_elapsed = root_start.elapsed();

        if state_root != block.state_root {
            if self.is_catching_up(block.height) {
                warn!(
                    ?digest,
                    height = block.height,
                    expected = ?block.state_root,
                    computed = ?state_root,
                    "verify_block: state root mismatch during catch-up; \
                     falling back to certificate trust \
                     (parent snapshot likely has empty changeset from prior trust)"
                );
                self.ledger.restore_persisted_snapshot(block).await;
                return true;
            }
            warn!(
                ?digest,
                expected = ?block.state_root,
                computed = ?state_root,
                "state root mismatch"
            );
            return false;
        }

        self.record_block_fees(digest, execution.outcome.gas_used, base_fee);

        let merged_changes = parent_snapshot.state.merge_changes(execution.outcome.changes.clone());
        let next_state = OverlayState::new(parent_snapshot.state.base(), merged_changes);

        self.ledger
            .insert_snapshot(
                digest,
                parent_digest,
                next_state,
                state_root,
                execution.outcome.changes,
                &block.txs,
            )
            .await;

        let prev_verified = self.last_verified_height.fetch_max(block.height, Ordering::Relaxed);
        if let Some(ref state) = self.node_state {
            state.set_last_verified_height(block.height);
        }
        if prev_verified < self.recovered_height.load(Ordering::Relaxed)
            && block.height >= self.recovered_height.load(Ordering::Relaxed)
        {
            info!(
                height = block.height,
                recovered_height = self.recovered_height.load(Ordering::Relaxed),
                "catch-up: first full-execution verification past recovery point"
            );
        }

        if let Some(ref m) = self.metrics {
            m.evm_execution_seconds.observe(exec_elapsed.as_secs_f64());
        }

        let total_elapsed = start.elapsed();
        debug!(
            ?digest,
            height = block.height,
            txs = block.txs.len(),
            snapshot_ms = snapshot_elapsed.as_millis(),
            exec_ms = exec_elapsed.as_millis(),
            root_ms = root_elapsed.as_millis(),
            total_ms = total_elapsed.as_millis(),
            "verified block"
        );
        true
    }

    /// Collect transaction IDs from unpersisted ancestor snapshots.
    fn collect_pending_tx_ids(
        &self,
        snapshots: &InMemorySnapshotStore<OverlayState<QmdbState>>,
        from: ConsensusDigest,
    ) -> Option<BTreeSet<kora_consensus::TxId>> {
        let mut excluded = BTreeSet::new();
        let mut current = Some(from);

        while let Some(digest) = current {
            if snapshots.is_persisted(&digest) {
                break;
            }
            let Some(snapshot) = snapshots.get(&digest) else {
                warn!(
                    ?digest,
                    collected_so_far = excluded.len(),
                    "snapshot chain gap during tx exclusion collection -- \
                     refusing to build block to prevent duplicate transactions"
                );
                return None;
            };
            excluded.extend(snapshot.tx_ids.iter().copied());
            current = snapshot.parent;
        }

        Some(excluded)
    }
}

impl<Env, S, E> Application<Env> for RevmApplication<S, E>
where
    Env: Rng + Spawner + Metrics + Clock,
    S: CertScheme + Send + Sync + 'static,
    E: BlockExecutor<OverlayState<QmdbState>, Tx = Bytes> + Clone + Send + Sync + 'static,
{
    type SigningScheme = S;
    type Context = Context<ConsensusDigest, S::PublicKey>;
    type Block = Block;

    fn propose(
        &mut self,
        context: (Env, Self::Context),
        mut ancestry: impl Ancestry<Self::Block>,
    ) -> impl std::future::Future<Output = Option<Self::Block>> + Send {
        let node_state = self.node_state.clone();
        let metrics = self.metrics.clone();
        let env = context.0;
        async move {
            let start = Instant::now();
            let parent = ancestry.next().await?;
            let ancestry_elapsed = start.elapsed();

            if let Some(ref state) = node_state {
                let finalized = state.finalized_height();
                if parent.height > finalized + MAX_PROPOSAL_LAG {
                    if let Some(ref m) = metrics {
                        m.proposal_lag_skips.inc();
                    }
                    warn!(
                        parent_height = parent.height,
                        finalized_height = finalized,
                        max_lag = MAX_PROPOSAL_LAG,
                        "skipping proposal: parent too far ahead of finalized height"
                    );
                    return None;
                }
            }

            let now_secs = unix_timestamp_secs(&env);
            let timestamp = match Block::next_timestamp(now_secs, parent.timestamp) {
                Some(ts) => ts,
                None => {
                    tracing::error!(
                        parent_timestamp = parent.timestamp,
                        "timestamp overflow: cannot produce a timestamp after parent"
                    );
                    return None;
                }
            };

            let build_start = Instant::now();
            let block = self.build_block(&parent, timestamp).await;
            let build_elapsed = build_start.elapsed();

            match block {
                Some(ref b) => {
                    if let Some(ref state) = node_state {
                        state.inc_proposed();
                    }
                    debug!(
                        height = b.height,
                        timestamp = b.timestamp,
                        ancestry_ms = ancestry_elapsed.as_millis(),
                        build_ms = build_elapsed.as_millis(),
                        total_ms = start.elapsed().as_millis(),
                        "propose complete"
                    );
                }
                None => {
                    warn!(
                        parent_height = parent.height,
                        parent_digest = ?parent.commitment(),
                        build_ms = build_elapsed.as_millis(),
                        "propose failed: build_block returned None \
                         (likely missing parent snapshot -- node may still be catching up)"
                    );
                }
            }

            block
        }
    }

    fn verify(
        &mut self,
        context: (Env, Self::Context),
        mut ancestry: impl Ancestry<Self::Block>,
    ) -> impl std::future::Future<Output = bool> + Send {
        let env = context.0;
        async move {
            let start = Instant::now();
            let now_secs = unix_timestamp_secs(&env);

            let mut blocks_to_verify = Vec::new();
            let mut verified_parent_timestamp: Option<u64> = None;
            while let Some(block) = ancestry.next().await {
                let digest = block.commitment();
                if self.ledger.query_state_root(digest).await.is_some() {
                    verified_parent_timestamp = Some(block.timestamp);
                    break;
                }
                blocks_to_verify.push(block);
            }
            let ancestry_elapsed = start.elapsed();

            if blocks_to_verify.is_empty() {
                trace!(ancestry_ms = ancestry_elapsed.as_millis(), "all blocks already verified");
                return true;
            }

            let block_count = blocks_to_verify.len();
            let tip_height = blocks_to_verify.first().map(|b| b.height).unwrap_or(0);

            let verify_start = Instant::now();
            let mut parent_ts = verified_parent_timestamp;
            for block in blocks_to_verify.into_iter().rev() {
                if !self.verify_block(&block, parent_ts, now_secs).await {
                    return false;
                }
                parent_ts = Some(block.timestamp);
            }
            let verify_elapsed = verify_start.elapsed();
            let total_elapsed = start.elapsed();

            debug!(
                tip_height,
                block_count,
                ancestry_ms = ancestry_elapsed.as_millis(),
                verify_ms = verify_elapsed.as_millis(),
                total_ms = total_elapsed.as_millis(),
                "verify complete"
            );

            true
        }
    }
}
