import { useRef, useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import type { MutableRefObject } from 'react';
import type { SceneState } from './sceneState';

// ================================================================
// CONSTANTS
// ================================================================

const ARC_SEGMENTS = 64;
const ARC_WIDTH = 0.15;
const THRESHOLD_FRACTION = 2 / 3;

const COLOR_ROSE = new THREE.Color(0xaa7088); // --rd-rose
const COLOR_WARNING = new THREE.Color(0xc89a68); // --rd-warning

// ================================================================
// COMPONENT
// ================================================================

interface Props {
  radius: number;
  sceneStateRef: MutableRefObject<SceneState>;
}

export function ThresholdArc({ radius, sceneStateRef }: Props) {
  const arcRef = useRef<THREE.Mesh>(null);
  const animatedProgress = useRef(0);

  // Build the arc shape as a ring segment
  const arcGeometry = useMemo(() => {
    return new THREE.RingGeometry(
      radius - ARC_WIDTH,
      radius + ARC_WIDTH,
      ARC_SEGMENTS,
      1,
      0,
      Math.PI * 2,
    );
  }, [radius]);

  // 2/3 threshold marker: use lineSegments (not <line> which maps to SVG)
  const thresholdAngle = THRESHOLD_FRACTION * Math.PI * 2;
  const thresholdGeometry = useMemo(() => {
    const inner = radius - ARC_WIDTH * 2;
    const outer = radius + ARC_WIDTH * 2;
    const geo = new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(
        Math.cos(thresholdAngle) * inner,
        0.01,
        Math.sin(thresholdAngle) * inner,
      ),
      new THREE.Vector3(
        Math.cos(thresholdAngle) * outer,
        0.01,
        Math.sin(thresholdAngle) * outer,
      ),
    ]);
    return geo;
  }, [radius, thresholdAngle]);

  useFrame((_state, delta) => {
    const arc = arcRef.current;
    if (!arc) return;

    const { arcProgress, phase } = sceneStateRef.current;

    // Smooth the arc progress
    animatedProgress.current = THREE.MathUtils.lerp(
      animatedProgress.current,
      arcProgress,
      6 * delta,
    );

    // Update arc geometry: replace with a partial ring
    const progress = Math.max(0, Math.min(1, animatedProgress.current));
    const thetaLength = progress * Math.PI * 2;
    const newGeo = new THREE.RingGeometry(
      radius - ARC_WIDTH,
      radius + ARC_WIDTH,
      ARC_SEGMENTS,
      1,
      -Math.PI / 2, // start from top
      thetaLength,
    );
    arc.geometry.dispose();
    arc.geometry = newGeo;

    // Color: rose normally, amber during nullification
    const mat = arc.material as THREE.MeshBasicMaterial;
    const isNullifying = phase === 'nullifying' || phase === 'nullified';
    const targetColor = isNullifying ? COLOR_WARNING : COLOR_ROSE;
    mat.color.lerp(targetColor, 6 * delta);

    // Opacity based on progress
    mat.opacity = THREE.MathUtils.lerp(
      mat.opacity,
      progress > 0.01 ? 0.6 : 0,
      8 * delta,
    );
  });

  return (
    <group rotation={[-Math.PI / 2, 0, 0]} position={[0, 0.01, 0]}>
      {/* Progress arc */}
      <mesh ref={arcRef} geometry={arcGeometry}>
        <meshBasicMaterial
          color={COLOR_ROSE}
          transparent
          opacity={0}
          side={THREE.DoubleSide}
          depthWrite={false}
          blending={THREE.AdditiveBlending}
        />
      </mesh>

      {/* 2/3 threshold marker */}
      <lineSegments geometry={thresholdGeometry}>
        <lineBasicMaterial
          color="#6a5a68"
          transparent
          opacity={0.3}
          depthWrite={false}
        />
      </lineSegments>
    </group>
  );
}
