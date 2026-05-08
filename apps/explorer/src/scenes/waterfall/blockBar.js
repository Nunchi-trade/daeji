import { hashToBarcode } from './hashbar';
/** Layout constants */
export const BAR_HEIGHT = 24; // px, logical (before DPR)
export const BAR_GAP = 2; // px between bars
const LABEL_PADDING = 8; // px from bar edge to label text
const NUMBER_WIDTH = 72; // px reserved for block number label
/** ROSEDUST color palette for gas interpolation */
const COLOR_EMPTY = [0x12, 0x11, 0x1a]; // --rd-void-surface
const COLOR_LOW = [0x3a, 0x20, 0x30]; // --rd-rose-deep
const COLOR_MID = [0xaa, 0x70, 0x88]; // --rd-rose
const COLOR_FULL = [0xdc, 0xa5, 0xbd]; // --rd-rose-glow
function lerpChannel(a, b, t) {
    return a + (b - a) * Math.max(0, Math.min(1, t));
}
/**
 * Interpolate between ROSEDUST colors based on gas ratio.
 *
 * 0.00       -> void-surface (empty block, barely visible)
 * 0.00-0.33  -> void-surface to rose-deep
 * 0.33-0.66  -> rose-deep to rose
 * 0.66-1.00  -> rose to rose-glow
 */
export function gasToColor(gasRatio) {
    let r, g, b;
    if (gasRatio < 0.33) {
        const t = gasRatio / 0.33;
        r = lerpChannel(COLOR_EMPTY[0], COLOR_LOW[0], t);
        g = lerpChannel(COLOR_EMPTY[1], COLOR_LOW[1], t);
        b = lerpChannel(COLOR_EMPTY[2], COLOR_LOW[2], t);
    }
    else if (gasRatio < 0.66) {
        const t = (gasRatio - 0.33) / 0.33;
        r = lerpChannel(COLOR_LOW[0], COLOR_MID[0], t);
        g = lerpChannel(COLOR_LOW[1], COLOR_MID[1], t);
        b = lerpChannel(COLOR_LOW[2], COLOR_MID[2], t);
    }
    else {
        const t = (gasRatio - 0.66) / 0.34;
        r = lerpChannel(COLOR_MID[0], COLOR_FULL[0], t);
        g = lerpChannel(COLOR_MID[1], COLOR_FULL[1], t);
        b = lerpChannel(COLOR_MID[2], COLOR_FULL[2], t);
    }
    return `rgb(${Math.round(r)}, ${Math.round(g)}, ${Math.round(b)})`;
}
/**
 * Draw a single block bar on the waterfall canvas.
 *
 * Layout:
 * +--------+--------------------------------------+--------+
 * | #286401| barcode fill                         |  3 txn |
 * +--------+--------------------------------------+--------+
 *  <- number  <- gas-proportional width (barcode inside) -> tx count ->
 */
export function drawBlockBar(ctx, block, y, canvasWidth) {
    const gasLimit = Number(block.gasLimit);
    const gasUsed = Number(block.gasUsed);
    const gasRatio = gasLimit > 0 ? gasUsed / gasLimit : 0;
    const txCount = block.transactions.length;
    // Bar dimensions
    const barAreaWidth = canvasWidth - NUMBER_WIDTH - 56; // 56px right label area
    const barWidth = Math.max(4, gasRatio * barAreaWidth); // minimum 4px even for empty
    const barX = NUMBER_WIDTH;
    // 1. Background: full-width subtle row stripe (alternating)
    const blockNum = Number(block.number);
    if (blockNum % 2 === 0) {
        ctx.fillStyle = 'rgba(255, 255, 255, 0.015)';
        ctx.fillRect(0, y, canvasWidth, BAR_HEIGHT);
    }
    // 2. Gas bar fill
    ctx.fillStyle = gasToColor(gasRatio);
    ctx.fillRect(barX, y + 1, barWidth, BAR_HEIGHT - 2);
    // 3. Hash barcode overlay (inside the bar)
    if (barWidth > 20) {
        hashToBarcode(block.hash, barWidth, BAR_HEIGHT - 2, ctx, barX, y + 1);
    }
    // 4. Border: 1px bottom edge
    ctx.fillStyle = 'rgba(255, 255, 255, 0.04)';
    ctx.fillRect(barX, y + BAR_HEIGHT - 1, barWidth, 1);
    // 5. Left accent bar (2px) - rose-glow if block has transactions
    if (txCount > 0) {
        ctx.fillStyle = '#dca5bd'; // --rd-rose-glow
        ctx.fillRect(barX, y + 1, 2, BAR_HEIGHT - 2);
    }
    // 6. Left label: block number (monospace)
    ctx.font = '10px "JetBrains Mono", monospace';
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    ctx.fillStyle = '#6a5a68'; // --rd-text-dim
    ctx.fillText(blockNum.toLocaleString(), NUMBER_WIDTH - LABEL_PADDING, y + BAR_HEIGHT / 2);
    // 7. Right label: tx count (only if > 0)
    if (txCount > 0) {
        ctx.textAlign = 'left';
        ctx.fillStyle = '#d8c8a0'; // --rd-bone-bright
        ctx.fillText(`${txCount} tx`, barX + barWidth + LABEL_PADDING, y + BAR_HEIGHT / 2);
    }
    // 8. Gas percentage (inside bar, right-aligned, if bar is wide enough)
    if (barWidth > 80) {
        ctx.textAlign = 'right';
        ctx.fillStyle = 'rgba(255, 255, 255, 0.4)';
        ctx.fillText(`${(gasRatio * 100).toFixed(1)}%`, barX + barWidth - 6, y + BAR_HEIGHT / 2);
    }
}
/**
 * Draw a highlighted version of a block bar (on hover).
 * Same layout but brighter colors and a border.
 */
export function drawBlockBarHighlight(ctx, _block, y, canvasWidth, barWidth) {
    const barX = NUMBER_WIDTH;
    // Highlight background
    ctx.fillStyle = 'rgba(170, 112, 136, 0.08)';
    ctx.fillRect(0, y, canvasWidth, BAR_HEIGHT);
    // Highlight border
    ctx.strokeStyle = 'rgba(170, 112, 136, 0.3)';
    ctx.lineWidth = 1;
    ctx.strokeRect(barX - 0.5, y + 0.5, barWidth + 1, BAR_HEIGHT - 1);
}
