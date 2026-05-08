import { describe, it, expect } from 'vitest';
import { generateHashArt } from './hashArt';
function createMockContext() {
    const fills = [];
    const ctx = {
        fillStyle: '',
        fillRect(x, y, w, h) {
            fills.push({ style: ctx.fillStyle, x, y, w, h });
        },
        fills,
    };
    return ctx;
}
describe('generateHashArt', () => {
    it('produces deterministic output for the same hash', () => {
        const hash = '0x6d1fb175dce4b6795ca4dc32a19a0d6b6ab5c25b8e48ce49aaee6c28d5d5a0c1';
        const ctx1 = createMockContext();
        const ctx2 = createMockContext();
        generateHashArt(ctx1, hash, 64);
        generateHashArt(ctx2, hash, 64);
        expect(ctx1.fills).toEqual(ctx2.fills);
    });
    it('produces different output for different hashes', () => {
        const hash1 = '0x6d1fb175dce4b6795ca4dc32a19a0d6b6ab5c25b8e48ce49aaee6c28d5d5a0c1';
        const hash2 = '0x0000000000000000000000000000000000000000000000000000000000000001';
        const ctx1 = createMockContext();
        const ctx2 = createMockContext();
        generateHashArt(ctx1, hash1, 64);
        generateHashArt(ctx2, hash2, 64);
        expect(ctx1.fills).not.toEqual(ctx2.fills);
    });
    it('always renders exactly 8x8 grid cells max (plus background)', () => {
        const hash = '0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff';
        const ctx = createMockContext();
        generateHashArt(ctx, hash, 64);
        // First fill is always the background rect
        expect(ctx.fills[0]).toEqual(expect.objectContaining({ x: 0, y: 0, w: 64, h: 64 }));
        // Remaining fills are grid cells, each 8x8 pixels
        const cellFills = ctx.fills.slice(1);
        for (const fill of cellFills) {
            expect(fill.w).toBe(8);
            expect(fill.h).toBe(8);
        }
    });
    it('has bilateral symmetry (mirrored left-right)', () => {
        const hash = '0x6d1fb175dce4b6795ca4dc32a19a0d6b6ab5c25b8e48ce49aaee6c28d5d5a0c1';
        const ctx = createMockContext();
        generateHashArt(ctx, hash, 64);
        const cellFills = ctx.fills.slice(1);
        const cellMap = new Map();
        for (const fill of cellFills) {
            cellMap.set(`${fill.x},${fill.y}`, fill.style);
        }
        for (const fill of cellFills) {
            const col = fill.x / 8;
            if (col >= 4)
                continue;
            const mirrorX = (7 - col) * 8;
            const mirrorKey = `${mirrorX},${fill.y}`;
            expect(cellMap.has(mirrorKey)).toBe(true);
            expect(cellMap.get(mirrorKey)).toBe(fill.style);
        }
    });
});
