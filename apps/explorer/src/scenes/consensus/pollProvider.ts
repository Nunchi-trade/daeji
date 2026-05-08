import { useRef, useCallback } from 'react';
import { getNodeStatus } from '@/data/rpc';
import type { NodeStatus, RoundPhase } from '@/data/types';
import type {
  ConsensusDataSource,
  ConsensusSceneEvent,
  ConsensusSnapshot,
} from './dataSource';

const POLL_INTERVAL_MS = 1000;
const VALIDATOR_COUNT = 4;

/**
 * Poll-based consensus data source (Phase A).
 *
 * Detects round changes by tracking counter deltas:
 * - currentView changed          -> new round started
 * - finalizedCount incremented   -> previous round finalized
 * - nullifiedCount incremented   -> previous round nullified
 *
 * Phase estimation from elapsed time:
 *   0-200ms:    proposing
 *   200-500ms:  notarizing
 *   500-800ms:  certifying
 *   800ms+:     finalizing
 *   1500ms+:    nullifying
 */
export function usePollConsensusProvider(): ConsensusDataSource {
  const handlersRef = useRef<Set<(event: ConsensusSceneEvent) => void>>(
    new Set(),
  );
  const prevRef = useRef<NodeStatus | null>(null);
  const viewStartRef = useRef(Date.now());
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const snapshotRef = useRef<ConsensusSnapshot>({
    currentView: 0,
    currentLeader: 0,
    validatorCount: VALIDATOR_COUNT,
    threshold: 3, // 2f+1 where f=1 for 4 validators
    phase: 'proposing',
    finalizedCount: 0,
    nullifiedCount: 0,
    isLeader: false,
  });

  const emit = useCallback((event: ConsensusSceneEvent) => {
    for (const handler of handlersRef.current) {
      handler(event);
    }
  }, []);

  const poll = useCallback(async () => {
    try {
      const status = await getNodeStatus();
      const prev = prevRef.current;
      prevRef.current = status;

      const snapshot = snapshotRef.current;
      snapshot.currentView = status.currentView;
      snapshot.currentLeader = status.currentView % VALIDATOR_COUNT;
      snapshot.finalizedCount = status.finalizedCount;
      snapshot.nullifiedCount = status.nullifiedCount;
      snapshot.isLeader = status.isLeader;

      if (!prev) return; // first poll, no deltas

      // View changed: new round started
      if (status.currentView !== prev.currentView) {
        viewStartRef.current = Date.now();
        const leader = status.currentView % VALIDATOR_COUNT;

        // Determine what happened to the previous round
        if (status.finalizedCount > prev.finalizedCount) {
          emit({ kind: 'finalized', view: prev.currentView });
        } else if (status.nullifiedCount > prev.nullifiedCount) {
          emit({ kind: 'nullified', view: prev.currentView });
        }

        emit({ kind: 'view_changed', view: status.currentView, leader });
        snapshot.phase = 'proposing';
        emit({ kind: 'phase_changed', phase: 'proposing' });
      }

      // Same view: estimate phase from elapsed time
      if (status.currentView === prev.currentView) {
        const elapsed = Date.now() - viewStartRef.current;
        let estimatedPhase: RoundPhase;

        if (elapsed < 200) estimatedPhase = 'proposing';
        else if (elapsed < 500) estimatedPhase = 'notarizing';
        else if (elapsed < 800) estimatedPhase = 'certifying';
        else if (elapsed < 1500) estimatedPhase = 'finalizing';
        else estimatedPhase = 'nullifying';

        if (estimatedPhase !== snapshot.phase) {
          snapshot.phase = estimatedPhase;
          emit({ kind: 'phase_changed', phase: estimatedPhase });
        }

        // Counter changed without view change (late detection)
        if (status.finalizedCount > prev.finalizedCount) {
          emit({ kind: 'finalized', view: status.currentView });
          snapshot.phase = 'finalized';
          emit({ kind: 'phase_changed', phase: 'finalized' });
        }

        if (status.nullifiedCount > prev.nullifiedCount) {
          emit({ kind: 'nullified', view: status.currentView });
          snapshot.phase = 'nullified';
          emit({ kind: 'phase_changed', phase: 'nullified' });
        }
      }
    } catch (err) {
      console.warn('Consensus poll failed:', err);
    }
  }, [emit]);

  return {
    subscribe(handler) {
      handlersRef.current.add(handler);
      return () => {
        handlersRef.current.delete(handler);
      };
    },
    getState() {
      return snapshotRef.current;
    },
    start() {
      poll();
      intervalRef.current = setInterval(poll, POLL_INTERVAL_MS);
    },
    stop() {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    },
  };
}
