# 08 — Scene 4: Consensus Ring

Scene 4 renders the kora consensus lifecycle as a validator ring with animated round phases, center block visualization, safety violation alerts, and a round history bar chart. Built with React Three Fiber.

**Depends on:** [`02-rosedust-tokens.md`](./02-rosedust-tokens.md), [`04-state-stores.md`](./04-state-stores.md)

---

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────────┐
│  ConsensusScene.tsx (React wrapper)                              │
│                                                                  │
│  ┌─ R3F Canvas ─────────────────────────────────────────────┐   │
│  │  OrthographicCamera (top-down, looking at ring)           │   │
│  │                                                           │   │
│  │         V0 (leader)                                       │   │
│  │          ◉                                                │   │
│  │        ╱   ╲                                              │   │
│  │      V3      V1        ← 4 ValidatorNode meshes           │   │
│  │        ╲   ╱              on ring of radius 6             │   │
│  │          V2                                               │   │
│  │                                                           │   │
│  │      ThresholdArc       ← circular progress arc            │   │
│  │      CenterBlock        ← block being formed at ring center│   │
│  │      ConnectionBeams    ← light lines from validators       │   │
│  │                                                           │   │
│  └───────────────────────────────────────────────────────────┘   │
│                                                                  │
│  ┌─ RoundHistoryBar (DOM, below canvas) ────────────────────┐   │
│  │  ██ ██ ██ ██ ░░ ██ ██ ██ ██ ██ ██ ██ ██ ██ ██ ██ ██ ██  │   │
│  │  avg: 450ms   nullified: 1/32 (3%)   violations: 0       │   │
│  └───────────────────────────────────────────────────────────┘   │
│                                                                  │
│  ┌─ SafetyViolationOverlay (DOM, full-screen) ──────────────┐   │
│  │  CSS flash + persistent alert panel                       │   │
│  └───────────────────────────────────────────────────────────┘   │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

### Two-Mode Data Strategy

The consensus scene has two data backends with the same visualization front-end:

| | Phase A (now) | Phase B (future) |
|---|---|---|
| **Data source** | Poll `kora_nodeStatus` every 1s | Subscribe to `kora_subscribe("consensus")` via WebSocket |
| **Events available** | `currentView`, `finalizedCount`, `nullifiedCount`, `isLeader` | All 10 Activity types in real-time |
| **Granularity** | Round-level (detect view changes, counter increments) | Event-level (individual votes, quorum events, safety violations) |
| **Visualization** | Leader rotation, finalize/nullify flash, estimated phase timing | Full vote-by-vote animation, precise phase transitions, connection beams |
| **Implementation** | `PollConsensusProvider` | `StreamConsensusProvider` |

Both providers implement the same `ConsensusDataSource` interface. The scene code never knows which one is active -- it reacts to the same events. Phase B is a drop-in replacement for the data source, not the rendering.

```typescript
// apps/explorer/src/scenes/consensus/dataSource.ts

/** Events the consensus scene reacts to, regardless of data source */
export type ConsensusSceneEvent =
  | { kind: 'view_changed'; view: number; leader: number }
  | { kind: 'finalized'; view: number }
  | { kind: 'nullified'; view: number }
  | { kind: 'phase_changed'; phase: RoundPhase }
  | { kind: 'safety_violation'; type: string; timestampMs: number }
  // Phase B only:
  | { kind: 'vote_received'; phase: 'notarize' | 'certify' | 'finalize'; validatorIndex: number }
  | { kind: 'quorum_reached'; phase: 'notarization' | 'certification' };

export interface ConsensusDataSource {
  /** Subscribe to scene events. Returns unsubscribe function. */
  subscribe(handler: (event: ConsensusSceneEvent) => void): () => void;

  /** Current state snapshot */
  getState(): ConsensusSnapshot;

  /** Start the data source (polling or WS subscription) */
  start(): void;

  /** Stop the data source */
  stop(): void;
}

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

export type RoundPhase =
  | 'proposing'
  | 'notarizing'
  | 'certifying'
  | 'finalizing'
  | 'finalized'
  | 'nullifying'
  | 'nullified'
  | 'idle';
```

---

## New ROSEDUST Tokens for Consensus

These tokens extend the base ROSEDUST palette for consensus-specific states. Add to `rosedust.css`:

```css
:root {
  /* ── Consensus phase colors ── */
  --rd-consensus-propose:   #cc90a8;   /* rose-bright: leader proposing */
  --rd-consensus-vote:      #aa7088;   /* rose: voting in progress */
  --rd-consensus-finalize:  #d8c8a0;   /* bone-bright: finalization */
  --rd-consensus-nullify:   #c89a68;   /* warning: round timeout */
  --rd-consensus-idle:      #3a303a;   /* text-ghost: between rounds */
}
```

These map to existing palette values -- no new colors are introduced. They are semantic aliases that make the consensus scene code self-documenting.

---

## 8.1 Consensus Scene Container

### File: `apps/explorer/src/scenes/consensus/ConsensusScene.tsx`

```tsx
import { useRef, useEffect, useMemo } from 'react';
import { Canvas } from '@react-three/fiber';
import { OrthographicCamera } from '@react-three/drei';
import { ValidatorRing } from './ValidatorRing';
import { ThresholdArc } from './ThresholdArc';
import { CenterBlock } from './CenterBlock';
import { ConnectionBeams } from './ConnectionBeams';
import { RoundHistoryBar } from './RoundHistoryBar';
import { SafetyViolationOverlay } from './SafetyViolationOverlay';
import { usePollConsensusProvider } from './pollProvider';
import { useStreamConsensusProvider } from './streamProvider';
import { useConsensusStore } from '../../stores/consensus';
import type { ConsensusDataSource, ConsensusSceneEvent } from './dataSource';

const VALIDATOR_COUNT = 4;
const RING_RADIUS = 6;

export function ConsensusScene() {
  const wsAvailable = useConsensusStore((s) => s.subscribed);

  // Select data source: WebSocket if available, polling otherwise
  const pollProvider = usePollConsensusProvider();
  const streamProvider = useStreamConsensusProvider();
  const dataSource: ConsensusDataSource = wsAvailable ? streamProvider : pollProvider;

  // Scene-level state refs (mutable, not React state -- for animation loop)
  const sceneStateRef = useRef({
    currentView: 0,
    currentLeader: 0,
    phase: 'idle' as RoundPhase,
    validatorStates: Array(VALIDATOR_COUNT).fill('idle') as ValidatorVisualState[],
    arcProgress: 0,       // 0-1, threshold arc fill
    centerState: 'empty' as CenterBlockState,
    safetyFlash: false,
  });

  // Subscribe to data source events
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
        style={{ width: '100%', height: 'calc(100% - 100px)' }}
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

        {/* Connection beams (Phase B: vote lines from validators to center) */}
        <ConnectionBeams
          validatorCount={VALIDATOR_COUNT}
          radius={RING_RADIUS}
          sceneStateRef={sceneStateRef}
        />
      </Canvas>

      {/* Round history bar chart (DOM, below canvas) */}
      <RoundHistoryBar />

      {/* Safety violation flash overlay (DOM, full-screen) */}
      <SafetyViolationOverlay />
    </div>
  );
}
```

