import type { AddressNode } from './nodes';

// ── Arc Data ─────────────────────────────────────────────────

export interface ArcData {
  fromX: number;
  fromY: number;
  toX: number;
  toY: number;
  cpX: number; // bezier control point
  cpY: number;
  segments: number;
  progress: number; // 0..1 (traveling), >1 (ghost phase)
  opacity: number;
  birthTime: number;
  status: 'pending' | 'confirmed' | 'reverted';
  txHash: string;
}

// ── Constants ────────────────────────────────────────────────

const ARC_TRAVEL_DURATION = 2.0; // seconds
const ARC_GHOST_DURATION = 30.0; // seconds after completion
const PENDING_PULSE_SPEED = 3.0; // rad/s

// ── Arc computation ──────────────────────────────────────────

/**
 * Compute a quadratic bezier arc between two address nodes.
 * The control point is offset perpendicular to the sender-receiver line,
 * magnitude proportional to tx value (log scale).
 */
export function computeArc(
  from: AddressNode,
  to: AddressNode,
  value: bigint,
  txHash: string,
  status: 'pending' | 'confirmed' | 'reverted',
): ArcData {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const dist = Math.hypot(dx, dy);

  // Perpendicular offset proportional to value
  const valueEth = Number(value) / 1e18;
  const curvature = Math.min(0.4, 0.05 + Math.log(1 + valueEth) * 0.1);

  // Midpoint
  const midX = (from.x + to.x) / 2;
  const midY = (from.y + to.y) / 2;

  // Perpendicular direction
  const safeDist = Math.max(dist, 0.001);
  const perpX = (-dy / safeDist) * curvature;
  const perpY = (dx / safeDist) * curvature;

  return {
    fromX: from.x,
    fromY: from.y,
    toX: to.x,
    toY: to.y,
    cpX: midX + perpX,
    cpY: midY + perpY,
    segments: Math.max(8, Math.min(32, Math.round(8 + valueEth * 2))),
    progress: 0,
    opacity: 1,
    birthTime: performance.now(),
    status,
    txHash,
  };
}

// ── Bezier evaluation ────────────────────────────────────────

/** Evaluate a quadratic bezier at parameter t in [0,1]. */
export function bezierPoint(
  arc: ArcData,
  t: number,
): { x: number; y: number } {
  const u = 1 - t;
  return {
    x: u * u * arc.fromX + 2 * u * t * arc.cpX + t * t * arc.toX,
    y: u * u * arc.fromY + 2 * u * t * arc.cpY + t * t * arc.toY,
  };
}

// ── ArcManager ───────────────────────────────────────────────

const MAX_ARCS = 256;

export class ArcManager {
  private arcs: ArcData[] = [];
  private readonly maxArcs: number;

  constructor(maxArcs = MAX_ARCS) {
    this.maxArcs = maxArcs;
  }

  addArc(
    from: AddressNode,
    to: AddressNode,
    value: bigint,
    txHash: string,
    status: 'pending' | 'confirmed' | 'reverted',
  ): ArcData {
    const arc = computeArc(from, to, value, txHash, status);

    if (this.arcs.length >= this.maxArcs) {
      // Remove oldest completed arc
      const idx = this.arcs.findIndex(
        (a) => a.progress >= 1 && a.status !== 'pending',
      );
      if (idx >= 0) {
        this.arcs.splice(idx, 1);
      } else {
        this.arcs.shift();
      }
    }

    this.arcs.push(arc);
    return arc;
  }

  resolveArc(txHash: string, status: 'confirmed' | 'reverted'): void {
    const arc = this.arcs.find((a) => a.txHash === txHash);
    if (!arc) return;
    arc.status = status;
    if (status === 'confirmed') {
      arc.progress = 0;
    }
  }

  /** Advance all arcs. Call every frame. */
  update(dt: number): void {
    const now = performance.now();

    for (let i = this.arcs.length - 1; i >= 0; i--) {
      const arc = this.arcs[i];

      if (arc.status === 'pending') {
        // Pulse at sender position
        const pulse = Math.sin(now * 0.001 * PENDING_PULSE_SPEED) * 0.5 + 0.5;
        arc.opacity = 0.5 + pulse * 0.5;
        continue;
      }

      if (arc.progress < 1) {
        // Particle is traveling
        arc.progress = Math.min(1, arc.progress + dt / ARC_TRAVEL_DURATION);
      } else {
        // Ghost phase: fade out
        const ageAfterTravel =
          (now - arc.birthTime) / 1000 - ARC_TRAVEL_DURATION;
        const fadeProgress = Math.min(ageAfterTravel / ARC_GHOST_DURATION, 1);
        arc.opacity = Math.max(0, 1 - fadeProgress);

        if (arc.opacity <= 0) {
          this.arcs.splice(i, 1);
        }
      }
    }
  }

  getArcs(): readonly ArcData[] {
    return this.arcs;
  }

  get activeCount(): number {
    return this.arcs.length;
  }
}
