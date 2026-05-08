// ================================================================
// INITIAL STATE FACTORY
// ================================================================
export function createInitialSceneState(validatorCount) {
    return {
        currentView: 0,
        currentLeader: 0,
        phase: 'idle',
        validatorStates: Array(validatorCount).fill('idle'),
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
export function handleConsensusEvent(state, event) {
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
                    state.arcProgress = Math.min(1, state.arcProgress + 1 / state.validatorStates.length);
                }
                else if (event.phase === 'certify' || event.phase === 'finalize') {
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