### Scene event handler

```typescript
// apps/explorer/src/scenes/consensus/sceneState.ts

import type { ConsensusSceneEvent, RoundPhase } from './dataSource';

export type ValidatorVisualState =
  | 'idle'
  | 'proposing'
  | 'voting'
  | 'voted'
  | 'certifying'
  | 'finalized'
  | 'nullifying';

export type CenterBlockState =
  | 'empty'       // no round in progress
  | 'wireframe'   // proposing: faint outline
  | 'filling'     // notarizing: partially solid
  | 'solid'       // notarization achieved: dream-colored solid
  | 'crystallized' // finalized: bone-bright, sharp edges
  | 'shattering'; // nullified: breaking apart

export interface SceneState {
  currentView: number;
  currentLeader: number;
  phase: RoundPhase;
  validatorStates: ValidatorVisualState[];
  arcProgress: number;
  centerState: CenterBlockState;
  safetyFlash: boolean;
}

/**
 * Update scene state in response to a consensus event.
 * Mutates state in-place (this is a mutable ref, not React state).
 */
export function handleConsensusEvent(state: SceneState, event: ConsensusSceneEvent): void {
  switch (event.kind) {
    case 'view_changed': {
      state.currentView = event.view;
      state.currentLeader = event.leader;
      state.phase = 'proposing';
      state.arcProgress = 0;
      state.centerState = 'wireframe';

      // Reset all validators to idle, then set leader to proposing
      state.validatorStates.fill('idle');
      state.validatorStates[event.leader] = 'proposing';
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
          // All validators flash finalized
          state.validatorStates.fill('finalized');
          break;
        case 'nullifying':
          state.centerState = 'empty'; // will animate to shattering
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
      if (event.phase === 'notarize') {
        state.validatorStates[idx] = 'voting';
        state.arcProgress = Math.min(1, state.arcProgress + 1 / 4);
      } else if (event.phase === 'certify' || event.phase === 'finalize') {
        state.validatorStates[idx] = 'certifying';
      }
      break;
    }

    case 'quorum_reached': {
      if (event.phase === 'notarization') {
        state.centerState = 'solid';
        state.arcProgress = 1;
        // All voting validators upgrade to voted
        state.validatorStates = state.validatorStates.map(
          (s) => s === 'voting' ? 'voted' : s,
        );
      }
      break;
    }

    case 'safety_violation': {
      state.safetyFlash = true;
      // Flash resets after 200ms (handled by animation system)
      break;
    }
  }
}
```

### Checklist

- [ ] `ConsensusScene.tsx` mounts R3F Canvas
- [ ] Orthographic camera: top-down, positioned at (0, 20, 0), looking down
- [ ] Camera zoom: 40 (ring fills ~60% of viewport width)
- [ ] Ring radius: 6 world units
- [ ] 4 validators evenly spaced (at 0, 90, 180, 270 degrees)
- [ ] Data source selection: WebSocket if `subscribed === true`, polling otherwise
- [ ] Scene state stored in mutable ref (not React state -- no re-renders per event)
- [ ] `handleConsensusEvent()` maps events to visual state mutations
- [ ] All sub-components receive `sceneStateRef` to read visual state in animation frames

---

## 8.2 Validator Nodes

### File: `apps/explorer/src/scenes/consensus/ValidatorNode.tsx`

```tsx
import { useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import type { MutableRefObject } from 'react';
import type { SceneState, ValidatorVisualState } from './sceneState';

/** ROSEDUST colors as Three.js Color objects */
const COLORS = {
  idle:        new THREE.Color(0x3a303a), // --rd-text-ghost
  proposing:   new THREE.Color(0xcc90a8), // --rd-consensus-propose
  voting:      new THREE.Color(0x6a4058), // --rd-rose-dim (half-voted)
  voted:       new THREE.Color(0xaa7088), // --rd-consensus-vote
  certifying:  new THREE.Color(0xd8c8a0), // --rd-consensus-finalize
  finalized:   new THREE.Color(0xd8c8a0), // --rd-bone-bright (flash)
  nullifying:  new THREE.Color(0xc89a68), // --rd-consensus-nullify
};

/** Size multiplier for each visual state */
const SIZE_SCALE: Record<ValidatorVisualState, number> = {
  idle:        0.8,
  proposing:   1.4,
  voting:      1.0,
  voted:       1.1,
  certifying:  1.2,
  finalized:   1.3,
  nullifying:  0.9,
};

interface Props {
  index: number;
  angle: number;           // radians, position on ring
  radius: number;
  sceneStateRef: MutableRefObject<SceneState>;
}

export function ValidatorNode({ index, angle, radius, sceneStateRef }: Props) {
  const meshRef = useRef<THREE.Mesh>(null);
  const glowRef = useRef<THREE.Mesh>(null);
  const labelRef = useRef<THREE.Sprite>(null);

  // Stable position on ring (never changes)
  const position: [number, number, number] = [
    Math.cos(angle) * radius,
    0,
    Math.sin(angle) * radius,
  ];

  useFrame((_state, delta) => {
    const mesh = meshRef.current;
    const glow = glowRef.current;
    if (!mesh || !glow) return;

    const sceneState = sceneStateRef.current;
    const visualState = sceneState.validatorStates[index];
    const isLeader = sceneState.currentLeader === index;

    // Target color and size
    const targetColor = COLORS[visualState] ?? COLORS.idle;
    const targetSize = SIZE_SCALE[visualState] ?? 0.8;

    // Smooth interpolation (lerp)
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

    // Pulse animation for proposing state
    if (visualState === 'proposing') {
      const pulse = 1 + Math.sin(Date.now() * 0.004) * 0.1; // 2.4s cycle, +/-10%
      mesh.scale.multiplyScalar(pulse);
    }

    // Flash on finalized (brief white burst)
    if (visualState === 'finalized') {
      const flash = Math.max(0, 1 - (Date.now() % 500) / 200);
      mat.emissive.lerp(new THREE.Color(0xffffff), flash * 0.5);
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

    // Leader crown: ring above node
    if (isLeader) {
      // Rotate the crown glow
      glow.rotation.y += delta * 0.5;
    }
  });

  return (
    <group position={position}>
      {/* Main node: octahedron (more geometric than sphere) */}
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

      {/* Glow ring: additive halo around node */}
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
          map={createLabelTexture(`V${index}`)}
          transparent
          opacity={0.7}
          depthTest={false}
        />
      </sprite>
    </group>
  );
}

/**
 * Create a text label texture using Canvas2D.
 * Rendered once per validator, cached for lifetime.
 */
function createLabelTexture(text: string): THREE.CanvasTexture {
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
  return texture;
}
```

### ValidatorRing wrapper

