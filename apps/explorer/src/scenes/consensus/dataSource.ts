import type { RoundPhase } from '@/data/types';

// ================================================================
// CONSENSUS SCENE EVENTS
// ================================================================

/** Events the consensus scene reacts to, regardless of data source. */
export type ConsensusSceneEvent =
  | { kind: 'view_changed'; view: number; leader: number }
  | { kind: 'finalized'; view: number }
  | { kind: 'nullified'; view: number }
  | { kind: 'phase_changed'; phase: RoundPhase }
  | { kind: 'safety_violation'; type: string; timestampMs: number }
  // Phase B only (WebSocket streaming):
  | {
      kind: 'vote_received';
      phase: 'notarize' | 'certify' | 'finalize';
      validatorIndex: number;
    }
  | { kind: 'quorum_reached'; phase: 'notarization' | 'certification' };

// ================================================================
// CONSENSUS DATA SOURCE INTERFACE
// ================================================================

export interface ConsensusSnapshot {
  currentView: number;
  currentLeader: number;
  validatorCount: number;
  threshold: number;
  phase: RoundPhase;
  finalizedCount: number;
  nullifiedCount: number;
  isLeader: boolean;
}

export interface ConsensusDataSource {
  /** Subscribe to scene events. Returns unsubscribe function. */
  subscribe(handler: (event: ConsensusSceneEvent) => void): () => void;

  /** Current state snapshot. */
  getState(): ConsensusSnapshot;

  /** Start the data source (polling or WS subscription). */
  start(): void;

  /** Stop the data source. */
  stop(): void;
}
