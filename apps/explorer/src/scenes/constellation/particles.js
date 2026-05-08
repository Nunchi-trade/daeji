import { clamp } from '@/lib/math';
import { bezierPoint } from './arcs';
// ── Constants ────────────────────────────────────────────────
const MAX_PARTICLES = 16384;
// Bone-bright color for arc particles
const ARC_COLOR = [0.847, 0.784, 0.627]; // #d8c8a0
// Firefly ambient colors (rose-dim variations)
const FIREFLY_COLORS = [
    [0.667, 0.439, 0.533], // #aa7088
    [0.863, 0.647, 0.741], // #dca5bd
    [0.4, 0.33, 0.38], // muted rose
];
// ── Particle struct layout ───────────────────────────────────
// Each particle: x, y, vx, vy, life, maxLife, r, g, b, a, size
//  Total: 11 floats per particle
const FLOATS_PER = 11;
const OFF_X = 0;
const OFF_Y = 1;
const OFF_VX = 2;
const OFF_VY = 3;
const OFF_LIFE = 4;
const OFF_MAX_LIFE = 5;
const OFF_R = 6;
const OFF_G = 7;
const OFF_B = 8;
const OFF_A = 9;
const OFF_SIZE = 10;
// ── Particle System ──────────────────────────────────────────
export class ParticleSystem {
    /** Flat buffer: 11 floats per particle */
    data;
    /** Number of alive particles */
    count = 0;
    maxParticles;
    constructor(maxParticles = MAX_PARTICLES) {
        this.maxParticles = maxParticles;
        this.data = new Float32Array(maxParticles * FLOATS_PER);
    }
    /**
     * Spawn particles along a bezier arc for a transaction.
     * Particles are distributed evenly along the curve and travel forward.
     */
    spawnArcParticles(arc, particleCount) {
        const count = clamp(particleCount, 4, 24);
        for (let i = 0; i < count; i++) {
            if (this.count >= this.maxParticles)
                return;
            const t = i / count;
            const pos = bezierPoint(arc, t);
            // Velocity: tangent direction along the curve, with slight spread
            const tNext = Math.min(1, t + 0.01);
            const posNext = bezierPoint(arc, tNext);
            const dx = posNext.x - pos.x;
            const dy = posNext.y - pos.y;
            const len = Math.hypot(dx, dy) || 0.001;
            const speed = 0.15 + Math.random() * 0.1;
            const base = this.count * FLOATS_PER;
            this.data[base + OFF_X] = pos.x + (Math.random() - 0.5) * 0.005;
            this.data[base + OFF_Y] = pos.y + (Math.random() - 0.5) * 0.005;
            this.data[base + OFF_VX] = (dx / len) * speed;
            this.data[base + OFF_VY] = (dy / len) * speed;
            this.data[base + OFF_LIFE] = 0;
            this.data[base + OFF_MAX_LIFE] = 1.5 + Math.random() * 1.5; // 1.5..3s
            this.data[base + OFF_R] = ARC_COLOR[0];
            this.data[base + OFF_G] = ARC_COLOR[1];
            this.data[base + OFF_B] = ARC_COLOR[2];
            this.data[base + OFF_A] = 0.8 + Math.random() * 0.2;
            this.data[base + OFF_SIZE] = 2 + Math.random() * 2;
            this.count++;
        }
    }
    /** Spawn a few ambient firefly particles at random positions. */
    spawnFireflies(centerX, centerY, radius, count) {
        for (let i = 0; i < count; i++) {
            if (this.count >= this.maxParticles)
                return;
            const angle = Math.random() * Math.PI * 2;
            const r = Math.random() * radius;
            const color = FIREFLY_COLORS[Math.floor(Math.random() * FIREFLY_COLORS.length)];
            const base = this.count * FLOATS_PER;
            this.data[base + OFF_X] = centerX + Math.cos(angle) * r;
            this.data[base + OFF_Y] = centerY + Math.sin(angle) * r;
            this.data[base + OFF_VX] = (Math.random() - 0.5) * 0.02;
            this.data[base + OFF_VY] = (Math.random() - 0.5) * 0.02;
            this.data[base + OFF_LIFE] = 0;
            this.data[base + OFF_MAX_LIFE] = 3 + Math.random() * 4; // 3..7s
            this.data[base + OFF_R] = color[0];
            this.data[base + OFF_G] = color[1];
            this.data[base + OFF_B] = color[2];
            this.data[base + OFF_A] = 0.2 + Math.random() * 0.3;
            this.data[base + OFF_SIZE] = 1 + Math.random() * 1.5;
            this.count++;
        }
    }
    /** Advance all particles, removing dead ones. Call every frame. */
    update(dt) {
        let writeIdx = 0;
        for (let i = 0; i < this.count; i++) {
            const base = i * FLOATS_PER;
            const life = this.data[base + OFF_LIFE] + dt;
            const maxLife = this.data[base + OFF_MAX_LIFE];
            // Dead?
            if (life >= maxLife)
                continue;
            const writeBase = writeIdx * FLOATS_PER;
            // Advance position
            this.data[writeBase + OFF_X] = this.data[base + OFF_X] + this.data[base + OFF_VX] * dt;
            this.data[writeBase + OFF_Y] = this.data[base + OFF_Y] + this.data[base + OFF_VY] * dt;
            // Drag
            this.data[writeBase + OFF_VX] = this.data[base + OFF_VX] * 0.99;
            this.data[writeBase + OFF_VY] = this.data[base + OFF_VY] * 0.99;
            // Life
            this.data[writeBase + OFF_LIFE] = life;
            this.data[writeBase + OFF_MAX_LIFE] = maxLife;
            // Color (unchanged)
            this.data[writeBase + OFF_R] = this.data[base + OFF_R];
            this.data[writeBase + OFF_G] = this.data[base + OFF_G];
            this.data[writeBase + OFF_B] = this.data[base + OFF_B];
            // Alpha fades out in last 30% of lifetime
            const lifeRatio = life / maxLife;
            const fade = lifeRatio > 0.7 ? 1 - (lifeRatio - 0.7) / 0.3 : 1;
            this.data[writeBase + OFF_A] = this.data[base + OFF_A] * fade;
            // Size
            this.data[writeBase + OFF_SIZE] = this.data[base + OFF_SIZE];
            writeIdx++;
        }
        this.count = writeIdx;
    }
}
// Struct offsets for GPU attribute setup
export const PARTICLE_FLOATS_PER = FLOATS_PER;
export const PARTICLE_OFFSETS = {
    x: OFF_X,
    y: OFF_Y,
    vx: OFF_VX,
    vy: OFF_VY,
    life: OFF_LIFE,
    maxLife: OFF_MAX_LIFE,
    r: OFF_R,
    g: OFF_G,
    b: OFF_B,
    a: OFF_A,
    size: OFF_SIZE,
};
