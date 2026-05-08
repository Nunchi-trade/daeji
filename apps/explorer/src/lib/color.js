/**
 * Color utilities for the Kora Explorer visualizations.
 */
import { clamp, lerp } from "./math";
/** Derive a hue angle [0, 360) from a hex block hash. */
export function hashToHue(hash) {
    const clean = hash.startsWith("0x") ? hash.slice(2) : hash;
    // Use the last 6 hex chars to derive a hue
    const segment = clean.slice(-6);
    const num = parseInt(segment, 16);
    return num % 360;
}
/** Rose palette: interpolates between deep rose and soft pink. */
export function roseGradient(t) {
    const tc = clamp(t, 0, 1);
    // Deep rose: hsl(340, 70%, 40%) -> Soft pink: hsl(350, 60%, 75%)
    const h = lerp(340, 350, tc);
    const s = lerp(70, 60, tc);
    const l = lerp(40, 75, tc);
    return `hsl(${h.toFixed(0)}, ${s.toFixed(0)}%, ${l.toFixed(0)}%)`;
}
/** Map gas usage ratio [0,1] to a color from green (low) through yellow to red (high). */
export function activityColor(gasRatio) {
    const r = clamp(gasRatio, 0, 1);
    // 0 -> hsl(120, 60%, 50%) green
    // 0.5 -> hsl(60, 70%, 50%) yellow
    // 1 -> hsl(0, 80%, 50%) red
    const h = lerp(120, 0, r);
    const s = lerp(60, 80, r);
    return `hsl(${h.toFixed(0)}, ${s.toFixed(0)}%, 50%)`;
}