```tsx
// apps/explorer/src/scenes/consensus/ValidatorRing.tsx

import type { MutableRefObject } from 'react';
import { ValidatorNode } from './ValidatorNode';
import type { SceneState } from './sceneState';

interface Props {
  validatorCount: number;
  radius: number;
  sceneStateRef: MutableRefObject<SceneState>;
}

export function ValidatorRing({ validatorCount, radius, sceneStateRef }: Props) {
  const nodes = Array.from({ length: validatorCount }, (_, i) => {
    const angle = (i / validatorCount) * Math.PI * 2; // evenly spaced
    return (
      <ValidatorNode
        key={i}
        index={i}
        angle={angle}
        radius={radius}
        sceneStateRef={sceneStateRef}
      />
    );
  });

  return <group>{nodes}</group>;
}
```

### Checklist

- [ ] `ValidatorNode` component: octahedron mesh with MeshStandardMaterial
- [ ] Position: `(cos(angle) * radius, 0, sin(angle) * radius)` for each validator
- [ ] 4 validators at 0, 90, 180, 270 degrees
- [ ] Size: scales with visual state (proposing=1.4x, idle=0.8x)
- [ ] Color states: idle=ghost, proposing=rose-bright, voting=rose-dim, voted=rose, certifying=bone-bright, nullifying=warning
- [ ] Color transitions: smooth lerp (not instant snap)
- [ ] Pulse animation: proposing state oscillates +/-10% on 2.4s cycle
- [ ] Flash animation: finalized state bursts white for 200ms
- [ ] Glow ring: additive blended ring, visible for active states, invisible for idle
- [ ] Label: `V0`-`V3` sprite above each node, JetBrains Mono
- [ ] Leader indicator: the proposing node is the `currentView % 4` validator, shown by `proposing` state

---

## 8.3 Round Phase Animation (Phase A -- Polling Only)

### File: `apps/explorer/src/scenes/consensus/pollProvider.ts`

Phase A polls `kora_nodeStatus` every 1 second and derives round events from counter changes.

```typescript
import { useRef, useCallback } from 'react';
import { httpClient } from '../../rpc/client';
import type { ConsensusDataSource, ConsensusSceneEvent, ConsensusSnapshot, RoundPhase } from './dataSource';

const POLL_INTERVAL_MS = 1000;
const VALIDATOR_COUNT = 4;

interface NodeStatus {
  chainId: number;
  validatorIndex: number;
  currentView: number;
  finalizedCount: number;
  proposedCount: number;
  nullifiedCount: number;
  peerCount: number;
  isLeader: boolean;
}

/**
 * Poll-based consensus data source.
 *
 * Detects round changes by tracking counter deltas:
 * - currentView changed          -> new round started
 * - finalizedCount incremented   -> previous round finalized
 * - nullifiedCount incremented   -> previous round nullified
 * - isLeader changed             -> leader rotation
 *
 * Phase estimation:
 * Since we only see snapshots, we estimate the round phase based on time
 * elapsed since the view changed. Typical round duration is 1-2s:
 *   0-200ms:    proposing
 *   200-500ms:  notarizing
 *   500-800ms:  certifying
 *   800ms+:     finalizing (waiting for quorum)
 *   1500ms+:    nullifying (timeout approaching)
 */
export function usePollConsensusProvider(): ConsensusDataSource {
  const handlersRef = useRef<Set<(event: ConsensusSceneEvent) => void>>(new Set());
  const prevRef = useRef<NodeStatus | null>(null);
  const viewStartRef = useRef(Date.now());
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const snapshotRef = useRef<ConsensusSnapshot>({
    currentView: 0,
    currentLeader: 0,
    validatorCount: VALIDATOR_COUNT,
    threshold: 3, // 2f+1 where f=1 for 4 validators
    phase: 'idle',
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
      const status: NodeStatus = await httpClient.request({
        method: 'kora_nodeStatus',
      });

      const prev = prevRef.current;
      prevRef.current = status;

      // Update snapshot
      const snapshot = snapshotRef.current;
      snapshot.currentView = status.currentView;
      snapshot.currentLeader = status.currentView % VALIDATOR_COUNT;
      snapshot.finalizedCount = status.finalizedCount;
      snapshot.nullifiedCount = status.nullifiedCount;
      snapshot.isLeader = status.isLeader;

      if (!prev) return; // first poll, no deltas to compute

      // View changed: new round started
      if (status.currentView !== prev.currentView) {
        viewStartRef.current = Date.now();
        const leader = status.currentView % VALIDATOR_COUNT;

        emit({ kind: 'view_changed', view: status.currentView, leader });

        // Determine what happened to the previous round
        if (status.finalizedCount > prev.finalizedCount) {
          emit({ kind: 'finalized', view: prev.currentView });
        } else if (status.nullifiedCount > prev.nullifiedCount) {
          emit({ kind: 'nullified', view: prev.currentView });
        }

        snapshot.phase = 'proposing';
        emit({ kind: 'phase_changed', phase: 'proposing' });
      }

      // Same view: estimate phase from elapsed time
      if (status.currentView === prev.currentView) {
        const elapsed = Date.now() - viewStartRef.current;
        let estimatedPhase: RoundPhase;

        if (elapsed < 200)       estimatedPhase = 'proposing';
        else if (elapsed < 500)  estimatedPhase = 'notarizing';
        else if (elapsed < 800)  estimatedPhase = 'certifying';
        else if (elapsed < 1500) estimatedPhase = 'finalizing';
        else                     estimatedPhase = 'nullifying';

        if (estimatedPhase !== snapshot.phase) {
          snapshot.phase = estimatedPhase;
          emit({ kind: 'phase_changed', phase: estimatedPhase });
        }
      }

      // Counter changed without view change (edge case: late detection)
      if (
        status.currentView === prev.currentView &&
        status.finalizedCount > prev.finalizedCount
      ) {
        emit({ kind: 'finalized', view: status.currentView });
        snapshot.phase = 'finalized';
        emit({ kind: 'phase_changed', phase: 'finalized' });
      }

      if (
        status.currentView === prev.currentView &&
        status.nullifiedCount > prev.nullifiedCount
      ) {
        emit({ kind: 'nullified', view: status.currentView });
        snapshot.phase = 'nullified';
        emit({ kind: 'phase_changed', phase: 'nullified' });
      }
    } catch (err) {
      console.warn('Consensus poll failed:', err);
    }
  }, [emit]);

  return {
    subscribe(handler) {
      handlersRef.current.add(handler);
      return () => handlersRef.current.delete(handler);
    },
    getState() {
      return snapshotRef.current;
    },
    start() {
      poll(); // immediate first poll
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
```

### Phase ring animation

The phase ring is a circular progress arc that fills as time elapses within a round. In Phase A, this is a time-based estimate. In Phase B, it will be driven by actual vote counts.

```typescript
// In ThresholdArc.tsx (useFrame callback):
// Phase A: arc fills based on elapsed time since view start
const elapsed = Date.now() - viewStartMs;
const estimatedRoundDuration = 1200; // 1.2s typical
const progress = Math.min(1, elapsed / estimatedRoundDuration);
// Apply ease-out curve for natural deceleration feel
const easedProgress = 1 - Math.pow(1 - progress, 3);
```

### Checklist

