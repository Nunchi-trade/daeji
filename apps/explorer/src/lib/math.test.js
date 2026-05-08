import { describe, it, expect } from "vitest";
import { clamp, lerp, smoothstep, mulberry32, perlin2D } from "./math";
describe("clamp", () => {
    it("clamps below minimum", () => {
        expect(clamp(-5, 0, 10)).toBe(0);
    });
    it("clamps above maximum", () => {
        expect(clamp(15, 0, 10)).toBe(10);
    });
    it("returns value when in range", () => {
        expect(clamp(5, 0, 10)).toBe(5);
    });
    it("returns min when equal to min", () => {
        expect(clamp(0, 0, 10)).toBe(0);
    });
    it("returns max when equal to max", () => {
        expect(clamp(10, 0, 10)).toBe(10);
    });
});
describe("lerp", () => {
    it("returns a at t=0", () => {
        expect(lerp(10, 20, 0)).toBe(10);
    });
    it("returns b at t=1", () => {
        expect(lerp(10, 20, 1)).toBe(20);
    });
    it("returns midpoint at t=0.5", () => {
        expect(lerp(0, 100, 0.5)).toBe(50);
    });
    it("extrapolates beyond t=1", () => {
        expect(lerp(0, 10, 2)).toBe(20);
    });
});
describe("smoothstep", () => {
    it("returns 0 below edge0", () => {
        expect(smoothstep(0, 1, -0.5)).toBe(0);
    });
    it("returns 1 above edge1", () => {
        expect(smoothstep(0, 1, 1.5)).toBe(1);
    });
    it("returns 0 at edge0", () => {
        expect(smoothstep(0, 1, 0)).toBe(0);
    });
    it("returns 1 at edge1", () => {
        expect(smoothstep(0, 1, 1)).toBe(1);
    });
    it("returns 0.5 at midpoint", () => {
        expect(smoothstep(0, 1, 0.5)).toBe(0.5);
    });
    it("is monotonically increasing", () => {
        const values = Array.from({ length: 11 }, (_, i) => smoothstep(0, 1, i / 10));
        for (let i = 1; i < values.length; i++) {
            expect(values[i]).toBeGreaterThanOrEqual(values[i - 1]);
        }
    });
});
describe("mulberry32", () => {
    it("produces deterministic values from same seed", () => {
        const a = mulberry32(42);
        const b = mulberry32(42);
        expect(a()).toBe(b());
        expect(a()).toBe(b());
        expect(a()).toBe(b());
    });
    it("produces values in [0, 1)", () => {
        const rng = mulberry32(123);
        for (let i = 0; i < 100; i++) {
            const v = rng();
            expect(v).toBeGreaterThanOrEqual(0);
            expect(v).toBeLessThan(1);
        }
    });
    it("produces different values from different seeds", () => {
        const a = mulberry32(1);
        const b = mulberry32(2);
        // Very unlikely to be equal
        expect(a()).not.toBe(b());
    });
});
describe("perlin2D", () => {
    it("returns values in approximately [-1, 1]", () => {
        for (let x = 0; x < 10; x++) {
            for (let y = 0; y < 10; y++) {
                const v = perlin2D(x * 0.1, y * 0.1);
                expect(v).toBeGreaterThanOrEqual(-1);
                expect(v).toBeLessThanOrEqual(1);
            }
        }
    });
    it("returns 0 at integer coordinates", () => {
        // Perlin noise gradients at integer boundaries produce 0
        expect(perlin2D(0, 0)).toBe(0);
        expect(perlin2D(1, 1)).toBe(0);
        expect(perlin2D(5, 5)).toBe(0);
    });
    it("is deterministic", () => {
        expect(perlin2D(3.7, 2.1)).toBe(perlin2D(3.7, 2.1));
    });
    it("produces non-zero values at non-integer coordinates", () => {
        // At least some non-integer coordinates should produce non-zero
        const v = perlin2D(0.5, 0.5);
        expect(v).not.toBe(0);
    });
});
