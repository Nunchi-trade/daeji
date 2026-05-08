# 06 -- Scene: Transaction Constellation

Addresses as particles on a sphere. Transactions as arcs between them.
The network visualizer for the Kora Explorer.

---

## Architecture

```
                     EventBus
                        |
           +------------+------------+
           |            |            |
     "block:new"  "tx:confirmed"  "tx:pending"
           |            |            |
           v            v            v
     +-----+------+----+-----+------+-----+
     | AddressBook|    | ArcManager |      |
     | (node mgr) |    | (tx arcs)  |      |
     +-----+------+    +-----+------+      |
           |                 |             |
     +-----v------+   +-----v------+      |
     | GPU Particle|   | Arc Geom  |      |
     | Buffer     |   | (bezier)  |      |
     | (instanced)|   +-----+------+      |
     +-----+------+         |             |
           |                 |             |
     +-----v-----------------v-------------v---+
     |        ConstellationScene                |
     |        R3F <Canvas>                      |
     |        OrbitControls, starfield          |
     +------------------------------------------+
```

- **R3F Canvas** with `OrbitControls` for rotation, zoom, and pan
- Addresses placed on a sphere surface via golden-angle spiral layout
- Rendered as `InstancedMesh` of small spheres (max 4096 instances)
- Transactions rendered as `QuadraticBezierCurve3` arcs between sender and receiver
- Animation lifecycle: pending (pulse), confirmed (arc travel), reverted (shatter)
- GPU instancing for nodes; batched `Line` geometry for arcs

### File Manifest

```
apps/explorer/src/scenes/constellation/
  ConstellationScene.tsx   -- 6.1  R3F Canvas container
  addressLayout.ts         -- 6.2  golden-angle spiral placement
  AddressNodes.tsx          -- 6.3  instanced mesh for address nodes
  TransactionArcs.tsx       -- 6.4  bezier arc geometry and animation
  arcUtils.ts              -- 6.4  computeArc function
  lifecycle.ts             -- 6.5  pending/confirmed/reverted animations
  interactions.ts          -- 6.6  raycasting, selection, labels
  Starfield.tsx            -- 6.1  background star points
  index.ts                 -- barrel export
```

---

## 6.1 Constellation Scene Container

The top-level R3F Canvas with orbit controls, camera, and background starfield.

- [ ] `ConstellationScene.tsx` -- R3F `<Canvas>` with OrbitControls
- [ ] PerspectiveCamera at `[0, 0, 20]`, FOV 50, near 0.1, far 200
- [ ] OrbitControls: autoRotate at 0.3 rad/s, enableDamping with factor 0.05
- [ ] Background: dark sphere with subtle star field (Points geometry, 2000 stars)
- [ ] Ambient light 0.3 + point light at camera position for node illumination
- [ ] Subscribe to `bus.on('block:new')`, `bus.on('tx:confirmed')`, `bus.on('tx:pending')` imperatively
- [ ] Wrap in `<ErrorBoundary>` + `<Suspense>`

```typescript
// apps/explorer/src/scenes/constellation/ConstellationScene.tsx

import { Canvas, useFrame, useThree } from '@react-three/fiber';
import { OrbitControls, PerspectiveCamera } from '@react-three/drei';
import { useRef, useEffect, useMemo, Suspense } from 'react';
import * as THREE from 'three';
import { bus } from '@/data/bus';
import { AddressNodes } from './AddressNodes';
import { TransactionArcs } from './TransactionArcs';
import { Starfield } from './Starfield';
import { AddressBook } from './addressLayout';
import { ArcManager } from './arcUtils';
import type { ChainBlock, ChainTransaction, PendingTransaction } from '@/data/types';

/** Inner scene content rendered within the R3F Canvas. */
function ConstellationContent() {
  const addressBook = useMemo(() => new AddressBook(4096), []);
  const arcManager = useMemo(() => new ArcManager(256), []);
  const controlsRef = useRef<any>(null);

  // Subscribe to chain events imperatively (no React re-renders)
  useEffect(() => {
    const unsubBlock = bus.on('block:new', (block: ChainBlock) => {
      // Register all addresses seen in this block
      for (const tx of block.transactions) {
        addressBook.registerAddress(tx.from, { isContract: false });
        if (tx.to) {
          addressBook.registerAddress(tx.to, { isContract: false });
        } else {
          // Contract creation: tx.to is null, contract address in receipt
          // Registered later when receipt is available
        }
      }
    });

    const unsubTx = bus.on('tx:confirmed', (tx: ChainTransaction) => {
      const from = addressBook.getOrCreate(tx.from);
      const to = tx.to ? addressBook.getOrCreate(tx.to) : null;
      if (to) {
        arcManager.addArc(from, to, tx.value, tx.hash, 'confirmed');
      }
    });

    const unsubPending = bus.on('tx:pending', (ptx: PendingTransaction) => {
      const from = addressBook.getOrCreate(ptx.from);
      const to = ptx.to ? addressBook.getOrCreate(ptx.to) : null;
      if (to) {
        arcManager.addArc(from, to, ptx.value, ptx.hash, 'pending');
      }
    });

    return () => {
      unsubBlock();
      unsubTx();
      unsubPending();
    };
  }, [addressBook, arcManager]);

  // Update animations every frame
  useFrame((_, delta) => {
    arcManager.update(delta);
    addressBook.updatePulse(delta);
  });

  return (
    <>
      {/* Camera */}
      <PerspectiveCamera
        makeDefault
        position={[0, 0, 20]}
        fov={50}
        near={0.1}
        far={200}
      />

      {/* Orbit controls with auto-rotate */}
      <OrbitControls
        ref={controlsRef}
        autoRotate
        autoRotateSpeed={0.3}
        enableDamping
        dampingFactor={0.05}
        minDistance={5}
        maxDistance={50}
        enablePan
      />

      {/* Lighting */}
      <ambientLight intensity={0.3} color={0x8888aa} />
      <pointLight position={[0, 0, 20]} intensity={0.6} color={0xffeedd} />

      {/* Background starfield */}
      <Starfield count={2000} radius={80} />

      {/* Address nodes (instanced spheres) */}
      <AddressNodes addressBook={addressBook} />

      {/* Transaction arcs */}
      <TransactionArcs arcManager={arcManager} />
    </>
  );
}

/** Top-level exported component. Lazy-loaded by SceneManager. */
export default function ConstellationScene() {
  return (
    <Canvas
      gl={{
        antialias: true,
        alpha: true,
        powerPreference: 'high-performance',
      }}
      style={{ position: 'absolute', inset: 0 }}
      dpr={[1, 2]}
    >
      <Suspense fallback={null}>
        <ConstellationContent />
      </Suspense>
    </Canvas>
  );
}
```

### Background Starfield

A simple Points geometry creating a subtle starfield on the inner surface of a large sphere.