- [ ] Poll `kora_nodeStatus` every 1s via HTTP RPC
- [ ] Track `currentView` changes -> emit `view_changed` event
- [ ] Track `finalizedCount` increments -> emit `finalized` + flash finalize animation
- [ ] Track `nullifiedCount` increments -> emit `nullified` + flash nullify animation (red/amber)
- [ ] Phase estimation: map elapsed time to proposing/notarizing/certifying/finalizing/nullifying
- [ ] Phase ring: circular progress arc fills as round progresses (time-based estimate)
- [ ] Leader = `currentView % 4`
- [ ] Timing estimate: round time approximately 1-2s (configurable)
- [ ] Emit `phase_changed` events for scene state updates
- [ ] Error handling: warn on poll failure, continue polling

---

## 8.4 Round Phase Animation (Phase B -- WebSocket Streaming)

### File: `apps/explorer/src/scenes/consensus/streamProvider.ts`

When consensus streaming is available (after doc 11 backend work), this provider replaces the polling provider with real-time event-driven animation.

```typescript
import { useRef, useCallback } from 'react';
import { bus } from '../../bus/events';
import type { ConsensusEvent } from '../../stores/consensus';
import type { ConsensusDataSource, ConsensusSceneEvent, ConsensusSnapshot, RoundPhase } from './dataSource';

const VALIDATOR_COUNT = 4;

/**
 * WebSocket-based consensus data source.
 *
 * Receives all 10 Activity types from kora_subscribe("consensus")
 * and maps them to scene events with precise timing.
 */
export function useStreamConsensusProvider(): ConsensusDataSource {
  const handlersRef = useRef<Set<(event: ConsensusSceneEvent) => void>>(new Set());
  const snapshotRef = useRef<ConsensusSnapshot>({
    currentView: 0,
    currentLeader: 0,
    validatorCount: VALIDATOR_COUNT,
    threshold: 3,
    phase: 'idle',
    finalizedCount: 0,
    nullifiedCount: 0,
    isLeader: false,
  });

  const unsubRef = useRef<(() => void) | null>(null);

  const emit = useCallback((event: ConsensusSceneEvent) => {
    for (const handler of handlersRef.current) {
      handler(event);
    }
  }, []);

  /** Map a raw ConsensusEvent (from WS) to one or more ConsensusSceneEvents */
  const processEvent = useCallback((event: ConsensusEvent) => {
    const snapshot = snapshotRef.current;

    switch (event.type) {
      case 'notarize': {
        // This node voted to notarize -> leader proposed, voting started
        if (event.view !== snapshot.currentView) {
          snapshot.currentView = event.view;
          snapshot.currentLeader = event.view % VALIDATOR_COUNT;
          emit({ kind: 'view_changed', view: event.view, leader: snapshot.currentLeader });
        }

        snapshot.phase = 'notarizing';
        emit({ kind: 'phase_changed', phase: 'notarizing' });

        // Propose animation: light beam from leader to center
        emit({ kind: 'vote_received', phase: 'notarize', validatorIndex: snapshot.currentLeader });
        break;
      }

      case 'notarization': {
        // Quorum achieved: enough validators notarized
        // Arcs from each voting validator to center
        snapshot.phase = 'certifying';
        emit({ kind: 'quorum_reached', phase: 'notarization' });
        emit({ kind: 'phase_changed', phase: 'certifying' });
        break;
      }

      case 'certification': {
        // This node voted to finalize
        // Outer ring fills with certification color
        emit({ kind: 'vote_received', phase: 'certify', validatorIndex: snapshot.currentLeader });
        break;
      }

      case 'finalize': {
        // Individual finalization vote
        emit({ kind: 'vote_received', phase: 'finalize', validatorIndex: snapshot.currentLeader });
        break;
      }

      case 'finalization': {
        // Block finalized: pulse outward, terrain tile appears
        snapshot.finalizedCount++;
        snapshot.phase = 'finalized';
        emit({ kind: 'finalized', view: event.view });
        emit({ kind: 'phase_changed', phase: 'finalized' });
        break;
      }

      case 'nullify': {
        // This node voted to nullify (timeout)
        snapshot.phase = 'nullifying';
        emit({ kind: 'phase_changed', phase: 'nullifying' });
        break;
      }

      case 'nullification': {
        // View skipped: ring fragments, red flash, next round auto-starts
        snapshot.nullifiedCount++;
        snapshot.phase = 'nullified';
        emit({ kind: 'nullified', view: event.view });
        emit({ kind: 'phase_changed', phase: 'nullified' });
        break;
      }

      case 'conflictingNotarize':
      case 'conflictingFinalize':
      case 'nullifyFinalize': {
        emit({ kind: 'safety_violation', type: event.type, timestampMs: event.timestampMs });
        break;
      }
    }
  }, [emit]);

  return {
    subscribe(handler) {
      handlersRef.current.add(handler);
      return () => handlersRef.current.delete(handler);
    },
    getState() {
      return snapshotRef.current;
    },
    start() {
      unsubRef.current = (() => {
        const handler = (event: ConsensusEvent) => processEvent(event);
        bus.on('consensus:event', handler);
        return () => bus.off('consensus:event', handler);
      })();
    },
    stop() {
      unsubRef.current?.();
      unsubRef.current = null;
    },
  };
}
```

### Visual mapping for Phase B events

| Event | Visual |
|-------|--------|
| `notarize` | Leader node brightens, light beam from leader to center ring |
| `notarization` | All voted nodes glow rose, threshold arc completes, center block solidifies (dream-colored) |
| `certification` | Individual double-ring on the certifying validator |
| `finalize` | Same as certification (cumulative visual) |
| `finalization` | All nodes flash bright (200ms), center block crystallizes (bone-bright), block drops to waterfall |
| `nullify` | Voting validator dims to warning amber |
| `nullification` | Center wireframe shatters (8 fragments, outward, fade), ring dims to ghost, red flash |

### Checklist

- [ ] Subscribe to `consensus:event` bus events (from `kora_subscribe("consensus")`)
- [ ] Map all 10 Activity types to scene events
- [ ] `notarize` -> propose animation: light beam from leader to center
- [ ] `notarization` -> all participating nodes glow rose, threshold arc fills
- [ ] `finalization` -> all nodes flash white (200ms), center crystallizes, block drops
- [ ] `nullification` -> ring fragments, red flash, next round auto-starts
- [ ] Phase transitions are event-driven (precise), not time-estimated
- [ ] Same `ConsensusDataSource` interface as poll provider -- drop-in replacement
- [ ] Safety violations forwarded as `safety_violation` events

---

## 8.5 Safety Violation Rendering

### File: `apps/explorer/src/scenes/consensus/SafetyViolationOverlay.tsx`

Safety violations are rendered as dramatic, persistent UI events. They use CSS overlays (not WebGL) so they appear on top of all scenes and panels.

