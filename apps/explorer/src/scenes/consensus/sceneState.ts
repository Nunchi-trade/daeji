import type { RoundPhase } from '@/data/types';
import type { ConsensusSceneEvent } from './dataSource';

// ================================================================
// VISUAL STATE TYPES
// ================================================================

export type ValidatorVisualState =
  | 'idle'
  | 'proposing'
  | 'voting'
  | 'voted'
  | 'certifying'
  | 'finalized'
  | 'nullifying';

export type CenterBlockState =
  | 'empty'
  | 'wireframe'
  | 'filling'
  | 'solid'
  | 'crystallized'
  | 'shattering';

export interface SceneState {
  currentView: number;
  currentLeader: number;
  phase: RoundPhase | 'idle';
  validatorStates: ValidatorVisualState[];
  arcProgress: number;
  centerState: CenterBlockState;
  safetyFlash: boolean;
  viewStartMs: number;
}

// ================================================================
// INITIAL STATE FACTORY
// ================================================================

export function createInitialSceneState(validatorCount: number): SceneState {
  return {
    currentView: 0,
    currentLeader: 0,
    phase: 'idle',
    validatorStates: Array(validatorCount).fill('idle') as ValidatorVisualState[],
    arcProgress: 0,
    centerState: 'empty',
    safetyFlash: false,
    viewStartMs: Date.now(),
  };
}

// ================================================================
// EVENT HANDLER
// ================================================================

/**
 * Update scene state in response to a consensus event.
 * Mutates state in-place (mutable ref, not React state).
 */
export function handleConsensusEvent(
  state: SceneState,
  event: ConsensusSceneEvent,
): void {
  switch (event.kind) {
    case 'view_changed': {
      state.currentView = event.view;
      state.currentLeader = event.leader;
      state.phase = 'proposing';
      state.arcProgress = 0;
      state.centerState = 'wireframe';
      state.viewStartMs = Date.now();

      // Reset all validators to idle, then set leader to proposing
      state.validatorStates.fill('idle');
      if (event.leader < state.validatorStates.length) {
        state.validatorStates[event.leader] = 'proposing';
      }
      break;
    }

    case 'phase_changed': {
      state.phase = event.phase;

      switch (event.phase) {
        case 'notarizing':
          state.centerState = 'filling';
          break;
        case 'certifying':
          state.centerState = 'solid';
          break;
        case 'finalized':
          state.centerState = 'crystallized';
          state.validatorStates.fill('finalized');
          break;
        case 'nullifying':
          state.centerState = 'empty';
          state.validatorStates.fill('nullifying');
          break;
        case 'nullified':
          state.centerState = 'shattering';
          break;
      }
      break;
    }

    case 'finalized': {
      state.phase = 'finalized';
      state.arcProgress = 1;
      state.centerState = 'crystallized';
      state.validatorStates.fill('finalized');
      break;
    }

    case 'nullified': {
      state.phase = 'nullified';
      state.arcProgress = 0;
      state.centerState = 'shattering';
      state.validatorStates.fill('nullifying');
      break;
    }

    case 'vote_received': {
      // Phase B only: individual vote animation
      const idx = event.validatorIndex;
      if (idx >= 0 && idx < state.validatorStates.length) {
        if (event.phase === 'notarize') {
          state.validatorStates[idx] = 'voting';
          state.arcProgress = Math.min(
            1,
            state.arcProgress + 1 / state.validatorStates.length,
          );
        } else if (event.phase === 'certify' || event.phase === 'finalize') {
          state.validatorStates[idx] = 'certifying';
        }
      }
      break;
    }

    case 'quorum_reached': {
      if (event.phase === 'notarization') {
        state.centerState = 'solid';
        state.arcProgress = 1;
        for (let i = 0; i < state.validatorStates.length; i++) {
          if (state.validatorStates[i] === 'voting') {
            state.validatorStates[i] = 'voted';
          }
        }
      }
      break;
    }

    case 'safety_violation': {
      state.safetyFlash = true;
      break;
    }
  }
}
