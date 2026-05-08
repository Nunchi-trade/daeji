import { clamp, lerp } from '@/lib/math';
// ── Constants ────────────────────────────────────────────────
const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5)); // ~2.39996 radians
const MAX_UINT32 = 0xffffffff;
// ROSEDUST palette (rgb [0..1])
const COLOR_DORMANT = [0.227, 0.188, 0.227, 0.15]; // #3a303a
const COLOR_ACTIVE = [0.863, 0.647, 0.741, 1.0]; // #dca5bd rose-glow
const COLOR_CONTRACT = [0.667, 0.439, 0.533, 1.0]; // #aa7088 rose
export const COLOR_HIGH_VALUE = [0.847, 0.784, 0.627, 1.0]; // #d8c8a0 bone-bright
// ── Deterministic position from address ──────────────────────
function parseHex4(hex, offset) {
    return parseInt(hex.slice(offset, offset + 8), 16);
}
/**
 * Golden-ratio spiral mapping: 20-byte address -> deterministic (x, y, z).
 * Outputs are in [-1, 1] range for x/y; z is ±0.25 for subtle parallax.
 */
export function addressToPosition(address) {
    const clean = address.slice(2); // strip 0x
    const angleInt = parseHex4(clean, 0);
    const radiusInt = parseHex4(clean, 8);
    const zInt = parseHex4(clean, 16);
    const theta = angleInt * GOLDEN_ANGLE;
    const baseRadius = radiusInt / MAX_UINT32;
    // Log scale so active addresses cluster inward
    const radius = 0.1 + Math.log(1 + baseRadius * 9) / Math.log(10) * 0.9;
    const x = Math.cos(theta) * radius;
    const y = Math.sin(theta) * radius;
    const z = (zInt / MAX_UINT32 - 0.5) * 0.5;
    return { x, y, z };
}
// ── AddressBook ──────────────────────────────────────────────
export class AddressBook {
    nodes = new Map();
    list = [];
    dirty = false;
    maxNodes;
    constructor(maxNodes = 4096) {
        this.maxNodes = maxNodes;
    }
    getOrCreate(address) {
        const key = address.toLowerCase();
        let node = this.nodes.get(key);
        if (node)
            return node;
        if (this.nodes.size >= this.maxNodes) {
            this.evictLeastActive();
        }
        const pos = addressToPosition(address);
        node = {
            address: address,
            x: pos.x,
            y: pos.y,
            z: pos.z,
            isContract: false,
            r: COLOR_DORMANT[0],
            g: COLOR_DORMANT[1],
            b: COLOR_DORMANT[2],
            a: COLOR_DORMANT[3],
            size: 2,
            pulsePhase: Math.random() * Math.PI * 2,
            lastActiveTime: 0,
            txCount: 0,
        };
        this.nodes.set(key, node);
        this.dirty = true;
        return node;
    }
    get(address) {
        return this.nodes.get(address.toLowerCase());
    }
    markActive(node) {
        node.lastActiveTime = performance.now();
        node.txCount++;
        node.a = 1.0;
        node.size = clamp(4 + Math.log(1 + node.txCount) * 1.5, 4, 8);
        const c = node.isContract ? COLOR_CONTRACT : COLOR_ACTIVE;
        node.r = c[0];
        node.g = c[1];
        node.b = c[2];
    }
    markContract(node) {
        node.isContract = true;
        if (node.a > 0.5) {
            node.r = COLOR_CONTRACT[0];
            node.g = COLOR_CONTRACT[1];
            node.b = COLOR_CONTRACT[2];
        }
    }
    /** Update pulse/decay for all nodes. Call every frame. */
    update(dt) {
        const now = performance.now();
        for (const node of this.nodes.values()) {
            node.pulsePhase += dt * 2.4;
            if (node.pulsePhase > Math.PI * 2)
                node.pulsePhase -= Math.PI * 2;
            const ageSec = (now - node.lastActiveTime) / 1000;
            // Fade opacity back to dormant over 30s
            if (ageSec > 2 && node.a > COLOR_DORMANT[3]) {
                node.a = Math.max(COLOR_DORMANT[3], node.a - dt * 0.03);
            }
            // Shrink back to dormant size
            if (ageSec > 30 && node.size > 2) {
                node.size = Math.max(2, node.size - dt * 0.1);
            }
            // Color decay toward dormant
            if (ageSec > 60) {
                node.r = lerp(node.r, COLOR_DORMANT[0], dt * 0.1);
                node.g = lerp(node.g, COLOR_DORMANT[1], dt * 0.1);
                node.b = lerp(node.b, COLOR_DORMANT[2], dt * 0.1);
            }
        }
    }
    /** Breathing effect scale for idle animation. */
    breathScale(node, time) {
        // 5s cycle, ±2% scale
        return 1.0 + Math.sin(time * 1.2566 + node.pulsePhase) * 0.02;
    }
    /** Pulse scale for active nodes. */
    pulseScale(node) {
        if (node.a < 0.5)
            return 1.0;
        return 1.0 + Math.sin(node.pulsePhase) * 0.08;
    }
    getNodeList() {
        if (this.dirty) {
            this.list = Array.from(this.nodes.values());
            this.dirty = false;
        }
        return this.list;
    }
    get size() {
        return this.nodes.size;
    }
    evictLeastActive() {
        let oldest = null;
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
