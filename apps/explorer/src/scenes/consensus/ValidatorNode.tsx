import { useRef, useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import type { MutableRefObject } from 'react';
import type { SceneState, ValidatorVisualState } from './sceneState';

// ================================================================
// ROSEDUST COLOR MAPPING
// ================================================================

const COLORS: Record<ValidatorVisualState, THREE.Color> = {
  idle: new THREE.Color(0x3a303a), // --rd-text-ghost
  proposing: new THREE.Color(0xcc90a8), // --rd-consensus-propose
  voting: new THREE.Color(0x6a4058), // --rd-rose-dim
  voted: new THREE.Color(0xaa7088), // --rd-consensus-vote
  certifying: new THREE.Color(0xd8c8a0), // --rd-consensus-finalize
  finalized: new THREE.Color(0xd8c8a0), // --rd-bone-bright
  nullifying: new THREE.Color(0xc89a68), // --rd-consensus-nullify
};

const SIZE_SCALE: Record<ValidatorVisualState, number> = {
  idle: 0.8,
  proposing: 1.4,
  voting: 1.0,
  voted: 1.1,
  certifying: 1.2,
  finalized: 1.3,
  nullifying: 0.9,
};

// Reusable scratch color to avoid per-frame allocations
const _white = new THREE.Color(0xffffff);

// ================================================================
// LABEL TEXTURE CACHE
// ================================================================

const labelCache = new Map<string, THREE.CanvasTexture>();

function getLabelTexture(text: string): THREE.CanvasTexture {
  const cached = labelCache.get(text);
  if (cached) return cached;

  const canvas = document.createElement('canvas');
  canvas.width = 64;
  canvas.height = 24;

  const ctx = canvas.getContext('2d')!;
  ctx.font = '14px "JetBrains Mono", monospace';
  ctx.fillStyle = '#c8b8c0'; // --rd-text
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(text, 32, 12);

  const texture = new THREE.CanvasTexture(canvas);
  texture.needsUpdate = true;
  labelCache.set(text, texture);
  return texture;
}

// ================================================================
// COMPONENT
// ================================================================

interface Props {
  index: number;
  angle: number;
  radius: number;
  sceneStateRef: MutableRefObject<SceneState>;
}

export function ValidatorNode({ index, angle, radius, sceneStateRef }: Props) {
  const meshRef = useRef<THREE.Mesh>(null);
  const glowRef = useRef<THREE.Mesh>(null);

  const position = useMemo<[number, number, number]>(
    () => [Math.cos(angle) * radius, 0, Math.sin(angle) * radius],
    [angle, radius],
  );

  const labelTexture = useMemo(() => getLabelTexture(`V${index}`), [index]);

  useFrame((_state, delta) => {
    const mesh = meshRef.current;
    const glow = glowRef.current;
    if (!mesh || !glow) return;

    const sceneState = sceneStateRef.current;
    const visualState = sceneState.validatorStates[index] ?? 'idle';
    const isLeader = sceneState.currentLeader === index;

    // Target color and size
    const targetColor = COLORS[visualState];
    const targetSize = SIZE_SCALE[visualState];

    // Smooth color transition
    const mat = mesh.material as THREE.MeshStandardMaterial;
    mat.color.lerp(targetColor, 8 * delta);
    mat.emissive.lerp(targetColor, 4 * delta);
    mat.emissiveIntensity = THREE.MathUtils.lerp(
      mat.emissiveIntensity,
      visualState === 'idle' ? 0.1 : 0.5,
      4 * delta,
    );

    // Smooth scale
    const currentScale = mesh.scale.x;
    const newScale = THREE.MathUtils.lerp(currentScale, targetSize, 6 * delta);
    mesh.scale.setScalar(newScale);

    // Pulse animation for proposing state (2.4s cycle, +/-10%)
    if (visualState === 'proposing') {
      const pulse = 1 + Math.sin(Date.now() * 0.004) * 0.1;
      mesh.scale.multiplyScalar(pulse);
    }

    // Flash on finalized (brief white burst)
    if (visualState === 'finalized') {
      const flash = Math.max(0, 1 - (Date.now() % 500) / 200);
      mat.emissive.lerp(_white, flash * 0.5);
    }

    // Glow ring: visible only for active states
    const glowMat = glow.material as THREE.MeshBasicMaterial;
    const glowVisible = visualState !== 'idle';
    glowMat.opacity = THREE.MathUtils.lerp(
      glowMat.opacity,
      glowVisible ? 0.3 : 0,
      4 * delta,
    );
    glow.scale.setScalar(newScale * 1.8);

    // Leader crown: rotate the glow ring
    if (isLeader) {
      glow.rotation.y += delta * 0.5;
    }
  });

  return (
    <group position={position}>
      {/* Main node: octahedron */}
      <mesh ref={meshRef} rotation={[0, angle, 0]}>
        <octahedronGeometry args={[0.5, 0]} />
        <meshStandardMaterial
          color={COLORS.idle}
          emissive={COLORS.idle}
          emissiveIntensity={0.1}
          transparent
          opacity={0.9}
          roughness={0.3}
          metalness={0.6}
        />
      </mesh>

      {/* Glow ring: additive halo */}
      <mesh ref={glowRef} rotation={[-Math.PI / 2, 0, 0]}>
        <ringGeometry args={[0.6, 1.0, 32]} />
        <meshBasicMaterial
          color={COLORS.voted}
          transparent
          opacity={0}
          side={THREE.DoubleSide}
          depthWrite={false}
          blending={THREE.AdditiveBlending}
        />
      </mesh>

      {/* Label: V0, V1, V2, V3 */}
      <sprite position={[0, 1.2, 0]} scale={[1.5, 0.4, 1]}>
        <spriteMaterial
          map={labelTexture}
          transparent
          opacity={0.7}
          depthTest={false}
        />
      </sprite>
    </group>
  );
}
