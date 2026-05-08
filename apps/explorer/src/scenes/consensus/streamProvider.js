import { useRef, useCallback } from 'react';
import { bus } from '@/data/bus';
const VALIDATOR_COUNT = 4;
/**
 * WebSocket-based consensus data source (Phase B).
 *
 * Receives all Activity types from kora_subscribe("consensus")
 * and maps them to scene events with precise timing.
 */
export function useStreamConsensusProvider() {
    const handlersRef = useRef(new Set());
    const snapshotRef = useRef({
        currentView: 0,
        currentLeader: 0,
        validatorCount: VALIDATOR_COUNT,
        threshold: 3,
        phase: 'proposing',
        finalizedCount: 0,
        nullifiedCount: 0,
        isLeader: false,
    });
    const unsubRef = useRef(null);
    const emit = useCallback((event) => {
        for (const handler of handlersRef.current) {
            handler(event);
        }
    }, []);
    const processEvent = useCallback((event) => {
        const snapshot = snapshotRef.current;
        switch (event.type) {
            case 'notarize': {
                if (event.view !== snapshot.currentView) {
                    snapshot.currentView = event.view;
                    snapshot.currentLeader = event.view % VALIDATOR_COUNT;
                    emit({
                        kind: 'view_changed',
                        view: event.view,
                        leader: snapshot.currentLeader,
                    });
                }
                snapshot.phase = 'notarizing';
                emit({ kind: 'phase_changed', phase: 'notarizing' });
                emit({
                    kind: 'vote_received',
                    phase: 'notarize',
                    validatorIndex: snapshot.currentLeader,
                });
                break;
            }
            case 'notarization': {
                snapshot.phase = 'certifying';
                emit({ kind: 'quorum_reached', phase: 'notarization' });
                emit({ kind: 'phase_changed', phase: 'certifying' });
                break;
            }
            case 'certification': {
                emit({
                    kind: 'vote_received',
                    phase: 'certify',
                    validatorIndex: snapshot.currentLeader,
                });
                break;
            }
            case 'finalize': {
                emit({
                    kind: 'vote_received',
                    phase: 'finalize',
                    validatorIndex: snapshot.currentLeader,
                });
                break;
            }
            case 'finalization': {
                snapshot.finalizedCount++;
                snapshot.phase = 'finalized';
                emit({ kind: 'finalized', view: event.view });
                emit({ kind: 'phase_changed', phase: 'finalized' });
                break;
            }
            case 'nullify': {
                snapshot.phase = 'nullifying';
                emit({ kind: 'phase_changed', phase: 'nullifying' });
                break;
            }
            case 'nullification': {
                snapshot.nullifiedCount++;
                snapshot.phase = 'nullified';
                emit({ kind: 'nullified', view: event.view });
                emit({ kind: 'phase_changed', phase: 'nullified' });
                break;
            }
            case 'conflictingNotarize':
            case 'conflictingFinalize':
            case 'nullifyFinalize': {
                emit({
                    kind: 'safety_violation',
                    type: event.type,
                    timestampMs: event.timestampMs,
                });
                break;
            }
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
            const handler = (event) => processEvent(event);
            bus.on('consensus:event', handler);
            unsubRef.current = () => bus.off('consensus:event', handler);
        },
        stop() {
            unsubRef.current?.();
            unsubRef.current = null;
        },
    };
}
