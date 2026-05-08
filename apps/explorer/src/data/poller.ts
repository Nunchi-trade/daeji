import { client } from './rpc';
import { bus } from './bus';
import type { ChainBlock, ChainTransaction, FeeDataPoint } from './types';

// ================================================================
// CONFIGURATION
// ================================================================

const BLOCK_POLL_INTERVAL = 4_000; // 4s -- avoid Railway rate limits
const FEE_POLL_INTERVAL = 15_000; // 15s -- fee history changes slowly

// ================================================================
// BLOCK POLLER
// ================================================================

export class BlockPoller {
  private lastBlockNumber: bigint = 0n;
  private blockTimer: ReturnType<typeof setInterval> | null = null;
  private feeTimer: ReturnType<typeof setInterval> | null = null;
  private running = false;

  // ================================================================
  // LIFECYCLE
  // ================================================================

  /** Start polling. Safe to call multiple times. */
  start(): void {
    if (this.running) return;
    this.running = true;

    // Immediate first poll, then interval
    this.pollBlock();
    this.pollFees();

    this.blockTimer = setInterval(() => this.pollBlock(), BLOCK_POLL_INTERVAL);
    this.feeTimer = setInterval(() => this.pollFees(), FEE_POLL_INTERVAL);
  }

  /** Stop all polling. */
  stop(): void {
    this.running = false;

    if (this.blockTimer !== null) {
      clearInterval(this.blockTimer);
      this.blockTimer = null;
    }
    if (this.feeTimer !== null) {
      clearInterval(this.feeTimer);
      this.feeTimer = null;
    }
  }

  /** Stop only block number polling (used when WS takes over). */
  disableBlockPoll(): void {
    if (this.blockTimer !== null) {
      clearInterval(this.blockTimer);
      this.blockTimer = null;
    }
  }

  // ================================================================
  // BLOCK POLLING
  // ================================================================

  private async pollBlock(): Promise<void> {
    try {
      const currentNumber = await client.getBlockNumber();

      // No new block: skip
      if (currentNumber <= this.lastBlockNumber) return;

      // Handle block gaps -- only fetch the latest few to avoid flooding
      const startBlock =
        this.lastBlockNumber === 0n
          ? currentNumber // first poll: just get latest
          : this.lastBlockNumber + 1n;

      // Cap catch-up to 3 blocks max to avoid rate limits
      const effectiveStart =
        currentNumber - startBlock > 3n ? currentNumber - 2n : startBlock;

      for (let n = effectiveStart; n <= currentNumber; n++) {
        await this.fetchAndEmitBlock(n);
      }

      this.lastBlockNumber = currentNumber;
    } catch (err) {
      // Log and continue. Never break the poll loop.
      console.warn('[BlockPoller] pollBlock error:', err);
    }
  }

  /**
   * Optimistic empty-block skip:
   * 1. Fetch block header (full=false) -- lightweight, no tx bodies
   * 2. If txCount is 0, emit block without fetching full bodies
   * 3. If txCount > 0, fetch again with full=true for transaction details
   */
  private async fetchAndEmitBlock(blockNumber: bigint): Promise<void> {
    // Step 1: header-only fetch
    const header = await client.getBlock({
      blockNumber,
      includeTransactions: false,
    });

    if (!header) {
      console.warn(`[BlockPoller] Block ${blockNumber} returned null`);
      return;
    }

    const arrivalTime = Date.now();
    const txCount = header.transactions.length;

    if (txCount === 0) {
      // Step 2a: empty block -- no need for a second fetch
      const block: ChainBlock = {
        number: header.number,
        hash: header.hash,
        parentHash: header.parentHash,
        timestamp: header.timestamp,
        gasUsed: header.gasUsed,
        gasLimit: header.gasLimit,
        baseFeePerGas: header.baseFeePerGas ?? 0n,
        transactions: [],
        stateRoot: header.stateRoot,
        transactionsRoot: header.transactionsRoot,
        receiptsRoot: header.receiptsRoot,
        _activityLevel: 0,
        _arrivalTime: arrivalTime,
      };

      bus.emit('block:new', block);
      return;
    }

    // Step 2b: block has transactions -- fetch full bodies
    const fullBlock = await client.getBlock({
      blockNumber,
      includeTransactions: true,
    });

    if (!fullBlock) return;

    const activityLevel =
      fullBlock.gasLimit > 0n
        ? Number((fullBlock.gasUsed * 10000n) / fullBlock.gasLimit) / 10000
        : 0;

    const block: ChainBlock = {
      number: fullBlock.number,
      hash: fullBlock.hash,
      parentHash: fullBlock.parentHash,
      timestamp: fullBlock.timestamp,
      gasUsed: fullBlock.gasUsed,
      gasLimit: fullBlock.gasLimit,
      baseFeePerGas: fullBlock.baseFeePerGas ?? 0n,
      transactions: fullBlock.transactions as unknown as ChainTransaction[],
      stateRoot: fullBlock.stateRoot,
      transactionsRoot: fullBlock.transactionsRoot,
      receiptsRoot: fullBlock.receiptsRoot,
      _activityLevel: activityLevel,
      _arrivalTime: arrivalTime,
    };

    bus.emit('block:new', block);

    // Emit individual transaction events + address:seen
    for (const tx of block.transactions) {
      bus.emit('tx:new', tx);
      bus.emit('address:seen', tx.from);
      if (tx.to) bus.emit('address:seen', tx.to);
    }
  }

  // ================================================================
  // FEE HISTORY POLLING
  // ================================================================

  private async pollFees(): Promise<void> {
    try {
      const history = await client.getFeeHistory({
        blockCount: 20,
        rewardPercentiles: [25, 50, 75],
      });

      if (!history || !history.baseFeePerGas) return;

      const points: FeeDataPoint[] = history.baseFeePerGas.map(
        (baseFee, i) => ({
          blockNumber: history.oldestBlock + BigInt(i),
          baseFee,
          gasUsedRatio: history.gasUsedRatio[i] ?? 0,
          reward25: history.reward?.[i]?.[0] ?? 0n,
          reward50: history.reward?.[i]?.[1] ?? 0n,
          reward75: history.reward?.[i]?.[2] ?? 0n,
        }),
      );

      bus.emit('fee:update', points);
    } catch (err) {
      // Log and continue. Fee data is secondary.
      console.warn('[BlockPoller] pollFees error:', err);
    }
  }
}