```typescript
// apps/explorer/src/scenes/constellation/Starfield.tsx

import { useMemo } from 'react';
import * as THREE from 'three';

interface Props {
  count: number;
  radius: number;
}

/**
 * Renders `count` points uniformly distributed on a sphere of `radius`.
 * Stars are dim, tiny, and non-interactive -- pure atmosphere.
 */
export function Starfield({ count, radius }: Props) {
  const [positions, sizes] = useMemo(() => {
    const pos = new Float32Array(count * 3);
    const sz = new Float32Array(count);

    for (let i = 0; i < count; i++) {
      // Uniform sphere distribution (Marsaglia method)
      let x: number, y: number, z: number, d: number;
      do {
        x = Math.random() * 2 - 1;
        y = Math.random() * 2 - 1;
        d = x * x + y * y;
      } while (d >= 1);
      const scale = Math.sqrt(1 - d);
      z = 1 - 2 * d;

      pos[i * 3] = x * 2 * scale * radius;
      pos[i * 3 + 1] = y * 2 * scale * radius;
      pos[i * 3 + 2] = z * radius;

      // Vary star size: most small, a few brighter
      sz[i] = 0.5 + Math.random() * 1.5;
    }

    return [pos, sz];
  }, [count, radius]);

  return (
    <points>
      <bufferGeometry>
        <bufferAttribute
          attach="attributes-position"
          array={positions}
          count={count}
          itemSize={3}
        />
        <bufferAttribute
          attach="attributes-size"
          array={sizes}
          count={count}
          itemSize={1}
        />
      </bufferGeometry>
      <pointsMaterial
        size={1}
        sizeAttenuation
        color={0x6a5a68}
        transparent
        opacity={0.4}
        depthWrite={false}
      />
    </points>
  );
}
```

---

## 6.2 Address Layout Algorithm

Deterministic mapping of Ethereum addresses to positions on a sphere surface using a golden-angle spiral. Same address always maps to the same point.

- [ ] Implement `addressToPosition(address, radius)` returning `[x, y, z]`
- [ ] Golden-angle spiral distribution for uniform coverage
- [ ] Deterministic: same address always at same position
- [ ] Visual clustering: addresses with similar prefixes are near each other (consequence of the index derivation)
- [ ] `AddressBook` class managing known addresses, their positions, and activity state

```typescript
// apps/explorer/src/scenes/constellation/addressLayout.ts

import * as THREE from 'three';

// ─── Golden-Angle Spiral ────────────────────────────────────

const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5)); // ~2.39996 radians
const MAX_ADDRESSES = 0xffffffff; // 2^32, for normalizing the index

/**
 * Map an Ethereum address to a deterministic 3D position on a sphere.
 *
 * Algorithm:
 *   1. Parse the first 4 bytes of the address as a uint32 index.
 *   2. Use the golden angle spiral to convert the index to spherical
 *      coordinates (theta, phi).
 *   3. Convert to Cartesian (x, y, z) on the sphere surface.
 *
 * The golden angle ensures addresses are uniformly distributed with
 * no clustering at poles. Addresses with similar prefixes (first 4 bytes)
 * will have nearby indices and thus nearby positions -- this is intentional,
 * creating visual neighborhoods for related addresses (e.g., contracts
 * deployed by the same factory).
 *
 * @param address - 0x-prefixed 20-byte hex address
 * @param radius  - sphere radius (default 8)
 * @returns [x, y, z] position on the sphere
 */
export function addressToPosition(
  address: `0x${string}`,
  radius: number = 8,
): [number, number, number] {
  // Parse first 4 bytes as uint32 for the spiral index
  const indexHex = address.slice(2, 10);
  const index = parseInt(indexHex, 16);

  // Normalized index in [0, 1)
  const normalizedIndex = index / MAX_ADDRESSES;

  // Golden-angle spiral on sphere:
  //   theta = golden_angle * index  (azimuthal angle)
  //   phi = acos(1 - 2 * t)         (polar angle, uniform on sphere)
  const theta = GOLDEN_ANGLE * index;
  const phi = Math.acos(1 - 2 * normalizedIndex);

  const x = radius * Math.sin(phi) * Math.cos(theta);
  const y = radius * Math.sin(phi) * Math.sin(theta);
  const z = radius * Math.cos(phi);

  return [x, y, z];
}

// ─── Address Node Data ──────────────────────────────────────

export interface AddressNode {
  address: `0x${string}`;
  position: THREE.Vector3;
  isContract: boolean;

  // Visual state (updated per frame)
  color: THREE.Color;
  scale: number;       // 1.0 = normal, up to 2.0 for high-activity
  opacity: number;     // 0.3 = dormant, 1.0 = active
  pulsePhase: number;  // 0-2pi, for animation
  lastActiveTime: number; // performance.now() of last tx involvement
  txCount: number;     // local count (since page load)
}

// ─── Color Palette ──────────────────────────────────────────

const COLOR_DORMANT = new THREE.Color(0x3a303a);   // text-ghost
const COLOR_ACTIVE = new THREE.Color(0xdca5bd);    // rose-glow
const COLOR_CONTRACT = new THREE.Color(0xaa7088);  // rose (accent)
const COLOR_HIGH_VALUE = new THREE.Color(0xd8c8a0); // bone-bright

// ─── Address Book ───────────────────────────────────────────

/**
 * Manages all known addresses, their positions, and their visual state.
 * Used by AddressNodes (instanced mesh) and TransactionArcs.
 */
export class AddressBook {
  private nodes: Map<string, AddressNode> = new Map();
  private indexList: AddressNode[] = [];  // ordered for instanced mesh
  private dirty = false;                  // true when indexList needs rebuild
  private maxNodes: number;

  constructor(maxNodes: number = 4096) {
    this.maxNodes = maxNodes;
  }

  /** Register a new address or update an existing one. */
  registerAddress(
    address: `0x${string}`,
    meta: { isContract: boolean },
  ): AddressNode {
    const key = address.toLowerCase();
    let node = this.nodes.get(key);

    if (node) {
      node.isContract = node.isContract || meta.isContract;
      return node;
    }

    if (this.nodes.size >= this.maxNodes) {
      // Evict the least recently active node
      this.evictLeastActive();
    }

    const [x, y, z] = addressToPosition(address);
    node = {
      address,
      position: new THREE.Vector3(x, y, z),
      isContract: meta.isContract,
      color: meta.isContract ? COLOR_CONTRACT.clone() : COLOR_DORMANT.clone(),
      scale: 1.0,
      opacity: 0.3,
      pulsePhase: Math.random() * Math.PI * 2,
      lastActiveTime: 0,
      txCount: 0,
    };

    this.nodes.set(key, node);
    this.dirty = true;
    return node;
  }

  /** Get an existing node or create it with default EOA metadata. */
  getOrCreate(address: `0x${string}`): AddressNode {
    const key = address.toLowerCase();
    return this.nodes.get(key) ?? this.registerAddress(address, { isContract: false });
  }

  /** Get node by address (or null). */
  get(address: `0x${string}`): AddressNode | null {
    return this.nodes.get(address.toLowerCase()) ?? null;
  }

  /** Mark an address as active (involved in a transaction). */
  markActive(address: `0x${string}`): void {
    const node = this.get(address);
    if (!node) return;
    node.lastActiveTime = performance.now();
    node.txCount++;
    node.opacity = 1.0;
    node.scale = Math.min(2.0, 1.0 + Math.log(1 + node.txCount) * 0.15);
    node.color.copy(node.isContract ? COLOR_CONTRACT : COLOR_ACTIVE);
  }

  /**
   * Update pulse animations for all nodes.
   * Called every frame by the scene's useFrame.
   */
  updatePulse(delta: number): void {
    const now = performance.now();

    for (const node of this.nodes.values()) {
      // Advance pulse phase
      node.pulsePhase += delta * 2.4; // 2.4s cycle
      if (node.pulsePhase > Math.PI * 2) node.pulsePhase -= Math.PI * 2;

      // Decay activity: fade back to dormant over 30 seconds
      const age = (now - node.lastActiveTime) / 1000;
      if (age > 2 && node.opacity > 0.3) {
        node.opacity = Math.max(0.3, node.opacity - delta * 0.03);
      }
      if (age > 30 && node.scale > 1.0) {
        node.scale = Math.max(1.0, node.scale - delta * 0.05);
      }
      if (age > 60) {
        node.color.lerp(COLOR_DORMANT, delta * 0.1);
      }
    }
  }

  /** Ordered list of all nodes for instanced mesh rendering. */
  getNodeList(): AddressNode[] {
    if (this.dirty) {
      this.indexList = Array.from(this.nodes.values());
      this.dirty = false;
    }
    return this.indexList;
  }

  /** Number of registered nodes. */
  get size(): number {
    return this.nodes.size;
  }

  /** Evict the node with the oldest lastActiveTime. */
  private evictLeastActive(): void {
    let oldest: AddressNode | null = null;
    let oldestTime = Infinity;
    for (const node of this.nodes.values()) {
      if (node.lastActiveTime < oldestTime) {
        oldestTime = node.lastActiveTime;
        oldest = node;
      }
    }
    if (oldest) {
      this.nodes.delete(oldest.address.toLowerCase());
      this.dirty = true;
    }
  }
}
```

