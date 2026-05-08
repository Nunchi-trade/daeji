import { useRef, useEffect } from 'react';
import { Canvas } from '@react-three/fiber';
import { OrthographicCamera } from '@react-three/drei';
import { ValidatorRing } from './ValidatorRing';
import { ThresholdArc } from './ThresholdArc';
import { CenterBlock } from './CenterBlock';
import { RoundHistory } from './RoundHistory';
import { usePollConsensusProvider } from './pollProvider';
import {
  createInitialSceneState,
  handleConsensusEvent,
} from './sceneState';
import type { ConsensusSceneEvent } from './dataSource';

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

    const unsub = dataSource.subscribe((event: ConsensusSceneEvent) => {
      handleConsensusEvent(sceneStateRef.current, event);
    });

    return () => {
      unsub();
      dataSource.stop();
    };
  }, [dataSource]);

  return (
    <div style={{ width: '100%', height: '100%', position: 'relative' }}>
      {/* R3F Canvas: the ring visualization */}
      <Canvas
        style={{ width: '100%', height: 'calc(100% - 110px)' }}
        gl={{ antialias: true, alpha: true }}
        dpr={[1, 2]}
      >
        <OrthographicCamera
          makeDefault
          position={[0, 20, 0]}
          rotation={[-Math.PI / 2, 0, 0]}
          zoom={40}
          near={0.1}
          far={100}
        />

        {/* Ambient light: very low, rose-tinted */}
        <ambientLight intensity={0.15} color="#aa7088" />

        {/* Directional light from upper-left */}
        <directionalLight
          position={[-5, 10, -5]}
          intensity={0.6}
          color="#e8d8e0"
        />

        {/* Validator nodes on the ring */}
        <ValidatorRing
          validatorCount={VALIDATOR_COUNT}
          radius={RING_RADIUS}
          sceneStateRef={sceneStateRef}
        />

        {/* Threshold arc around the ring */}
        <ThresholdArc
          radius={RING_RADIUS + 0.8}
          sceneStateRef={sceneStateRef}
        />

        {/* Center block (forming/crystallizing/shattering) */}
        <CenterBlock sceneStateRef={sceneStateRef} />
      </Canvas>

      {/* Round history bar chart (DOM, below canvas) */}
      <RoundHistory />
    </div>
  );
}

export default ConsensusScene;