```tsx
import { useEffect, useState, useCallback } from 'react';
import { useConsensusStore } from '../../stores/consensus';
import type { SafetyEvent } from '../../stores/consensus';

/** Violation descriptions for the alert panel */
const VIOLATION_INFO: Record<string, { title: string; description: string }> = {
  conflictingNotarize: {
    title: 'CONFLICTING NOTARIZE DETECTED',
    description: 'A validator produced two conflicting notarization votes for the same view. This indicates Byzantine behavior.',
  },
  conflictingFinalize: {
    title: 'CONFLICTING FINALIZE DETECTED',
    description: 'A validator produced two conflicting finalization votes for the same view. This is a critical safety violation.',
  },
  nullifyFinalize: {
    title: 'NULLIFY-FINALIZE CONFLICT',
    description: 'A validator sent both nullify and finalize for the same view. This should never happen in normal operation.',
  },
};

export function SafetyViolationOverlay() {
  const violations = useConsensusStore((s) => s.safetyViolations);
  const unacknowledged = useConsensusStore((s) => s.unacknowledgedViolations);
  const acknowledge = useConsensusStore((s) => s.acknowledgeSafetyViolation);
  const [flashActive, setFlashActive] = useState(false);

  // Flash on new violation
  useEffect(() => {
    if (unacknowledged > 0) {
      setFlashActive(true);
      const timer = setTimeout(() => setFlashActive(false), 200);
      return () => clearTimeout(timer);
    }
  }, [unacknowledged]);

  // Latest unacknowledged violation for the alert panel
  const latestUnacked = violations.find((v) => !v.acknowledged);

  return (
    <>
      {/* Full-screen red flash: CSS overlay, 200ms */}
      {flashActive && (
        <div
          style={{
            position: 'fixed',
            inset: 0,
            background: 'rgba(204, 85, 85, 0.30)', // --rd-danger at 30%
            pointerEvents: 'none',
            zIndex: 10000,
            animation: 'safety-flash 200ms ease-out forwards',
          }}
        />
      )}

      {/* Persistent alert panel: does NOT auto-dismiss */}
      {latestUnacked && (
        <div
          style={{
            position: 'fixed',
            top: '50%',
            left: '50%',
            transform: 'translate(-50%, -50%)',
            background: 'rgba(8, 8, 12, 0.92)',
            backdropFilter: 'blur(12px)',
            border: '1px solid rgba(204, 85, 85, 0.4)',
            borderLeft: '3px solid #cc5555',
            padding: '24px 32px',
            fontFamily: '"JetBrains Mono", monospace',
            color: '#c8b8c0',
            zIndex: 10001,
            maxWidth: 480,
            lineHeight: 1.6,
          }}
        >
          <div
            style={{
              fontSize: '10px',
              letterSpacing: '0.28em',
              color: '#cc5555',
              marginBottom: 12,
              textTransform: 'uppercase',
            }}
          >
            SAFETY VIOLATION
          </div>

          <div style={{ fontSize: '14px', color: '#e8d8e0', marginBottom: 8 }}>
            {VIOLATION_INFO[latestUnacked.type]?.title ?? latestUnacked.type.toUpperCase()}
          </div>

          <div style={{ fontSize: '11px', color: '#6a5a68', marginBottom: 4 }}>
            View {latestUnacked.view ?? '?'} &mdash;{' '}
            {new Date(latestUnacked.timestampMs).toISOString().replace('T', ' ').slice(0, 23)}
          </div>

          <div style={{ fontSize: '12px', marginBottom: 16 }}>
            {VIOLATION_INFO[latestUnacked.type]?.description}
          </div>

          <button
            onClick={() => {
              const idx = violations.indexOf(latestUnacked);
              if (idx >= 0) acknowledge(idx);
            }}
            style={{
              background: 'transparent',
              border: '1px solid rgba(255, 255, 255, 0.14)',
              color: '#c8b8c0',
              fontFamily: '"JetBrains Mono", monospace',
              fontSize: '10px',
              letterSpacing: '0.12em',
              textTransform: 'uppercase',
              padding: '6px 16px',
              cursor: 'pointer',
            }}
          >
            ACKNOWLEDGE
          </button>
        </div>
      )}

      {/* Inline style for the flash animation keyframes */}
      <style>{`
        @keyframes safety-flash {
          0%   { opacity: 1; }
          100% { opacity: 0; }
        }
      `}</style>
    </>
  );
}
```

### Violation types and visuals

| Violation | Ring Visual | Flash | Alert |
|-----------|-------------|-------|-------|
| `conflictingNotarize` | Two colored beams from same validator node diverging in different directions | 200ms red flash at 30% opacity | "A validator produced two conflicting notarization votes" |
| `conflictingFinalize` | Ring border fractures: red glow pulses along cracks | 200ms red flash at 30% opacity | "Two conflicting finalization votes" |
| `nullifyFinalize` | Node flickers between amber (nullify) and rose (finalize) rapidly | 200ms red flash at 30% opacity | "Both nullify and finalize for the same view" |

All three violations:
- Persist in UI until user clicks ACKNOWLEDGE
- Increment the `safety_violations` counter in the status panel
- Show a red LED dot next to the consensus health indicator
- Trigger an optional audio cue (dissonant chord, user-enabled in settings)

### Checklist

- [ ] `ConflictingNotarize`: two colored beams from same validator (different directions)
- [ ] `ConflictingFinalize`: ring fractures with red glow
- [ ] Full-screen flash overlay: CSS (not WebGL), `--rd-danger` at 30%, 200ms ease-out
- [ ] Persistent alert panel: center-screen, glass style, does NOT auto-dismiss
- [ ] Alert panel shows violation type, view number, timestamp, description
- [ ] ACKNOWLEDGE button: dismisses alert, marks violation as acknowledged in store
- [ ] Sound cue: optional, user-enabled, dissonant chord (not implemented here -- wired in audio layer)
- [ ] Red LED indicator persists in status panel until acknowledged

---

## 8.6 Round History Bar Chart

### File: `apps/explorer/src/scenes/consensus/RoundHistoryBar.tsx`

A DOM-based bar chart below the ring, showing the last 32 rounds as vertical bars. Height encodes round duration, color encodes outcome.