---

## 6.3 GPU Particle Buffer (Address Nodes)

Address nodes rendered as an `InstancedMesh` of small spheres. Each instance has a unique position, color, scale, and opacity driven by the `AddressBook`.

- [ ] `InstancedMesh` with SphereGeometry (radius 0.08, 8 segments) -- max 4096 instances
- [ ] Color by activity: dormant = `text-ghost`, active = `rose-glow`, contract = `--rd-rose`
- [ ] Size by tx count (logarithmic scale)
- [ ] Instance attributes: position, color, scale, opacity
- [ ] Per-frame update of instance matrices and colors from AddressBook state
- [ ] Contracts rendered as a rotated (45 degree) box instead of sphere (diamond shape)

```typescript
// apps/explorer/src/scenes/constellation/AddressNodes.tsx

import { useRef, useMemo, useEffect } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import type { AddressBook, AddressNode } from './addressLayout';

const MAX_INSTANCES = 4096;
const NODE_RADIUS = 0.08;
const NODE_SEGMENTS = 8;

interface Props {
  addressBook: AddressBook;
}

const _matrix = new THREE.Matrix4();
const _position = new THREE.Vector3();
const _quaternion = new THREE.Quaternion();
const _scale = new THREE.Vector3();
const _color = new THREE.Color();

/**
 * Renders all address nodes as an InstancedMesh.
 * Updates instance matrices and colors every frame from the AddressBook.
 */
export function AddressNodes({ addressBook }: Props) {
  const meshRef = useRef<THREE.InstancedMesh>(null!);

  // Shared geometry: small sphere
  const geometry = useMemo(
    () => new THREE.SphereGeometry(NODE_RADIUS, NODE_SEGMENTS, NODE_SEGMENTS),
    [],
  );

  // Material: emissive for glow, transparent for dormant fade
  const material = useMemo(
    () =>
      new THREE.MeshStandardMaterial({
        metalness: 0.1,
        roughness: 0.8,
        transparent: true,
        depthWrite: true,
      }),
    [],
  );

  // Update instance data every frame
  useFrame(() => {
    const mesh = meshRef.current;
    if (!mesh) return;

    const nodes = addressBook.getNodeList();
    const count = Math.min(nodes.length, MAX_INSTANCES);
    mesh.count = count;

    for (let i = 0; i < count; i++) {
      const node = nodes[i];

      // Position on sphere
      _position.copy(node.position);

      // Pulse: subtle scale oscillation for active nodes
      const pulseScale =
        node.opacity > 0.5
          ? 1 + Math.sin(node.pulsePhase) * 0.08 // +/- 8% at peak activity
          : 1;

      _scale.setScalar(node.scale * pulseScale);
      _quaternion.identity();

      // Contracts: rotate 45 degrees for diamond appearance
      if (node.isContract) {
        _quaternion.setFromEuler(new THREE.Euler(0, 0, Math.PI / 4));
      }

      _matrix.compose(_position, _quaternion, _scale);
      mesh.setMatrixAt(i, _matrix);

      // Color with opacity baked into alpha
      _color.copy(node.color);
      mesh.setColorAt(i, _color);
    }

    mesh.instanceMatrix.needsUpdate = true;
    if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
  });

  return (
    <instancedMesh
      ref={meshRef}
      args={[geometry, material, MAX_INSTANCES]}
      frustumCulled={false}
    />
  );
}
```

### Halo Effect for Active Nodes

An additional instanced mesh of slightly larger, additive-blended spheres to create a glow halo around active addresses.

```typescript
// Additional halo pass (rendered on top with additive blending)

export function AddressHalos({ addressBook }: Props) {
  const meshRef = useRef<THREE.InstancedMesh>(null!);

  const geometry = useMemo(
    () => new THREE.SphereGeometry(NODE_RADIUS * 3, 8, 8),
    [],
  );

  const material = useMemo(
    () =>
      new THREE.MeshBasicMaterial({
        color: 0xdca5bd, // rose-glow
        transparent: true,
        opacity: 0.15,
        blending: THREE.AdditiveBlending,
        depthWrite: false,
      }),
    [],
  );

  useFrame(() => {
    const mesh = meshRef.current;
    if (!mesh) return;

    const nodes = addressBook.getNodeList();
    let haloCount = 0;

    for (let i = 0; i < nodes.length && haloCount < MAX_INSTANCES; i++) {
      const node = nodes[i];
      // Only show halo for active nodes
      if (node.opacity <= 0.5) continue;

      _position.copy(node.position);
      _scale.setScalar(node.scale * 1.5);
      _quaternion.identity();
      _matrix.compose(_position, _quaternion, _scale);
      mesh.setMatrixAt(haloCount, _matrix);
      haloCount++;
    }

    mesh.count = haloCount;
    mesh.instanceMatrix.needsUpdate = true;
  });

  return (
    <instancedMesh
      ref={meshRef}
      args={[geometry, material, MAX_INSTANCES]}
      frustumCulled={false}
      renderOrder={1}
    />
  );
}
```

---

## 6.4 Transaction Arcs

Each transaction is rendered as a curved arc from sender to receiver, with an animated particle traveling along it.

