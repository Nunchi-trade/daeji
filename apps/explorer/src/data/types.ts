// ================================================================
// HEX BRANDED TYPE
// ================================================================

/** Hex-encoded string, always prefixed with 0x */
export type Hex = `0x${string}`;

// ================================================================
// BLOCK
// ================================================================

/** Block as returned by eth_getBlockByNumber with full transactions */
export interface ChainBlock {
  number: bigint;
  hash: Hex;
  parentHash: Hex;
  timestamp: bigint;
  gasUsed: bigint;
  gasLimit: bigint;
  baseFeePerGas: bigint;
  transactions: ChainTransaction[];
  stateRoot: Hex;
  transactionsRoot: Hex;
  receiptsRoot: Hex;

  // -- Computed locally, not from RPC --
  _activityLevel: number; // gasUsed / gasLimit, range 0..1
  _arrivalTime: number; // Date.now() when the explorer received this block
}

// ================================================================
// TRANSACTION
// ================================================================

export interface ChainTransaction {
  hash: Hex;
  from: Hex;
  to: Hex | null; // null = contract creation
  value: bigint;
  gas: bigint;
  gasPrice: bigint;
  input: Hex;
  nonce: number;
  blockNumber: bigint;
  blockHash: Hex;
  transactionIndex: number;
  type: number; // 0=legacy, 1=2930, 2=1559
}

// ================================================================
// RECEIPT
// ================================================================

export interface ChainReceipt {
  transactionHash: Hex;
  status: 'success' | 'reverted';
  blockNumber: bigint;
  blockHash: Hex;
  from: Hex;
  to: Hex | null;
  gasUsed: bigint;
  cumulativeGasUsed: bigint;
  contractAddress: Hex | null;
  logs: ChainLog[];
}

// ================================================================
// LOG
// ================================================================

export interface ChainLog {
  address: Hex;
  topics: Hex[];
  data: Hex;
  blockNumber: bigint;
  transactionHash: Hex;
  logIndex: number;
}

// ================================================================
// NODE STATUS (kora-specific)
// ================================================================

export interface NodeStatus {
  chainId: number;
  validatorIndex: number;
  validatorCount: number;
  uptimeSecs: number;
  currentView: number;
  finalizedCount: number;
  proposedCount: number;
  nullifiedCount: number;
  peerCount: number;
  isLeader: boolean;
}

// ================================================================
// FEE DATA
// ================================================================

export interface FeeDataPoint {
  blockNumber: bigint;
  baseFee: bigint;
  gasUsedRatio: number;
  reward25: bigint;
  reward50: bigint;
  reward75: bigint;
}

// ================================================================
// ADDRESS METADATA
// ================================================================

export interface AddressMeta {
  address: Hex;
  balance: bigint;
  nonce: number;
  isContract: boolean;
  firstSeen: bigint; // block number
  lastSeen: bigint; // block number
  txCount: number; // local count from cached blocks
}

// ================================================================
// CONSENSUS TYPES
// ================================================================

export type RoundPhase =
  | 'proposing'
  | 'notarizing'
  | 'certifying'
  | 'finalizing'
  | 'finalized'
  | 'nullifying'
  | 'nullified';

export type ConsensusEvent =
  | { type: 'notarize'; view: number; payload: string; timestampMs: number }
  | {
      type: 'notarization';
      view: number;
      payload: string;
      seed: string;
      timestampMs: number;
    }
  | {
      type: 'certification';
      view: number;
      payload: string;
      timestampMs: number;
    }
  | { type: 'finalize'; view: number; payload: string; timestampMs: number }
  | {
      type: 'finalization';
      view: number;
      payload: string;
      seed: string;
      blockHeight: number | null;
      timestampMs: number;
    }
  | { type: 'nullify'; timestampMs: number }
  | { type: 'nullification'; view: number; timestampMs: number }
  | {
      type: 'conflictingNotarize';
      timestampMs: number;
      severity: 'critical';
    }
  | {
      type: 'conflictingFinalize';
      timestampMs: number;
      severity: 'critical';
    }
  | { type: 'nullifyFinalize'; timestampMs: number; severity: 'critical' };

export interface ConsensusRound {
  view: number;
  leader: number;
  phase: RoundPhase;
  payload: string | null;
  startedAt: number;
  notarizeAt: number | null;
  notarizationAt: number | null;
  certificationAt: number | null;
  finalizeAt: number | null;
  finalizationAt: number | null;
  nullifyAt: number | null;
  nullificationAt: number | null;
}

export interface SafetyEvent {
  type: 'conflictingNotarize' | 'conflictingFinalize' | 'nullifyFinalize';
  timestampMs: number;
  acknowledged: boolean;
}

// ================================================================
// PENDING TRANSACTION (Phase 2)
// ================================================================

export interface PendingTransaction {
  hash: Hex;
  from: Hex;
  to: Hex | null;
  value: bigint;
  gasPrice: bigint;
  submittedAt: number; // Date.now() when seen
  status: 'pending' | 'included' | 'dropped';
}

// ================================================================
// CONNECTION STATE
// ================================================================

export type ConnectionState =
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'disconnected';

// ================================================================
// EVENT BUS TYPES
// ================================================================

export type SceneName = 'terrain' | 'constellation' | 'waterfall' | 'consensus';

export interface SearchResult {
  type: 'block' | 'transaction' | 'address';
  label: string;
  subtitle: string;
  source: 'cache' | 'rpc';
  navigateTo: string;
}

export interface ValidatorInfo {
  index: number;
  isLocal: boolean;
}

export type BusEvents = {
  // Chain events
  'block:new': ChainBlock;
  'block:reorg': { oldHead: bigint; newHead: bigint };
  'tx:new': ChainTransaction;
  'tx:confirmed': { hash: Hex; receipt: ChainReceipt };
  'tx:pending': ChainTransaction;
  'log:new': ChainLog;

  // Consensus events
  'consensus:event': ConsensusEvent;
  'consensus:round': ConsensusRound;
  'node:status': NodeStatus;

  // Connection state
  'connection:state': ConnectionState;

  // Fee data
  'fee:update': FeeDataPoint[];

  // Address
  'address:seen': Hex;

  // Navigation / UI events
  'scene:navigate': SceneName;
  'entity:select': { type: 'block' | 'tx' | 'address'; id: string };
  'entity:deselect': void;

  // Search
  'search:query': string;
  'search:result': SearchResult[];
};