```tsx
import { useMemo, useState } from 'react';
import { useConsensusStore } from '../../stores/consensus';

const BAR_COUNT = 32;
const BAR_WIDTH = 12;         // px per bar
const BAR_GAP = 3;            // px between bars
const MAX_BAR_HEIGHT = 60;    // px
const CHART_HEIGHT = 80;      // px total

/** ROSEDUST colors for round outcomes */
const COLORS = {
  finalized: '#aa7088',   // --rd-rose
  nullified: '#3a303a',   // --rd-text-ghost
  violation: '#cc5555',   // --rd-danger
};

interface RoundRecord {
  view: number;
  durationMs: number;
  outcome: 'finalized' | 'nullified' | 'violation';
}

export function RoundHistoryBar() {
  const roundHistory = useConsensusStore((s) => s.roundHistory);
  const [hoveredIndex, setHoveredIndex] = useState<number | null>(null);

  // Take last BAR_COUNT rounds
  const records: RoundRecord[] = useMemo(() => {
    return roundHistory.slice(-BAR_COUNT).map((round) => ({
      view: round.view,
      durationMs: round.finalizationAt
        ? round.finalizationAt - round.startedAt
        : round.nullificationAt
          ? round.nullificationAt - round.startedAt
          : 0,
      outcome: round.phase === 'finalized'
        ? 'finalized'
        : round.phase === 'nullified'
          ? 'nullified'
          : 'finalized', // default
    }));
  }, [roundHistory]);

  // Scale: max bar height based on longest round
  const maxDuration = Math.max(500, ...records.map((r) => r.durationMs));

  // Aggregate stats
  const nullifiedCount = records.filter((r) => r.outcome === 'nullified').length;
  const avgMs = records.length > 0
    ? Math.round(records.reduce((s, r) => s + r.durationMs, 0) / records.length)
    : 0;
  const violationCount = records.filter((r) => r.outcome === 'violation').length;

  return (
    <div
      style={{
        width: '100%',
        height: `${CHART_HEIGHT + 30}px`,
        background: 'rgba(8, 8, 12, 0.45)',
        borderTop: '1px solid rgba(255, 255, 255, 0.04)',
        padding: '8px 16px',
        fontFamily: '"JetBrains Mono", monospace',
        position: 'relative',
      }}
    >
      {/* Section label */}
      <div
        style={{
          fontSize: '10px',
          letterSpacing: '0.28em',
          color: '#6a5a68',
          textTransform: 'uppercase',
          marginBottom: 4,
        }}
      >
        ROUND HISTORY
      </div>

      {/* Bar chart */}
      <div
        style={{
          display: 'flex',
          alignItems: 'flex-end',
          gap: `${BAR_GAP}px`,
          height: `${MAX_BAR_HEIGHT}px`,
        }}
      >
        {records.map((record, i) => {
          const barHeight = Math.max(2, (record.durationMs / maxDuration) * MAX_BAR_HEIGHT);
          const isHovered = hoveredIndex === i;

          return (
            <div
              key={record.view}
              onMouseEnter={() => setHoveredIndex(i)}
              onMouseLeave={() => setHoveredIndex(null)}
              style={{
                width: `${BAR_WIDTH}px`,
                height: `${barHeight}px`,
                background: COLORS[record.outcome],
                opacity: isHovered ? 1 : 0.7,
                transition: 'opacity 80ms ease-out',
                cursor: 'pointer',
                position: 'relative',
              }}
              title={`View ${record.view}: ${record.durationMs}ms (${record.outcome})`}
            />
          );
        })}
      </div>

      {/* Hover tooltip */}
      {hoveredIndex !== null && records[hoveredIndex] && (
        <div
          style={{
            position: 'absolute',
            bottom: `${CHART_HEIGHT + 8}px`,
            left: `${16 + hoveredIndex * (BAR_WIDTH + BAR_GAP)}px`,
            background: 'rgba(8, 8, 12, 0.9)',
            border: '1px solid rgba(255, 255, 255, 0.07)',
            padding: '4px 8px',
            fontSize: '9px',
            color: '#c8b8c0',
            whiteSpace: 'nowrap',
            pointerEvents: 'none',
            zIndex: 10,
          }}
        >
          <div>View {records[hoveredIndex].view}</div>
          <div>{records[hoveredIndex].durationMs}ms</div>
          <div style={{ color: COLORS[records[hoveredIndex].outcome] }}>
            {records[hoveredIndex].outcome}
          </div>
        </div>
      )}

      {/* Stats line */}
      <div
        style={{
          fontSize: '9px',
          color: '#6a5a68',
          marginTop: 4,
          display: 'flex',
          gap: 16,
        }}
      >
        <span>avg: {avgMs}ms</span>
        <span>
          nullified: {nullifiedCount}/{records.length}
          {records.length > 0 && ` (${Math.round((nullifiedCount / records.length) * 100)}%)`}
        </span>
        <span style={{ color: violationCount > 0 ? '#cc5555' : undefined }}>
          violations: {violationCount}
        </span>
      </div>
    </div>
  );
}
```

### Checklist

- [ ] Last 32 rounds as vertical bars
- [ ] Bar height = round duration (ms), scaled relative to longest round in window
- [ ] Bar color: `--rd-rose` (finalized), `--rd-text-ghost` (nullified), `--rd-danger` (violation)
- [ ] Minimum bar height: 2px (even for very fast rounds)
- [ ] Hover: show tooltip with view number, duration (ms), outcome
- [ ] Stats line: avg duration, nullification rate (count + percent), violation count
- [ ] Section label: "ROUND HISTORY" in standard ROSEDUST label style
- [ ] Violation count renders in `--rd-danger` red when > 0

---

## 8.7 Center Block Visualization

### File: `apps/explorer/src/scenes/consensus/CenterBlock.tsx`

The center of the ring shows the block currently being built. It progresses through visual states as the round advances.