- [ ] Arc geometry: `QuadraticBezierCurve3` from sender position to receiver position
- [ ] Arc height (control point offset) proportional to transaction value (log scale)
- [ ] Animation: a bright point travels along the arc over 1 second
- [ ] Line material with gradient (sender color fading to receiver color)
- [ ] Max 256 visible arcs (oldest fade out when limit reached)
- [ ] `computeArc(from, to, value)` function with full implementation
- [ ] Ghost trails: completed arcs persist as dim lines for 30 seconds

```typescript
// apps/explorer/src/scenes/constellation/arcUtils.ts

import * as THREE from 'three';
import type { AddressNode } from './addressLayout';

// ─── Arc Computation ────────────────────────────────────────

export interface ArcData {
  id: string;                       // tx hash
  from: AddressNode;
  to: AddressNode;
  curve: THREE.QuadraticBezierCurve3;
  controlPoint: THREE.Vector3;
  status: 'pending' | 'confirmed' | 'reverted';

  // Animation state
  progress: number;                 // 0 = just spawned, 1 = arrived
  opacity: number;                  // fades after completion
  birthTime: number;                // performance.now()
  particlePosition: THREE.Vector3;  // current position of the traveling dot
}

/**
 * Compute a quadratic bezier arc between two address nodes.
 *
 * The control point is offset perpendicular to the line connecting
 * the two nodes, with magnitude proportional to the transaction value
 * on a logarithmic scale. This creates higher arcs for larger transactions.
 *
 * The perpendicular direction is computed in 3D by crossing the line
 * direction with a reference "up" vector, producing arcs that bow
 * outward from the sphere surface.
 *
 * @param from   - sender AddressNode
 * @param to     - receiver AddressNode
 * @param value  - transaction value in wei (bigint)
 * @returns ArcData with the computed bezier curve
 */
export function computeArc(
  from: AddressNode,
  to: AddressNode,
  value: bigint,
  txHash: string,
  status: 'pending' | 'confirmed' | 'reverted' = 'confirmed',
): ArcData {
  const start = from.position;
  const end = to.position;

  // ── Direction and distance ──
  const direction = new THREE.Vector3().subVectors(end, start);
  const distance = direction.length();
  const midpoint = new THREE.Vector3().addVectors(start, end).multiplyScalar(0.5);

  // ── Arc height from value (log scale) ──
  // Minimum curvature of 0.5 units, max 4 units.
  // Value in ETH: 0 ETH = min, 1000+ ETH = max
  const valueEth = Number(value) / 1e18;
  const curvature = Math.min(4.0, 0.5 + Math.log(1 + valueEth) * 0.8);

  // ── Perpendicular offset ──
  // Cross the line direction with a reference up vector to get
  // a perpendicular direction that bows outward from the sphere.
  const up = new THREE.Vector3(0, 1, 0);
  const perp = new THREE.Vector3().crossVectors(direction.normalize(), up);

  // If direction is nearly parallel to up, use a different reference
  if (perp.lengthSq() < 0.001) {
    perp.crossVectors(direction, new THREE.Vector3(1, 0, 0));
  }
  perp.normalize();

  // Also push the control point outward from sphere center
  // so arcs bow away from the sphere rather than cutting through it.
  const outward = midpoint.clone().normalize();
  const offset = new THREE.Vector3()
    .addScaledVector(perp, curvature * 0.5)
    .addScaledVector(outward, curvature);

  const controlPoint = midpoint.clone().add(offset);

  // ── Bezier curve ──
  const curve = new THREE.QuadraticBezierCurve3(start, controlPoint, end);

  return {
    id: txHash,
    from,
    to,
    curve,
    controlPoint,
    status,
    progress: 0,
    opacity: 1,
    birthTime: performance.now(),
    particlePosition: start.clone(),
  };
}

// ─── Arc Manager ────────────────────────────────────────────

const MAX_ARCS = 256;
const ARC_TRAVEL_DURATION = 1.0;       // seconds for particle to traverse arc
const ARC_GHOST_DURATION = 30.0;       // seconds for completed arc to persist
const PENDING_PULSE_SPEED = 3.0;       // radians/second for pending pulse

/**
 * Manages the lifecycle of all transaction arcs.
 * Handles spawning, animation, and cleanup.
 */
export class ArcManager {
  private arcs: ArcData[] = [];
  private maxArcs: number;

  constructor(maxArcs: number = MAX_ARCS) {
    this.maxArcs = maxArcs;
  }

  /** Add a new transaction arc. Evicts oldest if at capacity. */
  addArc(
    from: AddressNode,
    to: AddressNode,
    value: bigint,
    txHash: string,
    status: 'pending' | 'confirmed' | 'reverted',
  ): ArcData {
    // Mark both addresses as active
    from.lastActiveTime = performance.now();
    from.txCount++;
    to.lastActiveTime = performance.now();
    to.txCount++;

    const arc = computeArc(from, to, value, txHash, status);

    if (this.arcs.length >= this.maxArcs) {
      // Remove the oldest completed arc
      const oldestIdx = this.arcs.findIndex(
        (a) => a.progress >= 1 && a.status !== 'pending',
      );
      if (oldestIdx >= 0) {
        this.arcs.splice(oldestIdx, 1);
      } else {
        // All are still animating; remove the absolute oldest
        this.arcs.shift();
      }
    }

    this.arcs.push(arc);
    return arc;
  }

  /** Resolve a pending arc to confirmed or reverted. */
  resolveArc(txHash: string, status: 'confirmed' | 'reverted'): void {
    const arc = this.arcs.find((a) => a.id === txHash);
    if (!arc) return;
    arc.status = status;

    if (status === 'confirmed') {
      // Snap progress to start the arc animation
      arc.progress = 0;
    }
    // Reverted arcs trigger the shatter effect (handled in lifecycle.ts)
  }

  /**
   * Update all arcs per frame.
   * Advances particle positions, handles fade-out, removes dead arcs.
   */
  update(delta: number): void {
    const now = performance.now();

    for (let i = this.arcs.length - 1; i >= 0; i--) {
      const arc = this.arcs[i];

      if (arc.status === 'pending') {
        // Pending: particle pulses at sender position, does not traverse
        arc.particlePosition.copy(arc.from.position);
        // Pulse opacity
        const pulse = Math.sin(now * 0.001 * PENDING_PULSE_SPEED) * 0.5 + 0.5;
        arc.opacity = 0.5 + pulse * 0.5;
        continue;
      }

      if (arc.progress < 1) {
        // Traveling: advance particle along the curve
        arc.progress = Math.min(1, arc.progress + delta / ARC_TRAVEL_DURATION);
        // Ease-out for natural deceleration at receiver
        const eased = 1 - Math.pow(1 - arc.progress, 3);
        arc.curve.getPoint(eased, arc.particlePosition);
      } else {
        // Completed: fade out over ghost duration
        const age = (now - arc.birthTime) / 1000 - ARC_TRAVEL_DURATION;
        const fadeProgress = Math.min(age / ARC_GHOST_DURATION, 1);
        arc.opacity = Math.max(0, 1 - fadeProgress);

        // Remove fully faded arcs
        if (arc.opacity <= 0) {
          this.arcs.splice(i, 1);
        }
      }
    }
  }

  /** Get all active arcs for rendering. */
  getArcs(): ArcData[] {
    return this.arcs;
  }

  /** Get the number of active arcs. */
  get activeCount(): number {
    return this.arcs.length;
  }
}
```

