import { hashToHeightmap, extractParams } from './hashmap';
const TILE_SPACING = 8.5; // Z distance between tile centers (8 tile + 0.5 gap)
const SCROLL_SPEED = 8.5; // world units per second (1 tile per second)
const FADE_IN_DURATION = 0.4; // seconds
const DEFAULT_PARAMS = {
    baseElevation: 0.5,
    roughness: 0.5,
    ridgeFactor: 0.5,
    erosion: 0.5,
    hueShift: 0.5,
    saturation: 0.5,
    blendCurve: 0.5,
    features: 0.5,
};
export class TerrainRing {
    capacity;
    slots;
    head = 0;
    scrollOffset = 0;
    totalBlocksSeen = 0;
    reducedMotion;
    constructor(capacity = 10) {
        this.capacity = capacity;
        this.reducedMotion =
            typeof window !== 'undefined' &&
                window.matchMedia('(prefers-reduced-motion: reduce)').matches;
        this.slots = Array.from({ length: capacity }, (_, i) => ({
            id: i,
            active: false,
            blockNumber: 0n,
            blockHash: null,
            heightmap: null,
            activityLevel: 0,
            params: { ...DEFAULT_PARAMS },
            transactions: [],
            positionZ: -i * TILE_SPACING,
            opacity: 0,
            age: 0,
        }));
    }
    getSlots() {
        return this.slots;
    }
    /**
     * Advance the ring buffer with a new block.
     * Recycles the oldest slot and assigns it to the new block.
     */
    advance(block) {
        this.totalBlocksSeen++;
        const recycleIdx = this.head;
        this.head = (this.head + 1) % this.capacity;
        const slot = this.slots[recycleIdx];
        // Generate heightmap (32x32 grid)
        const heightmap = hashToHeightmap(block.hash, 32);
        const params = extractParams(block.hash);
        slot.active = true;
        slot.blockNumber = block.number;
        slot.blockHash = block.hash;
        slot.heightmap = heightmap;
        slot.activityLevel = block._activityLevel;
        slot.params = params;
        slot.transactions = block.transactions;
        slot.age = 0;
        slot.opacity = this.reducedMotion ? 1 : 0;
        // Position new tile at the far end (emerging from fog)
        slot.positionZ = -(this.totalBlocksSeen * TILE_SPACING) + this.scrollOffset;
    }
    /**
     * Called every frame. Scrolls all tiles toward the camera
     * and handles fade-in animation.
     */
    update(delta) {
        if (this.reducedMotion)
            return;
        this.scrollOffset += SCROLL_SPEED * delta;
        for (const slot of this.slots) {
            if (!slot.active)
                continue;
            slot.positionZ += SCROLL_SPEED * delta;
            // Fade-in
            slot.age += delta;
            if (slot.age < FADE_IN_DURATION) {
                slot.opacity = slot.age / FADE_IN_DURATION;
            }
            else {
                slot.opacity = 1;
            }
            // Fade-out when tile has scrolled past the camera
            if (slot.positionZ > 15) {
                slot.opacity = Math.max(0, 1 - (slot.positionZ - 15) / 5);
            }
        }
    }
    visibleCount() {
        return this.slots.filter((s) => s.active && s.opacity > 0).length;
    }
}