```tsx
import { useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import type { MutableRefObject } from 'react';
import type { SceneState, CenterBlockState } from './sceneState';

/** Colors for each center block state */
const CENTER_COLORS: Record<CenterBlockState, THREE.Color> = {
  empty:        new THREE.Color(0x3a303a), // --rd-text-ghost
  wireframe:    new THREE.Color(0x5a5a78), // --rd-dream-dim
  filling:      new THREE.Color(0x7a7a98), // --rd-dream
  solid:        new THREE.Color(0x9494b4), // --rd-dream-bright
  crystallized: new THREE.Color(0xd8c8a0), // --rd-bone-bright
  shattering:   new THREE.Color(0xcc5555), // --rd-danger
};

/** Opacity for each state */
const CENTER_OPACITY: Record<CenterBlockState, number> = {
  empty:        0.05,
  wireframe:    0.2,
  filling:      0.4,
  solid:        0.7,
  crystallized: 1.0,
  shattering:   0.6,
};

interface Props {
  sceneStateRef: MutableRefObject<SceneState>;
}

export function CenterBlock({ sceneStateRef }: Props) {
  const meshRef = useRef<THREE.Mesh>(null);
  const wireRef = useRef<THREE.LineSegments>(null);
  const flashRef = useRef<THREE.Mesh>(null);

  // Fragment meshes for shatter animation
  const fragmentRefs = useRef<THREE.Mesh[]>([]);
  const fragmentVelocities = useRef<THREE.Vector3[]>(
    Array.from({ length: 8 }, () =>
      new THREE.Vector3(
        (Math.random() - 0.5) * 4,
        Math.random() * 2,
        (Math.random() - 0.5) * 4,
      ),
    ),
  );

  const shatterStartRef = useRef(0);

  useFrame((_state, delta) => {
    const mesh = meshRef.current;
    const wire = wireRef.current;
    const flash = flashRef.current;
    if (!mesh || !wire) return;

    const { centerState } = sceneStateRef.current;
    const targetColor = CENTER_COLORS[centerState];
    const targetOpacity = CENTER_OPACITY[centerState];

    // Smooth color transition
    const mat = mesh.material as THREE.MeshStandardMaterial;
    mat.color.lerp(targetColor, 6 * delta);
    mat.opacity = THREE.MathUtils.lerp(mat.opacity, targetOpacity, 6 * delta);
    mat.emissive.lerp(targetColor, 4 * delta);
    mat.emissiveIntensity = THREE.MathUtils.lerp(
      mat.emissiveIntensity,
      centerState === 'crystallized' ? 0.8 : 0.2,
      4 * delta,
    );

    // Wireframe visibility: show for wireframe + filling states
    const wireMat = wire.material as THREE.LineBasicMaterial;
    const wireVisible = centerState === 'wireframe' || centerState === 'filling';
    wireMat.opacity = THREE.MathUtils.lerp(
      wireMat.opacity,
      wireVisible ? 0.4 : 0,
      6 * delta,
    );

    // Scale: smaller for empty/wireframe, full for solid/crystallized
    const targetScale = centerState === 'empty' ? 0.3
      : centerState === 'wireframe' ? 0.6
      : centerState === 'filling' ? 0.8
      : centerState === 'shattering' ? 0.3  // shrinks as it shatters
      : 1.0;
    const currentScale = mesh.scale.x;
    mesh.scale.setScalar(THREE.MathUtils.lerp(currentScale, targetScale, 6 * delta));
    wire.scale.copy(mesh.scale);

    // Gentle rotation
    mesh.rotation.y += delta * 0.3;
    wire.rotation.y = mesh.rotation.y;

    // Crystallized flash (200ms white overlay)
    if (flash && centerState === 'crystallized') {
      const flashMat = flash.material as THREE.MeshBasicMaterial;
      flashMat.opacity = Math.max(0, flashMat.opacity - delta * 5); // fade out
    }

    // Shatter animation
    if (centerState === 'shattering') {
      if (shatterStartRef.current === 0) {
        shatterStartRef.current = Date.now();
      }
      const elapsed = (Date.now() - shatterStartRef.current) / 1000;

      fragmentRefs.current.forEach((frag, i) => {
        if (!frag) return;
        const vel = fragmentVelocities.current[i];
        frag.position.add(vel.clone().multiplyScalar(delta));
        frag.material.opacity = Math.max(0, 1 - elapsed * 2); // fade over 500ms
        frag.rotation.x += delta * 3;
        frag.rotation.z += delta * 2;
      });
    } else {
      shatterStartRef.current = 0;
      // Reset fragments to center
      fragmentRefs.current.forEach((frag) => {
        if (frag) {
          frag.position.set(0, 0, 0);
          frag.material.opacity = 0;
        }
      });
    }
  });

  return (
    <group position={[0, 0, 0]}>
      {/* Solid block mesh */}
      <mesh ref={meshRef}>
        <boxGeometry args={[1.2, 1.2, 1.2]} />
        <meshStandardMaterial
          color={CENTER_COLORS.empty}
          emissive={CENTER_COLORS.empty}
          emissiveIntensity={0.2}
          transparent
          opacity={0.05}
          roughness={0.2}
          metalness={0.7}
        />
      </mesh>

      {/* Wireframe overlay */}
      <lineSegments ref={wireRef}>
        <edgesGeometry args={[new THREE.BoxGeometry(1.2, 1.2, 1.2)]} />
        <lineBasicMaterial
          color="#9494b4"
          transparent
          opacity={0}
          depthWrite={false}
        />
      </lineSegments>

      {/* Flash overlay (white, used on crystallize) */}
      <mesh ref={flashRef}>
        <boxGeometry args={[1.3, 1.3, 1.3]} />
        <meshBasicMaterial
          color="#ffffff"
          transparent
          opacity={0}
          depthWrite={false}
          blending={THREE.AdditiveBlending}
        />
      </mesh>

      {/* Shatter fragments (8 small boxes) */}
      {Array.from({ length: 8 }, (_, i) => (
        <mesh
          key={i}
          ref={(el) => { if (el) fragmentRefs.current[i] = el; }}
        >
          <boxGeometry args={[0.3, 0.3, 0.3]} />
          <meshStandardMaterial
            color="#cc5555"
            transparent
            opacity={0}
            emissive="#cc5555"
            emissiveIntensity={0.5}
          />
        </mesh>
      ))}
    </group>
  );
}
```

### Center block state machine

```
 empty ──── wireframe ──── filling ──── solid ──── crystallized
   │            │              │           │            │
   │            │              │           │            ├── (block drops to waterfall)
   │            │              │           │            └── (reset to empty)
   │            │              │           │
   │            └──────────────┴───────────┴── shattering
   │                                                │
   └────────────────────────────────────────────────┘
                                             (reset to empty after 500ms)
```

### Hash art on block face

When the block crystallizes (finalized), a hash art pattern renders on one face. This uses the same `generateHashArt()` from `lib/hashArt.ts` as the block detail view, rendered to a Canvas2D texture and applied to the box material's map.

```typescript
// In CenterBlock.tsx -- on crystallize event:
const texture = new THREE.CanvasTexture(generateHashArtCanvas(blockHash, 64));
(meshRef.current!.material as THREE.MeshStandardMaterial).map = texture;
```

### Checklist

- [ ] Center of ring: box geometry (1.2 x 1.2 x 1.2 world units)
- [ ] State progression: empty -> wireframe -> filling -> solid -> crystallized
- [ ] Empty: nearly invisible (5% opacity), small (0.3x scale)
- [ ] Wireframe: faint edges visible (dream-dim), 0.6x scale
- [ ] Filling: partially opaque (40%), dream-colored, 0.8x scale
- [ ] Solid: 70% opacity, dream-bright, full scale
- [ ] Crystallized: 100% opacity, bone-bright, emissive glow, 200ms white flash
- [ ] Shattering: 8 fragment boxes fly outward with random velocities, fade over 500ms, red-colored
- [ ] Hash art: on crystallize, block hash generates a texture on the block face
- [ ] Gentle Y-axis rotation: 0.3 rad/s
- [ ] All transitions smooth (lerp, 6x delta rate)

---

## 8.8 Interaction

### File: `apps/explorer/src/scenes/consensus/interaction.ts`

```typescript
import { bus } from '../../bus/events';

/** Click on validator node: show validator info tooltip */
export function onValidatorClick(validatorIndex: number): void {
  bus.emit('entity:select', {
    type: 'validator',
    index: validatorIndex,
  });
}

/** Click on center block: navigate to block detail (if finalized) */
export function onCenterBlockClick(blockNumber: bigint | null): void {
  if (blockNumber !== null) {
    bus.emit('entity:select', {
      type: 'block',
      number: blockNumber,
    });
  }
}
```

### R3F click handling

Click detection in R3F uses the `onClick` prop on mesh elements:

```tsx
// In ValidatorNode.tsx:
<mesh
  ref={meshRef}
  onClick={() => onValidatorClick(index)}
  onPointerOver={() => { document.body.style.cursor = 'pointer'; }}
  onPointerOut={() => { document.body.style.cursor = 'default'; }}
>
```

```tsx
// In CenterBlock.tsx:
<mesh
  ref={meshRef}
  onClick={() => {
    if (sceneStateRef.current.centerState === 'crystallized') {
      onCenterBlockClick(currentBlockNumber);
    }
  }}
>
```

### Scene transition

When navigating to or from the consensus scene, the camera performs a smooth transition:

