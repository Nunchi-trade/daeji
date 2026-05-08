# Scenes — Detailed Visual Specifications

Each scene is a self-contained Three.js or Canvas2D visualization that consumes chain data
and renders it continuously. Scenes compose — multiple can run simultaneously with different
weights/opacities depending on the explorer mode.

---

## Scene 1: Hash Terrain

**The signature visual.** An infinite procedural landscape generated from block hashes.

### Concept

Every block hash is 32 bytes. Those bytes seed a terrain heightmap tile.
As blocks arrive (1/sec), a new tile appends to the right edge of the landscape.
The camera slowly tracks rightward, revealing new terrain as the chain grows.
You are watching the chain literally *build ground beneath itself*.

### Generation Algorithm

```
block.hash (32 bytes) → split into 8 groups of 4 bytes each

bytes[0..4]   → base elevation (0.0 - 1.0)
bytes[4..8]   → roughness / noise frequency
bytes[8..12]  → ridge presence and direction
bytes[12..16] → erosion factor
bytes[16..20] → hue offset from base rose palette
bytes[20..24] → saturation modifier
bytes[24..28] → tile-to-tile blend curve
bytes[28..32] → secondary feature placement (peaks, valleys)
```

Each tile is a 16×16 vertex grid (256 vertices). Heightmap values interpolated with Perlin noise seeded by the hash bytes. Adjacent tiles blend at edges using the `blend curve` bytes for continuity.

### Visual Treatment