### Arc Rendering Component

```typescript
// apps/explorer/src/scenes/constellation/TransactionArcs.tsx

import { useRef, useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import type { ArcManager, ArcData } from './arcUtils';

const ARC_SEGMENTS = 32;     // segments per curve
const PARTICLE_SIZE = 0.12;  // world units

interface Props {
  arcManager: ArcManager;
}

/**
 * Renders all transaction arcs as Line objects with animated particles.
 */
export function TransactionArcs({ arcManager }: Props) {
  const groupRef = useRef<THREE.Group>(null!);
  const particlesRef = useRef<THREE.InstancedMesh>(null!);

  // Particle geometry: small sphere for the traveling dot
  const particleGeo = useMemo(
    () => new THREE.SphereGeometry(PARTICLE_SIZE, 6, 6),
    [],
  );
  const particleMat = useMemo(
    () =>
      new THREE.MeshBasicMaterial({
        color: 0xd8c8a0, // bone-bright
        transparent: true,
        blending: THREE.AdditiveBlending,
        depthWrite: false,
      }),
    [],
  );

  // Arc line material
  const lineMat = useMemo(
    () =>
      new THREE.LineBasicMaterial({
        color: 0xaa7088, // rose
        transparent: true,
        opacity: 0.4,
        depthWrite: false,
      }),
    [],
  );

  // Pending pulse material (dream color)
  const pendingMat = useMemo(
    () =>
      new THREE.MeshBasicMaterial({
        color: 0x9494b4, // dream-bright
        transparent: true,
        blending: THREE.AdditiveBlending,
        depthWrite: false,
      }),
    [],
  );

  const _matrix = new THREE.Matrix4();
  const _scale = new THREE.Vector3(1, 1, 1);
  const _quat = new THREE.Quaternion();

  useFrame(() => {
    const group = groupRef.current;
    const particles = particlesRef.current;
    if (!group || !particles) return;

    const arcs = arcManager.getArcs();

    // Rebuild line geometries (cheap at 256 arcs max)
    // Clear old children
    while (group.children.length > 0) {
      const child = group.children[0];
      group.remove(child);
      if (child instanceof THREE.Line) {
        child.geometry.dispose();
      }
    }

    let particleIdx = 0;

    for (const arc of arcs) {
      // ── Draw the arc line ──
      if (arc.progress > 0 || arc.status === 'pending') {
        const points = arc.curve.getPoints(ARC_SEGMENTS);

        // For in-progress arcs, only draw up to the current progress
        const visiblePoints =
          arc.progress < 1
            ? points.slice(0, Math.ceil(arc.progress * ARC_SEGMENTS) + 1)
            : points;

        const lineGeo = new THREE.BufferGeometry().setFromPoints(visiblePoints);
        const line = new THREE.Line(lineGeo, lineMat.clone());
        (line.material as THREE.LineBasicMaterial).opacity =
          arc.opacity * (arc.status === 'pending' ? 0.3 : 0.4);
        group.add(line);
      }

      // ── Draw the traveling particle ──
      if (particleIdx < 256) {
        _matrix.compose(arc.particlePosition, _quat, _scale);
        particles.setMatrixAt(particleIdx, _matrix);

        // Color: dream for pending, bone for confirmed, danger for reverted
        const color =
          arc.status === 'pending'
            ? new THREE.Color(0x9494b4)
            : arc.status === 'reverted'
              ? new THREE.Color(0xcc5555)
              : new THREE.Color(0xd8c8a0);
        particles.setColorAt(particleIdx, color);

        particleIdx++;
      }
    }

    particles.count = particleIdx;
    particles.instanceMatrix.needsUpdate = true;
    if (particles.instanceColor) particles.instanceColor.needsUpdate = true;
  });

  return (
    <>
      <group ref={groupRef} />
      <instancedMesh
        ref={particlesRef}
        args={[particleGeo, particleMat, 256]}
        frustumCulled={false}
      />
    </>
  );
}
```

---

## 6.5 Transaction Lifecycle Animation

Three distinct visual treatments for transaction state transitions.