```typescript
// In SceneManager.tsx -- consensus scene transition
// Entering: camera starts zoomed out (zoom=20) and zooms in to zoom=40 over 500ms
// Leaving: camera zooms out from 40 to 20, scene fades to 0 opacity

// CSS transition on the canvas wrapper:
// opacity: 1, transform: translateY(0)
// -> opacity: 0, transform: translateY(12px)
// duration: 500ms, ease: var(--rd-ease-out)
```

### Checklist

- [ ] Click validator node: emit `entity:select` for validator info tooltip
- [ ] Click center block: navigate to block detail (only when crystallized/finalized)
- [ ] Hover: cursor changes to pointer on interactive elements
- [ ] Hover round history bar: tooltip shows view number, duration, outcome
- [ ] Scene transition: 500ms crossfade with `translateY(12px)` entrance animation
- [ ] Camera zoom animation on scene enter (20 -> 40 over 500ms)

---

## 8.9 Two-Mode Implementation

### Code structure

The scene visualization code is completely decoupled from the data source:

```
apps/explorer/src/scenes/consensus/
├── ConsensusScene.tsx       # 8.1  Container: picks data source, holds R3F Canvas
├── dataSource.ts            # 8.1  ConsensusDataSource interface + types
├── sceneState.ts            # 8.1  handleConsensusEvent(), SceneState type
├── pollProvider.ts          # 8.3  Phase A: poll kora_nodeStatus every 1s
├── streamProvider.ts        # 8.4  Phase B: subscribe to kora_subscribe("consensus")
├── ValidatorNode.tsx        # 8.2  Single validator octahedron mesh
├── ValidatorRing.tsx        # 8.2  Ring of N validator nodes
├── ThresholdArc.tsx         # 8.3  Circular progress arc
├── CenterBlock.tsx          # 8.7  Block being formed at ring center
├── ConnectionBeams.tsx      # 8.4  Light lines from validators to center (Phase B)
├── RoundHistoryBar.tsx      # 8.6  DOM bar chart below canvas
├── SafetyViolationOverlay.tsx # 8.5  CSS flash + alert panel
└── interaction.ts           # 8.8  Click handlers
```

### Data source swap

The swap happens in `ConsensusScene.tsx`:

```typescript
// Phase A (now):
const wsAvailable = useConsensusStore((s) => s.subscribed);
// subscribed = false because WebSocket doesn't exist yet
// -> uses usePollConsensusProvider()

// Phase B (future):
// After doc 11 backend work, the consensus store's `subscribed` flag
// is set to true when kora_subscribe("consensus") succeeds
// -> uses useStreamConsensusProvider()
// Zero changes to ValidatorNode, CenterBlock, ThresholdArc, etc.
```

### What degrades in Phase A

| Feature | Phase A (polling) | Phase B (streaming) |
|---------|-------------------|---------------------|
| Leader identification | Yes (currentView % 4) | Yes (from event data) |
| Finalize detection | Yes (counter increment) | Yes (finalization event) |
| Nullify detection | Yes (counter increment) | Yes (nullification event) |
| Individual vote animation | No (only see round outcomes) | Yes (vote-by-vote) |
| Connection beams | No (no vote data) | Yes (beam per vote) |
| Phase timing | Estimated (time-based) | Precise (event timestamps) |
| Safety violations | No (not available via poll) | Yes (all 3 types) |
| Quorum progress | Estimated (time-based arc) | Precise (vote count arc) |

### Checklist

- [ ] `ConsensusDataSource` interface used by all scene components
- [ ] `PollConsensusProvider` implements interface for Phase A
- [ ] `StreamConsensusProvider` implements interface for Phase B
- [ ] Phase B is a drop-in replacement: same events, same handler functions
- [ ] Scene components never import `pollProvider` or `streamProvider` directly
- [ ] `ConsensusScene.tsx` selects data source based on `useConsensusStore.subscribed`
- [ ] Degradation is graceful: Phase A shows round-level animation, Phase B adds vote-level detail

---

## 8.10 Verification

End-to-end acceptance criteria:

- [ ] **Ring renders:** 4 validator nodes appear in ring formation (0, 90, 180, 270 degrees)
- [ ] **Leader rotation:** Leader node changes on each round (highlighted with `proposing` state)
- [ ] **Leader identification:** `currentView % 4` correctly maps to validator index
- [ ] **Phase progression:** Nodes transition through idle -> proposing -> voting -> finalized states
- [ ] **Finalize animation:** When `finalizedCount` increments, all nodes flash white for 200ms, center block crystallizes
- [ ] **Nullify animation:** When `nullifiedCount` increments, nodes dim to amber, center shatters into 8 fragments
- [ ] **Threshold arc:** Circular arc around ring fills during round progression
- [ ] **Center block:** Progresses through empty -> wireframe -> filling -> solid -> crystallized
- [ ] **Round history:** Bar chart shows last 32 rounds with correct colors (rose/ghost/red)
- [ ] **Round history hover:** Tooltip shows view, duration, and outcome
- [ ] **Stats line:** Average duration, nullification rate, violation count
- [ ] **Safety violation:** Full-screen red flash on violation event
- [ ] **Safety alert:** Persistent panel with violation details, ACKNOWLEDGE button
- [ ] **Phase A polling:** Works with `kora_nodeStatus` only, no WebSocket required
- [ ] **Phase B streaming:** Picks up `consensus:event` bus events when WS available
- [ ] **Data source swap:** Seamless transition from Phase A to Phase B
- [ ] **Click interaction:** Validator click shows info, center block click opens BlockDetail
- [ ] **Scene transition:** Smooth 500ms crossfade with translateY(12px) entrance
- [ ] **60fps:** Solid 60fps with ring, arc, center block, and history all rendering

---

## File Manifest

```
apps/explorer/src/scenes/consensus/
├── ConsensusScene.tsx          # 8.1  React container, R3F Canvas, data source selection
├── dataSource.ts               # 8.1  ConsensusDataSource interface, event types, snapshot type
├── sceneState.ts               # 8.1  SceneState type, handleConsensusEvent(), visual state types
├── pollProvider.ts             # 8.3  Phase A: poll kora_nodeStatus, derive events from deltas
├── streamProvider.ts           # 8.4  Phase B: map kora_subscribe events to scene events
├── ValidatorNode.tsx           # 8.2  Single node: octahedron, color lerp, pulse, flash, glow
├── ValidatorRing.tsx           # 8.2  Ring wrapper: positions N nodes at equal angles
├── ThresholdArc.tsx            # 8.3  Circular progress arc (THREE.RingGeometry, partial fill)
├── CenterBlock.tsx             # 8.7  Center block: box, wireframe, flash, shatter fragments
├── ConnectionBeams.tsx         # 8.4  Phase B: light lines from voters to center
├── RoundHistoryBar.tsx         # 8.6  DOM bar chart: 32 rounds, color-coded, hover tooltips
├── SafetyViolationOverlay.tsx  # 8.5  CSS flash, persistent alert panel, ACKNOWLEDGE button
└── interaction.ts              # 8.8  Click handlers: validator info, block detail navigation
```