- **Base color:** rose-deep (#3a2030) at valleys → rose-glow (#dca5bd) at peaks
- **Empty blocks:** very low relief (flat terrain), ghost opacity, barely visible
- **Blocks with transactions:** dramatic relief, bright peaks, bone-colored highlights at summits
- **Gas usage:** modulates terrain roughness — high gas = jagged, complex; low gas = smooth, gentle
- **State root changes:** when stateRoot differs from parent, the color palette shifts subtly — the "geology" changes

### Lighting

- Single directional light from upper-left (simulating the vignette light source)
- Ambient light: very low, rose-tinted
- Fog: exponential, starting at z=40, color = void (#060608)
- No shadows (too expensive, and fog provides depth cues)

### Camera

- Orthographic or low-FOV perspective (15°) for that flat, tilt-shift look
- Slowly tracks rightward at 1 tile/second (matching block time)
- Pointermove lerp: mouse position gently tilts camera ±5° on both axes
- Smooth lerp factor: 0.04 (glacial response, not twitchy)

### Transaction Markers

When a block contains transactions, vertical beams of light rise from the terrain at that tile:
- One beam per transaction
- Height = gas used (relative to block gas limit)
- Color = bone-bright (value transfer) or rose-bright (contract creation)
- Beam has volumetric glow (additive blended billboard quad)
- Fades over 10 seconds after block arrival

### Three.js Implementation Notes

```javascript
// Terrain as InstancedMesh of planes, or single BufferGeometry
// updated per-block
const geometry = new THREE.PlaneGeometry(1, 1, 15, 15);
// Modify vertex Y positions based on hash-derived heightmap
// Use ShaderMaterial for height-based coloring + fog

const material = new THREE.ShaderMaterial({
  uniforms: {
    uColorLow:  { value: new THREE.Color(0x3a2030) },
    uColorHigh: { value: new THREE.Color(0xdca5bd) },
    uFogColor:  { value: new THREE.Color(0x060608) },
    uFogDensity: { value: 0.025 },
  },
  vertexShader: `/* height-based color + fog */`,
  fragmentShader: `/* lerp colors by height, apply exp fog */`,
  transparent: true,
});
```

Ring buffer of ~200 tiles. Oldest tiles recycled as new blocks arrive. Camera position modulo ring size.

---

## Scene 2: Particle Constellation

**The network visualizer.** Addresses as stars. Transactions as particle streams between them.

### Concept

Every address that has appeared on-chain gets a stable position in 2D space (derived from address hash). The address exists as a softly glowing node. When a transaction connects two addresses, a stream of particles flows from sender to receiver along a curved arc.

### Node Placement

```
address (20 bytes) → deterministic (x, y) position

bytes[0..4]  → angle on a golden-ratio spiral (θ = hash_int * φ)
bytes[4..8]  → radius from center (log scale, so active addresses cluster inward)
bytes[8..12] → z-depth (subtle parallax, ±0.5 units)
```

Addresses with more transactions naturally appear at smaller radii (more active = more central) through a running average that pulls high-activity nodes inward.

### Node Rendering

- **Dormant address:** 2px dot, text-ghost color, no glow. Nearly invisible.
- **Active address (recent txn):** 4-6px dot, rose-glow, subtle pulse animation (2.4s), soft halo
- **Contract:** diamond shape (rotated square), rose-bright
- **High-value address:** larger dot, bone-bright ring, double halo
- **Hovered address:** label appears (JetBrains Mono 10px, address truncated), balance shown

### Transaction Arcs

When a transaction occurs:

1. **Arc path:** Cubic bezier from sender to receiver. Control point offset perpendicular to the line, magnitude proportional to value
2. **Particle stream:** 8-16 particles travel along the arc over 1.5-3 seconds
3. **Particle style:** 2px circles, bone-bright, additive blend, slight size variation
4. **Trail:** Each particle leaves a fading trail (0.3s decay)
5. **Value encoding:** More particles = higher value. Particle brightness = gas price relative to base fee
6. **Direction:** Particles always flow sender → receiver

After the stream completes, the arc path persists as a ghost line (text-ghost opacity) for 30 seconds, showing the connection existed.

### Idle State (Empty Chain)

When no transactions are flowing (current state of this chain):
- Known addresses pulse gently at staggered intervals
- Faint connection lines between previously-connected addresses at ghost opacity
- Slow orbital drift — all nodes rotate around center at 0.001 rad/s
- Occasional "firefly" ambient particles drift through empty space
- The constellation breathes — nodes scale up/down 2% on a 5s cycle

### With Pending Transactions (requires `eth_subscribe("newPendingTransactions")`)

- Pending tx: particle spawns at sender but doesn't arc yet — orbits the sender node, pulsing dream-bright (#9494b4)
- Inclusion: orbiting particle *snaps* along the arc to receiver, color shifts dream → bone
- Revert: orbiting particle shatters (8 fragments, outward, fade to danger red)

### Camera

- 2D top-down (orthographic), slight perspective tilt optional
- Scroll wheel zooms (smooth lerp)
- Click-drag pans
- Double-click an address → zoom to it, show detail panel

### Implementation Notes

WebGL2 particle system. Not Three.js InstancedMesh (too heavy for thousands of particles).
Use raw WebGL2 with transform feedback for GPU-side particle physics, or a lightweight lib like `regl`.

```javascript
// Particle buffer: position, velocity, lifetime, color, size
// Updated per-frame on GPU via transform feedback
// Rendered as GL_POINTS with custom fragment shader for soft circles
```

---

## Scene 3: Block Waterfall

**The timeline.** Blocks as translucent volumes falling and stacking.

### Concept

New blocks appear at the top of the screen and fall downward, stacking into a column.
Each block is a translucent rectangular volume. Its visual properties encode the block's data.
The stack compresses as it grows — older blocks become thinner, eventually just lines.

### Block Rendering

Each block is a rect with:

```
Width:    fixed (matches column width)
Height:   lerp(4px, 60px) based on gas_used / gas_limit
          empty blocks = thin slivers, full blocks = tall
Border:   1px solid rgba(255,255,255,0.07) — standard ROSEDUST border
Fill:     hash-derived pattern (see below)
Left bar: 2px accent — rose-dim normally, rose-glow if block has txns
```

### Hash-Derived Fill

Each block's interior fill is generated from its hash:
- Divide the rect into 8 vertical bands
- Each band's opacity/color comes from 4 bytes of the hash
- Result: each block has a unique "barcode" pattern — visually distinct, recognizable

### Falling Animation

1. Block spawns above viewport, transparent
2. Falls with eased deceleration (expo ease-out) over 400ms
3. Lands on top of stack with subtle "impact" — 1px white flash on bottom edge, 80ms
4. Block below compresses slightly (spring physics, 200ms settle)

### Stack Behavior

- Visible stack: 50 most recent blocks
- Blocks compress as they age: `height = base_height * (1 - age/50 * 0.7)`
- Oldest visible blocks are nearly line-thin
- Below the visible stack: fade to void

### Transaction Indicators Within Blocks

When a block has transactions, they're visible inside the block rect:
- Each transaction is a horizontal line at a Y position proportional to its index
- Line color: bone-bright (value transfer) or rose (contract interaction)
- Line length: proportional to gas used
- Hovering the block expands it and reveals transaction details

### Complementary Element: Block Pulse

Below the waterfall, a pulse indicator:
```
●───●───●───●───●───●───●───●
```
One dot per second, advancing left to right. When a block arrives, the current dot flashes rose-glow.
Missed blocks (if any) leave a gap. This is the chain's heartbeat — steady rhythm made visible.

---

## Scene 4: Consensus Ring (requires `kora_consensusState`)

**The governance visualizer.** Validators as nodes on a ring. Consensus as light accumulating.

### Concept

N validators arranged on a circle. As a consensus round progresses:
1. **Propose:** One node glows bright (the proposer), sends a pulse outward
2. **Pre-vote:** Nodes that vote glow rose as they sign. Lines connect them to center.
3. **Pre-commit:** Voted nodes intensify. The center begins to crystallize.
4. **Finalize:** Threshold reached → the center *flashes* and a block materializes, drops into the waterfall

### Visual

```
         ◉  proposer (bright rose, pulsing)
       ╱   ╲
      ●     ●  voted (rose-dim, connected to center)
     ╱  ┌─┐  ╲
    ○   │◆│   ○  center: forming block (dream → rose as votes accumulate)
     ╲  └─┘  ╱
      ●     ○  not yet voted (text-ghost)
       ╲   ╱
         ○
```

### Threshold Visualization

A circular progress arc around the ring. As votes come in, the arc fills.
2/3 threshold is marked with a faint line. When the arc crosses it: crystallization event.

### Without `kora_consensusState`

If the custom RPC isn't available, this scene can approximate by using block arrival timing.
Blocks arriving at steady 1s intervals → calm steady ring pulse.
Blocks arriving late → ring "strains," amber warning glow.
Missed blocks → ring dims, gap in pulse.

---

## Scene 5: State Diff Aurora (requires `debug_traceTransaction` or state diff data)

**The change visualizer.** State changes as color bands in an aurora-like display.

### Concept

When a block changes state (different stateRoot from parent), the *type* and *magnitude*
of changes render as flowing color bands across the top of the viewport:

- Balance changes → bone-colored band, width = total ETH moved
- Storage writes → rose-colored band, width = number of slots written
- Contract creations → bright rose flash, single vertical stripe
- Self-destructs → red band, sharp edge

The bands flow left-to-right, stretching and fading, like a slow-motion aurora borealis.
When the chain is idle (same stateRoot), the aurora dims to nothing.

This is pure atmosphere — no data to click, no interactivity. Just a visual ambient indicator
of "how much changed in the world this second."

---

## Scene 6: Contract Organism (requires `eth_getCode` + `debug_storageRangeAt`)

**The deep dive.** A single contract rendered as a living organism.

### Concept

Select a contract address. Its bytecode length determines body size.
Its storage slots are rendered as a grid of cells (like a cellular automaton).
As blocks pass and storage changes, cells light up, change color, pulse.

- **Cold storage (unchanged):** text-ghost, barely visible
- **Warm storage (changed in last 100 blocks):** rose-dim, soft pulse
- **Hot storage (changed this block):** bone-bright, sharp flash, decay over 5s
- **New slot (first write):** rose-bright border flash (birth)
- **Zeroed slot (cleared):** brief red flash, then invisible

### Layout

The storage grid wraps into a roughly square shape. The contract's "body" is this grid.
External calls to the contract appear as particles arriving from off-screen, hitting the organism,
and triggering cell changes.

This scene only works for a single contract at a time. It's the microscope view.

---

## Scene Composition

Multiple scenes can run simultaneously with controlled blending:

| Explorer Mode | Primary Scene | Secondary Scene | Overlay |
|---------------|---------------|-----------------|---------|
| TERRAIN | Hash Terrain (100%) | — | Glass panels, pulse |
| MOSAIC | — (2D tiles instead) | — | Glass panels |
| DETAIL: Block | Block Waterfall (left) | Hash Terrain tile (right bg) | Data panels |
| DETAIL: Address | Particle Constellation (focused) | — | Balance/tx panels |
| DETAIL: Contract | Contract Organism | — | Code/storage panels |
| DETAIL: Consensus | Consensus Ring | Block Waterfall (small) | Validator panels |

Scene transitions use a 500ms crossfade (opacity) with the incoming scene offset by `translateY(12px)` fading up (standard ROSEDUST entrance).

---

## Implementation Deep Dives

### Scene 1 — Hash Terrain: Heightmap Algorithm

The 32-byte block hash deterministically generates a 16×16 terrain tile. Here's the exact algorithm:

```typescript
// apps/explorer/src/scenes/terrain/hashmap.ts

/** Convert a 32-byte block hash into a 16×16 heightmap */
export function hashToHeightmap(hashHex: string): Float32Array {
  const bytes = hexToBytes(hashHex); // Uint8Array[32]
  const heights = new Float32Array(256); // 16×16 grid

  // Extract terrain parameters from hash byte groups
  const baseElevation = bytesToNorm(bytes, 0, 4);    // 0.0 - 1.0
  const roughness     = bytesToNorm(bytes, 4, 8);    // noise frequency
  const ridgeFactor   = bytesToNorm(bytes, 8, 12);   // ridge presence
  const erosion       = bytesToNorm(bytes, 12, 16);  // smoothing factor
  const hueShift      = bytesToNorm(bytes, 16, 20);  // palette offset
  const saturation    = bytesToNorm(bytes, 20, 24);  // color intensity
  const blendCurve    = bytesToNorm(bytes, 24, 28);  // edge blending
  const features      = bytesToNorm(bytes, 28, 32);  // peaks/valleys

  // Seed a deterministic PRNG from the full hash
  const rng = mulberry32(bytesToUint32(bytes, 0));

  for (let y = 0; y < 16; y++) {
    for (let x = 0; x < 16; x++) {
      const i = y * 16 + x;
      const nx = x / 15; // normalized 0-1
      const ny = y / 15;

      // Layer 1: Perlin noise at hash-derived frequency
      let h = perlin2D(nx * (2 + roughness * 6), ny * (2 + roughness * 6), rng);

      // Layer 2: Ridge noise (absolute value creates sharp peaks)
      const ridge = 1.0 - Math.abs(perlin2D(nx * 4, ny * 4, rng));
      h = lerp(h, ridge, ridgeFactor * 0.6);

      // Layer 3: Base elevation offset
      h = h * 0.7 + baseElevation * 0.3;

      // Layer 4: Erosion smoothing (reduces high frequencies)
      // Applied later as a blur pass

      // Layer 5: Feature placement (peaks at deterministic positions)
      const featureX = features * 15;
      const featureY = (1 - features) * 15;
      const distToFeature = Math.hypot(x - featureX, y - featureY) / 16;
      h += Math.max(0, 0.3 - distToFeature) * features;

      heights[i] = clamp(h, 0, 1);
    }
  }

  // Erosion pass: box blur with hash-derived kernel size
  if (erosion > 0.2) {
    const kernelSize = Math.round(1 + erosion * 2); // 1-3
    boxBlur(heights, 16, 16, kernelSize);
  }

  return heights;
}

/** Convert 4 bytes at offset to normalized float 0-1 */
function bytesToNorm(bytes: Uint8Array, start: number, end: number): number {
  let val = 0;
  for (let i = start; i < end; i++) val = (val << 8) | bytes[i];
  return val / 0xFFFFFFFF;
}
```

### Scene 1 — Hash Terrain: Vertex Shader

```glsl
// apps/explorer/src/scenes/terrain/terrain.vert.glsl
precision highp float;

uniform float uTime;
uniform float uTileIndex;     // which tile in the ring buffer
uniform float uActivityLevel; // 0.0 (empty block) → 1.0 (full block)

attribute float aHeight;      // from heightmap, 0-1

varying float vHeight;
varying float vFogDepth;
varying vec2 vUv;

void main() {
  vHeight = aHeight;
  vUv = uv;

  // Displace Y by height
  vec3 pos = position;
  pos.y = aHeight * 2.0 * (0.3 + uActivityLevel * 0.7);
  // Empty blocks: max height 0.6. Full blocks: max height 2.0.

  vec4 mvPos = modelViewMatrix * vec4(pos, 1.0);
  vFogDepth = -mvPos.z; // distance from camera

  gl_Position = projectionMatrix * mvPos;
}
```

### Scene 1 — Hash Terrain: Fragment Shader

```glsl
// apps/explorer/src/scenes/terrain/terrain.frag.glsl
precision highp float;

uniform vec3 uColorLow;    // rose-deep #3a2030
uniform vec3 uColorHigh;   // rose-glow #dca5bd
uniform vec3 uColorPeak;   // bone-bright #d8c8a0
uniform vec3 uFogColor;    // void #060608
uniform float uFogDensity;
uniform float uHueShift;   // from hash bytes[16..20], 0-1
uniform float uSaturation; // from hash bytes[20..24], 0-1

varying float vHeight;
varying float vFogDepth;
varying vec2 vUv;

// HSV rotation for hash-derived hue shift
vec3 hueRotate(vec3 color, float shift) {
  float angle = shift * 0.3; // max ±0.15 turns — subtle
  float s = sin(angle * 6.2832);
  float c = cos(angle * 6.2832);
  vec3 weights = vec3(0.299, 0.587, 0.114);
  float dot_val = dot(color, weights);
  vec3 result;
  result.r = dot_val + (color.r - dot_val) * c + (0.168 * color.r - 0.131 * color.g - 0.037 * color.b) * s;
  result.g = dot_val + (color.g - dot_val) * c + (0.330 * color.r + 0.174 * color.g - 0.504 * color.b) * s;
  result.b = dot_val + (color.b - dot_val) * c + (-0.497 * color.r + 0.429 * color.g + 0.068 * color.b) * s;
  return result;
}

void main() {
  // Height-based color gradient: valley → mid → peak
  vec3 color;
  if (vHeight < 0.5) {
    color = mix(uColorLow, uColorHigh, vHeight * 2.0);
  } else {
    color = mix(uColorHigh, uColorPeak, (vHeight - 0.5) * 2.0);
  }

  // Apply hash-derived hue shift
  color = hueRotate(color, uHueShift - 0.5);

  // Saturation modulation
  float grey = dot(color, vec3(0.299, 0.587, 0.114));
  color = mix(vec3(grey), color, 0.6 + uSaturation * 0.4);

  // Exponential fog
  float fogFactor = 1.0 - exp(-uFogDensity * vFogDepth * vFogDepth);
  color = mix(color, uFogColor, clamp(fogFactor, 0.0, 1.0));

  gl_FragColor = vec4(color, 1.0);
}
```

### Scene 1 — Hash Terrain: Ring Buffer Architecture

```typescript
// Terrain tile management
const RING_SIZE = 200; // max tiles in memory

interface TerrainTile {
  blockNumber: bigint;
  mesh: THREE.Mesh;          // PlaneGeometry(1, 1, 15, 15)
  heightmap: Float32Array;   // 256 floats
  activityLevel: number;     // gasUsed / gasLimit
  hueShift: number;          // from hash
  saturation: number;        // from hash
}

class TerrainRing {
  private tiles: TerrainTile[] = [];
  private head = 0; // index of newest tile

  addBlock(block: Block): void {
    const heightmap = hashToHeightmap(block.hash);
    const activity = Number(block.gasUsed) / Number(block.gasLimit);

    if (this.tiles.length < RING_SIZE) {
      // Growing phase: create new mesh
      const tile = this.createTile(block, heightmap, activity);
      this.tiles.push(tile);
    } else {
      // Recycle oldest tile
      const oldest = this.tiles[this.head];
      this.recycleTile(oldest, block, heightmap, activity);
    }
    this.head = (this.head + 1) % RING_SIZE;
  }

  // Camera X position = total blocks seen (modulo viewport width)
  // Only render tiles within camera frustum (typically 30-40 visible)
}
```

### Scene 2 — Particle Constellation: GPU Architecture

The constellation uses raw WebGL2 with `regl` for maximum particle throughput. No Three.js here — we need 10,000+ particles with per-frame GPU updates.

```typescript
// apps/explorer/src/scenes/constellation/particles.ts

interface ParticleBuffer {
  // Per-particle attributes (GPU buffer, updated via transform feedback)
  position: Float32Array;   // [x, y] — 2 floats per particle
  velocity: Float32Array;   // [vx, vy] — arc-following velocity
  lifetime: Float32Array;   // 0.0 (just spawned) → 1.0 (dead)
  color: Float32Array;      // [r, g, b, a] — 4 floats
  size: Float32Array;       // point size in pixels

  // Capacity
  maxParticles: 16384;      // 16K particles max
  activeCount: number;      // current live count
}

// GPU particle update shader (transform feedback)
const UPDATE_VERT = `#version 300 es
  in vec2 aPosition;
  in vec2 aVelocity;
  in float aLifetime;

  out vec2 vPosition;
  out vec2 vVelocity;
  out float vLifetime;

  uniform float uDeltaTime;

  void main() {
    vPosition = aPosition + aVelocity * uDeltaTime;
    vVelocity = aVelocity * 0.99; // slight drag
    vLifetime = aLifetime + uDeltaTime * 0.3; // age
  }
`;

// Render shader (GL_POINTS with soft circle)
const RENDER_FRAG = `#version 300 es
  precision highp float;
  in vec4 vColor;
  out vec4 fragColor;

  void main() {
    // Soft circle from point sprite
    vec2 coord = gl_PointCoord - 0.5;
    float dist = length(coord);
    float alpha = 1.0 - smoothstep(0.3, 0.5, dist);
    fragColor = vec4(vColor.rgb, vColor.a * alpha);
  }
`;
```

### Scene 2 — Address Node Layout Algorithm

```typescript
// Deterministic address → screen position
function addressToPosition(address: string): { x: number; y: number; z: number } {
  const bytes = hexToBytes(address); // 20 bytes

  // Golden angle spiral distribution
  const angleInt = bytesToUint32(new Uint8Array(bytes.buffer, 0, 4));
  const theta = angleInt * 2.39996323; // golden angle in radians

  // Radius: log scale so active addresses cluster inward
  const radiusInt = bytesToUint32(new Uint8Array(bytes.buffer, 4, 4));
  const baseRadius = radiusInt / 0xFFFFFFFF; // 0-1
  const radius = 0.1 + Math.log(1 + baseRadius * 9) / Math.log(10) * 0.9;
  // Result: radius ∈ [0.1, 1.0] with logarithmic distribution

  // Subtle z-depth for parallax
  const zInt = bytesToUint32(new Uint8Array(bytes.buffer, 8, 4));
  const z = (zInt / 0xFFFFFFFF - 0.5) * 0.5; // ±0.25

  return {
    x: Math.cos(theta) * radius,
    y: Math.sin(theta) * radius,
    z,
  };
}
```

### Scene 2 — Transaction Arc Bezier

```typescript
// Cubic bezier arc from sender → receiver
function computeArc(
  from: { x: number; y: number },
  to: { x: number; y: number },
  value: bigint, // transaction value in wei
): { controlPoint: { x: number; y: number }; segments: number } {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const dist = Math.hypot(dx, dy);

  // Perpendicular offset proportional to value (capped)
  const valueEth = Number(value) / 1e18;
  const curvature = Math.min(0.4, 0.05 + Math.log(1 + valueEth) * 0.1);

  // Control point: midpoint offset perpendicular to the line
  const midX = (from.x + to.x) / 2;
  const midY = (from.y + to.y) / 2;
  const perpX = -dy / dist * curvature;
  const perpY = dx / dist * curvature;

  return {
    controlPoint: { x: midX + perpX, y: midY + perpY },
    segments: Math.max(8, Math.min(32, Math.round(8 + valueEth * 2))),
  };
}
```

### Scene 3 — Block Waterfall: Hash Barcode Pattern

```typescript
// apps/explorer/src/scenes/waterfall/hashbar.ts

/** Generate a barcode pattern from block hash for waterfall card fill */
export function hashToBarcode(
  hashHex: string,
  width: number,
  height: number,
  ctx: CanvasRenderingContext2D,
): void {
  const bytes = hexToBytes(hashHex);
  const bandWidth = width / 8;

  for (let i = 0; i < 8; i++) {
    // Each 4-byte group controls one vertical band
    const b0 = bytes[i * 4];
    const b1 = bytes[i * 4 + 1];
    const b2 = bytes[i * 4 + 2];
    const b3 = bytes[i * 4 + 3];

    // Hue: slight variation within rose spectrum
    const hue = 330 + (b0 / 255) * 30; // 330-360 (rose range)
    const sat = 20 + (b1 / 255) * 30;  // 20-50%
    const light = 10 + (b2 / 255) * 25; // 10-35%
    const alpha = 0.15 + (b3 / 255) * 0.45; // 0.15-0.60

    ctx.fillStyle = `hsla(${hue}, ${sat}%, ${light}%, ${alpha})`;
    ctx.fillRect(i * bandWidth, 0, bandWidth, height);
  }
}
```

### Scene 4 — Consensus Ring: Threshold Animation

```typescript
// When votes accumulate past 2/3 threshold
interface ConsensusAnimation {
  // Ring of N validators
  validators: Array<{
    angle: number;     // position on ring (2π / N * i)
    voted: boolean;
    glowIntensity: number; // 0 → 1 on vote, animated
  }>;

  // Progress arc
  voteCount: number;
  threshold: number;  // 2f+1
  arcProgress: number; // animated 0 → voteCount/totalValidators

  // Center block crystallization
  crystallization: number; // 0 (no votes) → 1 (threshold crossed)
  // When crystallization hits 1.0:
  //   - Flash: white overlay 0 → 0.3 → 0 over 200ms
  //   - Block drops into waterfall
  //   - Ring resets for next round
}
```

### Scene 4 — Consensus Ring: Data Source Mapping

With the `kora_subscribe("consensus")` WebSocket subscription, the Consensus Ring receives all 10 Activity types from the Simplex engine. Here's how each event maps to visuals:

```typescript
// apps/explorer/src/scenes/consensus/ConsensusRing.tsx

bus.on('consensus:event', (event: ConsensusEvent) => {
  switch (event.type) {
    case 'notarize':
      // This node voted to notarize
      // → Current leader node brightens (they proposed)
      // → This node's ring segment fills rose-dim
      // → Threshold arc advances by 1/threshold
      ring.setPhase('notarizing');
      ring.highlightLeader(event.view % VALIDATOR_COUNT);
      ring.advanceArc(1);
      break;

    case 'notarization':
      // Quorum achieved — enough validators notarized
      // → All participating nodes glow rose
      // → Threshold arc completes (fills to 100%)
      // → Center block wireframe solidifies (dream-colored)
      // → Transition: notarizing → certifying
      ring.setPhase('certifying');
      ring.fillArc(1.0);
      ring.centerBlock.solidify('dream');
      break;

    case 'certification':
      // This node voted to finalize
      // → This node shows double ring (certification indicator)
      // → Center block shifts dream → rose
      ring.setPhase('finalizing');
      ring.showCertificationRing(THIS_VALIDATOR_INDEX);
      ring.centerBlock.shiftColor('dream', 'rose');
      break;

    case 'finalize':
      // Individual finalization vote received
      // → Same as certification visually
      ring.showCertificationRing(THIS_VALIDATOR_INDEX);
      break;

    case 'finalization':
      // Block finalized — committed to chain
      // → ALL nodes flash bright (200ms white overlay)
      // → Center block CRYSTALLIZES (sharp edges, bone-bright)
      // → Block drops into waterfall scene below
      // → Ring resets for next round
      // → Timing arc records this round's duration
      ring.setPhase('finalized');
      ring.flashAll(200);
      ring.centerBlock.crystallize();
      ring.dropToWaterfall(event.blockHeight);
      ring.recordRoundDuration(event.timestampMs - ring.roundStartMs);
      setTimeout(() => ring.reset(event.view + 1), 500);
      break;

    case 'nullify':
      // This node voted to nullify (timeout)
      // → This node dims to warning amber
      // → Threshold arc color shifts rose → amber
      ring.setPhase('nullifying');
      ring.dimNode(THIS_VALIDATOR_INDEX, 'warning');
      ring.arcColor('warning');
      break;

    case 'nullification':
      // View skipped — no block produced
      // → All nodes dim to text-ghost
      // → Center wireframe SHATTERS (fragments outward, fade)
      // → Timing arc records as nullified (ghost color)
      // → Ring resets
      ring.setPhase('nullified');
      ring.dimAll('ghost');
      ring.centerBlock.shatter();
      ring.recordNullification();
      setTimeout(() => ring.reset(event.view + 1), 500);
      break;

    // ── Safety Violations ──
    // These trigger dramatic, persistent visual alerts

    case 'conflictingNotarize':
    case 'conflictingFinalize':
    case 'nullifyFinalize':
      // SAFETY VIOLATION
      // → Full-screen red flash (30% opacity, 200ms)
      // → All ring connections turn danger red
      // → Ring border pulses red continuously
      // → Alert panel appears (does NOT auto-dismiss)
      // → Audio: dissonant chord (if enabled)
      ring.safetyViolation(event.type);
      atmosphere.flashDanger();
      alertPanel.show({
        type: event.type,
        timestamp: event.timestampMs,
        severity: 'critical',
      });
      audio?.playViolationChord();
      break;
  }
});
```

### Scene 4 — Consensus Ring: Round History Visualization

Below the main ring, a horizontal timeline shows recent round history:

```
┌─ ROUND HISTORY ────────────────────────────────────────────────┐
│                                                                 │
│  ██ ██ ██ ██ ██ ██ ██ ██ ░░ ██ ██ ██ ██ ██ ██ ██ ██ ██ ██ ██  │
│  98 99 00 01 02 03 04 05 06 07 08 09 10 11 12 13 14 15 16 17  │
│                          ↑                                      │
│                     nullified                                   │
│                                                                 │
│  avg: 450ms  nullified: 1/20 (5%)  violations: 0               │
└─────────────────────────────────────────────────────────────────┘
```

Each bar:
- **Height** = round duration (taller = slower finalization)
- **Color** = outcome (rose = finalized, ghost = nullified, danger = violation)
- **Width** = uniform (one bar per round)

This gives at-a-glance consensus health: a wall of uniform rose bars means the chain is healthy. Gaps (nullified) or red bars (violations) are instantly visible.

### Scene Lifecycle Management

```typescript
// apps/explorer/src/scenes/SceneManager.tsx
// Controls which scenes are active, handles transitions

interface SceneState {
  active: SceneName;            // currently rendering
  transitioning: boolean;       // during crossfade
  transitionProgress: number;   // 0-1

  // Each scene maintains its own WebGL context / R3F canvas
  // Only the active scene runs its animation loop
  // Inactive scenes are suspended (requestAnimationFrame paused)
}

// Scene lifecycle:
// 1. MOUNT: Scene component mounts, WebGL context created
// 2. WARM: Data starts flowing (EventBus subscriptions active)
// 3. ACTIVE: Animation loop running, visible
// 4. FADE-OUT: opacity 1→0 over 300ms, animation loop still running
// 5. SUSPEND: Animation loop paused, EventBus subscriptions kept
//             (scene stays warm for fast re-entry)
// 6. UNMOUNT: Only on mode unavailability (e.g., consensus scene without kora_consensusState)
//             WebGL context destroyed, all resources freed

// Memory: suspended scenes retain their GPU buffers (~10-30MB each)
// If memory pressure detected (navigator.deviceMemory < 4GB):
//   - Only keep active scene + one suspended scene
//   - Others fully unmount
```

### Hash Art Generator (Mosaic Tiles + Block Detail)

The generative art from block hashes uses Canvas2D for portability:

```typescript
// apps/explorer/src/panels/DetailView/HashArt.tsx

type PatternAlgorithm = 'flow' | 'voronoi' | 'circuit' | 'wave'
  | 'crystal' | 'spiral' | 'grid' | 'organic';

export function generateHashArt(
  hash: string,
  size: number,      // 128 for detail view, 64 for mosaic tile
  canvas: HTMLCanvasElement,
): void {
  const bytes = hexToBytes(hash);
  const ctx = canvas.getContext('2d')!;

  // bytes[0..8]: select algorithm (8 patterns)
  const algorithmIndex = bytes[0] % 8;
  const algorithm = PATTERNS[algorithmIndex];

  // bytes[8..16]: color variation within rose spectrum
  const hueBase = 330 + (bytesToNorm(bytes, 8, 10) * 30);   // 330-360
  const satRange = 20 + (bytesToNorm(bytes, 10, 12) * 40);  // 20-60

  // bytes[16..24]: symmetry
  const symmetry = bytes[16] % 4; // 0=none, 1=bilateral, 2=radial-4, 3=radial-8

  // bytes[24..32]: density/fill
  const density = bytesToNorm(bytes, 24, 28);
  const fillFactor = bytesToNorm(bytes, 28, 32);

  // Draw with symmetry
  ctx.fillStyle = '#060608'; // void background
  ctx.fillRect(0, 0, size, size);

  algorithm(ctx, size, { hueBase, satRange, density, fillFactor, bytes });

  // Apply symmetry transforms
  applySymmetry(ctx, canvas, size, symmetry);
}
```

Each of the 8 pattern algorithms produces visually distinct generative art, all within the ROSEDUST color space. The result is unique per block hash and deterministic — the same hash always produces the same image. These images are downloadable as PNG from the block detail view.
