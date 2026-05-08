import { describe, it, expect } from 'vitest';
import { hashToHeightmap, extractParams } from './hashmap';
const HASH_A = '0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890';
const HASH_B = '0x1111111111111111222222222222222233333333333333334444444444444444';
const HASH_ZERO = '0x0000000000000000000000000000000000000000000000000000000000000000';
describe('hashToHeightmap', () => {
    it('returns a Float32Array of gridSize*gridSize', () => {
        const hm = hashToHeightmap(HASH_A, 16);
        expect(hm).toBeInstanceOf(Float32Array);
        expect(hm.length).toBe(256);
    });
    it('returns 32x32 by default', () => {
        const hm = hashToHeightmap(HASH_A);
        expect(hm.length).toBe(1024);
    });
    it('all values are in [0, 1]', () => {
        const hm = hashToHeightmap(HASH_A, 32);
        for (let i = 0; i < hm.length; i++) {
            expect(hm[i]).toBeGreaterThanOrEqual(0);
            expect(hm[i]).toBeLessThanOrEqual(1);
        }
    });
    it('is deterministic: same hash always produces identical heightmap', () => {
        const hm1 = hashToHeightmap(HASH_A, 16);
        const hm2 = hashToHeightmap(HASH_A, 16);
        expect(Array.from(hm1)).toEqual(Array.from(hm2));
    });
    it('different hashes produce different heightmaps', () => {
        const hmA = hashToHeightmap(HASH_A, 16);
        const hmB = hashToHeightmap(HASH_B, 16);
        // Not every element will differ, but the arrays should not be equal
        let same = true;
        for (let i = 0; i < hmA.length; i++) {
            if (hmA[i] !== hmB[i]) {
                same = false;
                break;
            }
        }
        expect(same).toBe(false);
    });
    it('zero hash does not crash and produces valid output', () => {
        const hm = hashToHeightmap(HASH_ZERO, 16);
        expect(hm.length).toBe(256);
        for (let i = 0; i < hm.length; i++) {
            expect(hm[i]).toBeGreaterThanOrEqual(0);
            expect(hm[i]).toBeLessThanOrEqual(1);
        }
    });
    it('heightmap min is 0 and max is 1 (post-normalization)', () => {
        const hm = hashToHeightmap(HASH_A, 32);
        let min = Infinity;
        let max = -Infinity;
        for (let i = 0; i < hm.length; i++) {
            if (hm[i] < min)
                min = hm[i];
            if (hm[i] > max)
                max = hm[i];
        }
        expect(min).toBeCloseTo(0, 5);
        expect(max).toBeCloseTo(1, 5);
    });
});
describe('extractParams', () => {
    it('returns all 8 terrain parameters', () => {
        const params = extractParams(HASH_A);
        expect(params).toHaveProperty('baseElevation');
        expect(params).toHaveProperty('roughness');
        expect(params).toHaveProperty('ridgeFactor');
        expect(params).toHaveProperty('erosion');
        expect(params).toHaveProperty('hueShift');
        expect(params).toHaveProperty('saturation');
        expect(params).toHaveProperty('blendCurve');
        expect(params).toHaveProperty('features');
    });
    it('all params are in [0, 1]', () => {
        const params = extractParams(HASH_A);
        for (const value of Object.values(params)) {
            expect(value).toBeGreaterThanOrEqual(0);
            expect(value).toBeLessThanOrEqual(1);
        }
    });
    it('is deterministic', () => {
        const p1 = extractParams(HASH_A);
        const p2 = extractParams(HASH_A);
        expect(p1).toEqual(p2);
    });
    it('zero hash produces 0 for all params', () => {
        const params = extractParams(HASH_ZERO);
        for (const value of Object.values(params)) {
            expect(value).toBe(0);
        }
    });
    it('different hashes produce different params', () => {
        const pA = extractParams(HASH_A);
        const pB = extractParams(HASH_B);
        // At least one param should differ
        const keys = Object.keys(pA);
        const anyDiff = keys.some((k) => pA[k] !== pB[k]);
        expect(anyDiff).toBe(true);
    });
});
