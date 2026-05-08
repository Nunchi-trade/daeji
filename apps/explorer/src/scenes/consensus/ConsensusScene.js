import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useRef, useEffect } from 'react';
import { Canvas } from '@react-three/fiber';
import { OrthographicCamera } from '@react-three/drei';
import { ValidatorRing } from './ValidatorRing';
import { ThresholdArc } from './ThresholdArc';
import { CenterBlock } from './CenterBlock';
import { RoundHistory } from './RoundHistory';
import { usePollConsensusProvider } from './pollProvider';
import { createInitialSceneState, handleConsensusEvent, } from './sceneState';
// ================================================================
// CONSTANTS
// ================================================================
const VALIDATOR_COUNT = 4;
const RING_RADIUS = 6;
// ================================================================
// SCENE COMPONENT
// ================================================================
function ConsensusScene() {
    // Phase A: always use polling for now (no WS consensus subscription yet)
    const dataSource = usePollConsensusProvider();
    // Mutable scene state ref -- updated imperatively, not via React state
    const sceneStateRef = useRef(createInitialSceneState(VALIDATOR_COUNT));
    useEffect(() => {
        dataSource.start();
        const unsub = dataSource.subscribe((event) => {
            handleConsensusEvent(sceneStateRef.current, event);
        });
        return () => {
            unsub();
            dataSource.stop();
        };
    }, [dataSource]);
    return (_jsxs("div", { style: { width: '100%', height: '100%', position: 'relative' }, children: [_jsxs(Canvas, { style: { width: '100%', height: 'calc(100% - 110px)' }, gl: { antialias: true, alpha: true }, dpr: [1, 2], children: [_jsx(OrthographicCamera, { makeDefault: true, position: [0, 20, 0], rotation: [-Math.PI / 2, 0, 0], zoom: 40, near: 0.1, far: 100 }), _jsx("ambientLight", { intensity: 0.15, color: "#aa7088" }), _jsx("directionalLight", { position: [-5, 10, -5], intensity: 0.6, color: "#e8d8e0" }), _jsx(ValidatorRing, { validatorCount: VALIDATOR_COUNT, radius: RING_RADIUS, sceneStateRef: sceneStateRef }), _jsx(ThresholdArc, { radius: RING_RADIUS + 0.8, sceneStateRef: sceneStateRef }), _jsx(CenterBlock, { sceneStateRef: sceneStateRef })] }), _jsx(RoundHistory, {})] }));
}
export default ConsensusScene;