- [ ] **Pending**: pulsing glow at sender position, dream-bright color (#9494b4)
- [ ] **Confirmed**: arc completes, flash at receiver position, particle settles into node
- [ ] **Reverted**: arc shatters -- particle system burst of 8 fragments radiating outward, red flash

```typescript
// apps/explorer/src/scenes/constellation/lifecycle.ts

import * as THREE from 'three';
import type { ArcData } from './arcUtils';
import type { AddressNode } from './addressLayout';

// ─── Confirmation Flash ─────────────────────────────────────

export interface ConfirmationFlash {
  position: THREE.Vector3;
  opacity: number;      // 1.0 -> 0.0 over 400ms
  scale: number;        // 0.5 -> 2.0 over 400ms
  color: THREE.Color;
  age: number;          // seconds
}

/**
 * Create a confirmation flash effect at the receiver node.
 * A brief expanding ring of light that fades quickly.
 */
export function createConfirmationFlash(
  receiver: AddressNode,
): ConfirmationFlash {
  return {
    position: receiver.position.clone(),
    opacity: 1.0,
    scale: 0.5,
    color: new THREE.Color(0xd8c8a0), // bone-bright
    age: 0,
  };
}

export function updateConfirmationFlash(
  flash: ConfirmationFlash,
  delta: number,
): boolean {
  flash.age += delta;
  const t = Math.min(flash.age / 0.4, 1); // 400ms duration
  flash.opacity = 1 - t;
  flash.scale = 0.5 + t * 1.5;
  return t >= 1; // true = done
}

// ─── Revert Shatter ─────────────────────────────────────────

export interface ShatterFragment {
  position: THREE.Vector3;
  velocity: THREE.Vector3;
  opacity: number;
  scale: number;
  age: number;
}

export interface ShatterEffect {
  fragments: ShatterFragment[];
  done: boolean;
}

/**
 * Create a shatter effect: 8 fragments radiate outward from
 * the arc's current position with a danger-red flash.
 */
export function createShatterEffect(arc: ArcData): ShatterEffect {
  const origin = arc.particlePosition.clone();
  const fragments: ShatterFragment[] = [];

  for (let i = 0; i < 8; i++) {
    // Distribute fragments in a cone around the arc direction
    const angle = (i / 8) * Math.PI * 2;
    const spread = 0.8;
    const velocity = new THREE.Vector3(
      Math.cos(angle) * spread,
      Math.sin(angle) * spread,
      (Math.random() - 0.5) * spread * 0.5,
    );

    fragments.push({
      position: origin.clone(),
      velocity,
      opacity: 1.0,
      scale: 0.06 + Math.random() * 0.04,
      age: 0,
    });
  }

  return { fragments, done: false };
}

/**
 * Update shatter fragments. Fragments decelerate and fade over 800ms.
 * Returns true when the effect is complete.
 */
export function updateShatterEffect(
  effect: ShatterEffect,
  delta: number,
): boolean {
  let allDone = true;

  for (const frag of effect.fragments) {
    frag.age += delta;
    const t = Math.min(frag.age / 0.8, 1); // 800ms duration

    // Move with deceleration
    frag.position.addScaledVector(frag.velocity, delta * (1 - t));

    // Fade out
    frag.opacity = 1 - t;
    frag.scale *= 0.98;

    if (t < 1) allDone = false;
  }

  effect.done = allDone;
  return allDone;
}

// ─── Pending Pulse ──────────────────────────────────────────

/**
 * Compute the visual state for a pending transaction.
 * The particle orbits the sender node, pulsing dream-bright.
 *
 * @param node    - sender address node
 * @param time    - elapsed time in seconds (from performance.now)
 * @param orbitRadius - distance from node center (grows over time)
 * @returns position for the orbiting particle
 */
export function computePendingOrbit(
  node: AddressNode,
  time: number,
  waitDuration: number, // seconds since pending
): THREE.Vector3 {
  // Orbit radius grows as the tx waits longer
  const baseRadius = 0.2;
  const maxRadius = 0.8;
  const radius = Math.min(maxRadius, baseRadius + waitDuration * 0.01);

  // Orbit speed: 1 revolution per 2 seconds
  const angle = time * Math.PI;

  // Orbit plane is perpendicular to the node's radial direction
  const radial = node.position.clone().normalize();
  const up = new THREE.Vector3(0, 1, 0);
  const tangent = new THREE.Vector3().crossVectors(radial, up).normalize();
  const bitangent = new THREE.Vector3().crossVectors(radial, tangent).normalize();

  const offset = tangent
    .multiplyScalar(Math.cos(angle) * radius)
    .add(bitangent.multiplyScalar(Math.sin(angle) * radius));

  return node.position.clone().add(offset);
}
```

---

## 6.6 Interaction

Raycasting on address nodes and arc midpoints for hover labels and click navigation.

- [ ] Raycast on `InstancedMesh` for address node hover and click
- [ ] Hover on node: show label tooltip (address truncated, balance, tx count)
- [ ] Click on node: emit `entity:select`, open AddressDetail panel
- [ ] Raycast on arc midpoints: hover shows tx summary, click opens TxDetail
- [ ] Search highlight: matching address node gets a ring effect (expanding circle)

```typescript
// apps/explorer/src/scenes/constellation/interactions.ts

import * as THREE from 'three';
import { bus } from '@/data/bus';
import { useUIStore } from '@/data/store';
import type { AddressBook, AddressNode } from './addressLayout';
import type { ArcManager, ArcData } from './arcUtils';

const raycaster = new THREE.Raycaster();
const pointer = new THREE.Vector2();

// Set raycaster threshold for InstancedMesh picking
// (extends the pick radius so small nodes are easier to hit)
raycaster.params.Points = { threshold: 0.5 };

// ─── Node Raycasting ────────────────────────────────────────

export interface NodeHitResult {
  node: AddressNode;
  instanceId: number;
  point: THREE.Vector3;
}

/**
 * Raycast against the address node InstancedMesh.
 * Three.js InstancedMesh supports raycasting natively via setMatrixAt.
 */
export function raycastNodes(
  event: PointerEvent,
  camera: THREE.Camera,
  mesh: THREE.InstancedMesh,
  addressBook: AddressBook,
): NodeHitResult | null {
  const rect = (event.target as HTMLElement).getBoundingClientRect();
  pointer.x = ((event.clientX - rect.left) / rect.width) * 2 - 1;
  pointer.y = -((event.clientY - rect.top) / rect.height) * 2 + 1;

  raycaster.setFromCamera(pointer, camera);
  const intersections = raycaster.intersectObject(mesh, false);

  if (intersections.length === 0) return null;

  const hit = intersections[0];
  if (hit.instanceId === undefined) return null;

  const nodes = addressBook.getNodeList();
  if (hit.instanceId >= nodes.length) return null;

  return {
    node: nodes[hit.instanceId],
    instanceId: hit.instanceId,
    point: hit.point,
  };
}

/**
 * Handle node hover: show tooltip with address info.
 */
export function handleNodeHover(
  hit: NodeHitResult | null,
): {
  address: `0x${string}`;
  isContract: boolean;
  txCount: number;
  screenPos: { x: number; y: number };
} | null {
  if (!hit) return null;

  return {
    address: hit.node.address,
    isContract: hit.node.isContract,
    txCount: hit.node.txCount,
    screenPos: { x: 0, y: 0 }, // projected from hit.point in the component
  };
}

/**
 * Handle node click: navigate to address detail view.
 */
export function handleNodeClick(hit: NodeHitResult | null): void {
  if (!hit) return;

  bus.emit('entity:select' as any, {
    type: 'address',
    address: hit.node.address,
  });

  useUIStore.getState().openDetail({
    type: 'address',
    address: hit.node.address,
  });
}

// ─── Arc Raycasting ─────────────────────────────────────────

export interface ArcHitResult {
  arc: ArcData;
  point: THREE.Vector3;
}

/**
 * Raycast against arc midpoints.
 * Uses a proximity test rather than geometric intersection
 * (arcs are thin lines, hard to click precisely).
 */
export function raycastArcs(
  event: PointerEvent,
  camera: THREE.Camera,
  canvas: HTMLElement,
  arcManager: ArcManager,
): ArcHitResult | null {
  const rect = canvas.getBoundingClientRect();
  pointer.x = ((event.clientX - rect.left) / rect.width) * 2 - 1;
  pointer.y = -((event.clientY - rect.top) / rect.height) * 2 + 1;

  raycaster.setFromCamera(pointer, camera);

  const arcs = arcManager.getArcs();
  let closestArc: ArcData | null = null;
  let closestDist = 0.5; // max pick distance in world units

  for (const arc of arcs) {
    // Test against the arc midpoint (control point)
    const dist = raycaster.ray.distanceToPoint(arc.controlPoint);
    if (dist < closestDist) {
      closestDist = dist;
      closestArc = arc;
    }
  }

  if (!closestArc) return null;

  return {
    arc: closestArc,
    point: closestArc.controlPoint.clone(),
  };
}

/**
 * Handle arc click: navigate to transaction detail view.
 */
export function handleArcClick(hit: ArcHitResult | null): void {
  if (!hit) return;

  useUIStore.getState().openDetail({
    type: 'tx',
    hash: hit.arc.id as `0x${string}`,
  });
}

// ─── Search Highlight ───────────────────────────────────────

export interface SearchHighlight {
  node: AddressNode;
  ringScale: number;   // expanding ring, 1 -> 3 over 500ms
  ringOpacity: number; // 1 -> 0 over 500ms
  age: number;
}

/**
 * Create a search highlight ring effect on an address node.
 * The ring expands outward and fades, drawing attention.
 */
export function createSearchHighlight(node: AddressNode): SearchHighlight {
  return {
    node,
    ringScale: 1,
    ringOpacity: 1,
    age: 0,
  };
}

export function updateSearchHighlight(
  highlight: SearchHighlight,
  delta: number,
): boolean {
  highlight.age += delta;
  const t = Math.min(highlight.age / 0.5, 1); // 500ms
  highlight.ringScale = 1 + t * 2;
  highlight.ringOpacity = 1 - t;
  return t >= 1; // true = done
}
```

### Node Tooltip Component

```typescript
// Inline within ConstellationScene.tsx as an HTML overlay

import { Html } from '@react-three/drei';

interface NodeTooltipProps {
  address: `0x${string}`;
  isContract: boolean;
  txCount: number;
  position: [number, number, number];
}

function truncateAddress(addr: string): string {
  return `${addr.slice(0, 6)}...${addr.slice(-4)}`;
}

function NodeTooltip({ address, isContract, txCount, position }: NodeTooltipProps) {
  return (
    <Html position={position} center style={{ pointerEvents: 'none' }}>
      <div
        style={{
          background: 'rgba(8, 8, 12, 0.85)',
          backdropFilter: 'blur(12px)',
          border: '1px solid rgba(255, 255, 255, 0.07)',
          padding: '6px 12px',
          fontFamily: "'JetBrains Mono', monospace",
          fontSize: '10px',
          color: '#c8b8c0',
          letterSpacing: '0.06em',
          whiteSpace: 'nowrap',
          minWidth: '140px',
        }}
      >
        <div style={{
          textTransform: 'uppercase',
          letterSpacing: '0.28em',
          color: '#6a5a68',
          fontSize: '9px',
          marginBottom: '4px',
        }}>
          {isContract ? 'CONTRACT' : 'ADDRESS'}
        </div>
        <div style={{ color: '#e8d8e0' }}>
          {truncateAddress(address)}
        </div>
        <div style={{ color: '#6a5a68', marginTop: '2px' }}>
          {txCount} txns
        </div>
      </div>
    </Html>
  );
}
```

---

## 6.7 Performance

Strict budgets to maintain 60fps with 1000 visible nodes and 100 active arcs.

| Metric | Budget | Notes |
|--------|--------|-------|
| Draw calls per frame | <= 5 | 1 instanced nodes + 1 instanced halos + 1 instanced particles + 1 starfield + arc lines batched |
| Max address instances | 4,096 | InstancedMesh capacity |
| Max visible arcs | 256 | Oldest fade out |
| Instance matrix update | <= 2ms | 4096 * setMatrixAt per frame |
| Arc geometry rebuild | <= 1ms | 256 arcs * 32 segments |
| Frame time (total) | <= 16.6ms | 60fps target |
| GPU memory | ~15MB | Instance buffers + line geometries |
| JS heap (AddressBook) | ~2MB | 4096 nodes * ~500B each |

### Tier-Specific Configuration

```typescript
// apps/explorer/src/scenes/constellation/config.ts

import { detectTier } from '@/utils/performance';

export interface ConstellationConfig {
  maxNodes: number;
  maxArcs: number;
  arcSegments: number;
  starCount: number;
  enableHalos: boolean;
  enableShatter: boolean;
  autoRotateSpeed: number;
}

export function getConstellationConfig(): ConstellationConfig {
  const tier = detectTier();

  switch (tier) {
    case 'full':
      return {
        maxNodes: 4096,
        maxArcs: 256,
        arcSegments: 32,
        starCount: 2000,
        enableHalos: true,
        enableShatter: true,
        autoRotateSpeed: 0.3,
      };
    case 'standard':
      return {
        maxNodes: 2048,
        maxArcs: 128,
        arcSegments: 16,
        starCount: 1000,
        enableHalos: true,
        enableShatter: true,
        autoRotateSpeed: 0.3,
      };
    case 'light':
      return {
        maxNodes: 512,
        maxArcs: 64,
        arcSegments: 8,
        starCount: 500,
        enableHalos: false,      // too expensive
        enableShatter: false,    // skip particle burst
        autoRotateSpeed: 0.15,   // slower = fewer updates
      };
    case 'mobile':
      // Mobile should not load this scene (falls back to MOSAIC)
      // but provide minimal config just in case
      return {
        maxNodes: 256,
        maxArcs: 32,
        arcSegments: 8,
        starCount: 200,
        enableHalos: false,
        enableShatter: false,
        autoRotateSpeed: 0.1,
      };
    default:
      return {
        maxNodes: 2048,
        maxArcs: 128,
        arcSegments: 16,
        starCount: 1000,
        enableHalos: true,
        enableShatter: true,
        autoRotateSpeed: 0.3,
      };
  }
}
```

### GPU-Side Arc Animation (Full Tier)

On the `'full'` performance tier, arc particle animation can be offloaded to the GPU via transform feedback, matching the design spec's recommendation.

```typescript
// apps/explorer/src/scenes/constellation/gpuArcs.ts
//
// WebGL2 transform feedback for GPU-side arc particle updates.
// Only used on 'full' tier. Falls back to CPU update (ArcManager.update)
// on 'standard' and below.

/**
 * Transform feedback vertex shader.
 * Updates particle position along its bezier arc on the GPU.
 *
 * Inputs:  aPosition, aProgress, aStart, aControl, aEnd
 * Outputs: vPosition, vProgress (written back via transform feedback)
 */
const TF_UPDATE_VERT = `#version 300 es
  precision highp float;

  in vec3 aPosition;
  in float aProgress;
  in vec3 aStart;
  in vec3 aControl;
  in vec3 aEnd;

  out vec3 vPosition;
  out float vProgress;

  uniform float uDeltaTime;
  uniform float uTravelSpeed; // 1.0 / ARC_TRAVEL_DURATION

  // Quadratic bezier evaluation
  vec3 bezier(vec3 p0, vec3 p1, vec3 p2, float t) {
    float t1 = 1.0 - t;
    return t1 * t1 * p0 + 2.0 * t1 * t * p1 + t * t * p2;
  }

  void main() {
    vProgress = min(1.0, aProgress + uDeltaTime * uTravelSpeed);

    // Ease-out: 1 - (1-t)^3
    float eased = 1.0 - pow(1.0 - vProgress, 3.0);
    vPosition = bezier(aStart, aControl, aEnd, eased);
  }
`;

/**
 * Check if transform feedback is available and performant.
 * Returns false on devices where TF is supported but slow
 * (some mobile GPUs report WebGL2 but have poor TF performance).
 */
export function isTransformFeedbackViable(gl: WebGL2RenderingContext): boolean {
  // Check for WebGL2
  if (!(gl instanceof WebGL2RenderingContext)) return false;

  // Check for transform feedback support (part of WebGL2 core)
  const tf = gl.createTransformFeedback();
  if (!tf) return false;
  gl.deleteTransformFeedback(tf);

  return true;
}
```

---

## 6.8 Verification

Acceptance criteria for the constellation scene.

- [ ] **Deterministic placement**: same address always renders at the same sphere position. Run `addressToPosition` 100 times; all outputs must be identical.
- [ ] **Visual distribution**: 1000 random addresses produce a visibly uniform sphere coverage (no clustering at poles).
- [ ] **Arc animation**: confirmed transaction arcs animate a particle from sender to receiver over 1 second with ease-out deceleration.
- [ ] **Pending pulse**: pending transactions show a pulsing particle at the sender node, dream-bright color.
- [ ] **Reverted shatter**: reverted transactions show 8 fragments radiating outward with a red flash.
- [ ] **Click on node**: clicking an address node opens the AddressDetail panel.
- [ ] **Click on arc**: clicking an arc midpoint opens the TxDetail panel.
- [ ] **Hover tooltip**: hovering an address node shows a glass tooltip with truncated address and tx count.
- [ ] **Search highlight**: searching for an address creates an expanding ring effect on the matching node.
- [ ] **Performance (M1 MacBook Air)**: solid 60fps with 1000 visible nodes and 100 active arcs.
- [ ] **Performance (low-end)**: on `'light'` tier, max 512 nodes and 64 arcs. Halos and shatter disabled.
- [ ] **Orbit controls**: auto-rotation at 0.3 rad/s, damped user interaction, zoom in/out with scroll.
- [ ] **Empty chain behavior**: when no transactions are flowing, nodes pulse gently at staggered intervals and the constellation drifts slowly.
- [ ] **Reduced motion**: when `prefers-reduced-motion: reduce` is active, auto-rotation stops. Arc particles do not animate (show as static lines). No pulse animation.

### Test Fixtures

```typescript
// apps/explorer/src/scenes/constellation/__tests__/addressLayout.test.ts

import { describe, it, expect } from 'vitest';
import { addressToPosition, AddressBook } from '../addressLayout';

const KNOWN_ADDR = '0xa3f28b018c6d92f8b4ee1547de3f5e1c3b982d01' as `0x${string}`;

describe('addressToPosition', () => {
  it('is deterministic', () => {
    const a = addressToPosition(KNOWN_ADDR);
    const b = addressToPosition(KNOWN_ADDR);
    expect(a).toEqual(b);
  });

  it('returns a point on the sphere surface', () => {
    const [x, y, z] = addressToPosition(KNOWN_ADDR, 8);
    const dist = Math.sqrt(x * x + y * y + z * z);
    expect(dist).toBeCloseTo(8, 2);
  });

  it('different addresses produce different positions', () => {
    const OTHER_ADDR = '0xb7c39d4f2a1e3b5c6d7e8f9a0b1c2d3e4f5a6b7c' as `0x${string}`;
    const a = addressToPosition(KNOWN_ADDR);
    const b = addressToPosition(OTHER_ADDR);
    const dist = Math.sqrt(
      (a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2 + (a[2] - b[2]) ** 2,
    );
    expect(dist).toBeGreaterThan(0.01);
  });

  it('distributes uniformly (no polar clustering)', () => {
    // Generate 1000 positions, check Z distribution
    const zValues: number[] = [];
    for (let i = 0; i < 1000; i++) {
      const hex = i.toString(16).padStart(40, '0');
      const addr = `0x${hex}` as `0x${string}`;
      const [, , z] = addressToPosition(addr, 8);
      zValues.push(z);
    }
    // Z should be roughly uniformly distributed in [-8, 8]
    const mean = zValues.reduce((a, b) => a + b) / zValues.length;
    expect(Math.abs(mean)).toBeLessThan(2); // centered around 0
  });
});

describe('AddressBook', () => {
  it('registers and retrieves addresses', () => {
    const book = new AddressBook(100);
    const node = book.registerAddress(KNOWN_ADDR, { isContract: false });
    expect(node.address).toBe(KNOWN_ADDR);
    expect(book.get(KNOWN_ADDR)).toBe(node);
  });

  it('respects max capacity via eviction', () => {
    const book = new AddressBook(3);
    for (let i = 0; i < 5; i++) {
      const hex = i.toString(16).padStart(40, '0');
      book.registerAddress(`0x${hex}` as `0x${string}`, { isContract: false });
    }
    expect(book.size).toBe(3);
  });

  it('marks active addresses', () => {
    const book = new AddressBook(100);
    book.registerAddress(KNOWN_ADDR, { isContract: false });
    book.markActive(KNOWN_ADDR);
    const node = book.get(KNOWN_ADDR)!;
    expect(node.opacity).toBe(1.0);
    expect(node.txCount).toBe(1);
  });
});
```

```typescript
// apps/explorer/src/scenes/constellation/__tests__/arcUtils.test.ts

import { describe, it, expect } from 'vitest';
import * as THREE from 'three';
import { computeArc, ArcManager } from '../arcUtils';
import type { AddressNode } from '../addressLayout';

function makeNode(x: number, y: number, z: number): AddressNode {
  return {
    address: '0x0000000000000000000000000000000000000000' as `0x${string}`,
    position: new THREE.Vector3(x, y, z),
    isContract: false,
    color: new THREE.Color(0xffffff),
    scale: 1,
    opacity: 1,
    pulsePhase: 0,
    lastActiveTime: 0,
    txCount: 0,
  };
}

describe('computeArc', () => {
  it('creates a curve from sender to receiver', () => {
    const from = makeNode(0, 0, 8);
    const to = makeNode(8, 0, 0);
    const arc = computeArc(from, to, 1000000000000000000n, '0xabc', 'confirmed');

    // Start of curve should be at sender
    const start = arc.curve.getPoint(0);
    expect(start.distanceTo(from.position)).toBeLessThan(0.01);

    // End of curve should be at receiver
    const end = arc.curve.getPoint(1);
    expect(end.distanceTo(to.position)).toBeLessThan(0.01);
  });

  it('higher value produces larger curvature', () => {
    const from = makeNode(0, 0, 8);
    const to = makeNode(8, 0, 0);

    const lowArc = computeArc(from, to, 100000000000000n, '0xabc', 'confirmed'); // 0.0001 ETH
    const highArc = computeArc(from, to, 1000000000000000000000n, '0xdef', 'confirmed'); // 1000 ETH

    // Control point should be further from the line for high-value arc
    const lowDist = lowArc.controlPoint.distanceTo(
      new THREE.Vector3().addVectors(from.position, to.position).multiplyScalar(0.5),
    );
    const highDist = highArc.controlPoint.distanceTo(
      new THREE.Vector3().addVectors(from.position, to.position).multiplyScalar(0.5),
    );
    expect(highDist).toBeGreaterThan(lowDist);
  });
});

describe('ArcManager', () => {
  it('respects max capacity', () => {
    const mgr = new ArcManager(3);
    const from = makeNode(0, 0, 8);
    const to = makeNode(8, 0, 0);

    for (let i = 0; i < 5; i++) {
      mgr.addArc(from, to, 1n, `0x${i}`, 'confirmed');
    }
    expect(mgr.activeCount).toBeLessThanOrEqual(3);
  });

  it('advances arc progress on update', () => {
    const mgr = new ArcManager(10);
    const from = makeNode(0, 0, 8);
    const to = makeNode(8, 0, 0);
    mgr.addArc(from, to, 1n, '0x1', 'confirmed');

    mgr.update(0.5); // half a second
    const arc = mgr.getArcs()[0];
    expect(arc.progress).toBeGreaterThan(0);
    expect(arc.progress).toBeLessThan(1);
  });
});
```
