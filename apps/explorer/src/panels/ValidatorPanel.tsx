import { useEffect, useState, useRef } from 'react';
import { client } from '@/data/rpc';
import { PanelHeader } from '@/panels/PanelHeader';
import { AnimatedNumber } from '@/lib/AnimatedNumber';

interface NodeStatusData {
  chainId: number;
  validatorIndex: number;
  validatorCount: number;
  uptimeSecs: number;
  currentView: number;
  finalizedCount: number;
  proposedCount: number;
  nullifiedCount: number;
  peerCount: number;
  isLeader: boolean;
}

function formatUptime(secs: number): string {
  if (!secs) return '--';
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

function Metric({
  label,
  value,
  color,
  large,
}: {
  label: string;
  value: React.ReactNode;
  color?: string;
  large?: boolean;
}) {
  return (
    <div style={{ padding: '10px 14px' }}>
      <div
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: '11px',
          textTransform: 'uppercase',
          letterSpacing: '0.08em',
          color: '#8a7a82',
          marginBottom: 4,
        }}
      >
        {label}
      </div>
      <div
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: large ? '18px' : '14px',
          fontWeight: large ? 600 : 400,
          color: color ?? '#d8c8c4',
          fontVariantNumeric: 'tabular-nums',
        }}
      >
        {value}
      </div>
    </div>
  );
}

function ValidatorRing({
  validatorCount,
  validatorIndex,
  leaderIndex,
  finalizationRate,
}: {
  validatorCount: number;
  validatorIndex: number;
  leaderIndex: number;
  finalizationRate: number;
}) {
  const cx = 100;
  const cy = 90;
  const r = 70;

  // Arc from 150° to 30° (top half, left to right)
  const startDeg = 150;
  const endDeg = 30;
  // Going clockwise from 150 to 30 means going through 0°: 150 -> 90 -> 30
  // We distribute across 180° of arc (from 150 to 30 going counterclockwise through top)
  const arcSpan = 180; // degrees from 150 down to 30 going counterclockwise (through top)

  const toRad = (deg: number) => (deg * Math.PI) / 180;

  const getPos = (index: number) => {
    const t = validatorCount > 1 ? index / (validatorCount - 1) : 0.5;
    // Interpolate from startDeg to endDeg going counterclockwise (decreasing degrees)
    const deg = startDeg - t * arcSpan;
    return {
      x: cx + r * Math.cos(toRad(deg)),
      y: cy - r * Math.sin(toRad(deg)),
    };
  };

  // Progress arc path: same semicircle
  const arcStart = { x: cx + r * Math.cos(toRad(startDeg)), y: cy - r * Math.sin(toRad(startDeg)) };
  const arcEnd = { x: cx + r * Math.cos(toRad(endDeg)), y: cy - r * Math.sin(toRad(endDeg)) };
  const bgArcPath = `M ${arcStart.x} ${arcStart.y} A ${r} ${r} 0 0 1 ${arcEnd.x} ${arcEnd.y}`;

  // Circumference of the 180° arc
  const arcLength = Math.PI * r; // half circumference
  const filledLength = arcLength * Math.min(1, Math.max(0, finalizationRate));

  return (
    <svg
      viewBox="0 0 200 110"
      width="100%"
      style={{ display: 'block', margin: '0 auto' }}
    >
      {/* Background arc */}
      <path
        d={bgArcPath}
        fill="none"
        stroke="var(--rd-text-ghost, #3a303a)"
        strokeWidth={3}
        strokeLinecap="round"
      />
      {/* Filled progress arc */}
      <path
        d={bgArcPath}
        fill="none"
        stroke="var(--rd-success, #7a8a78)"
        strokeWidth={3}
        strokeLinecap="round"
        strokeDasharray={`${filledLength} ${arcLength}`}
        strokeDashoffset={0}
      />

      {/* Validator nodes */}
      {Array.from({ length: validatorCount }, (_, i) => {
        const { x, y } = getPos(i);
        const isLocal = i === validatorIndex;
        const isLeader = i === leaderIndex;
        const color = isLeader ? '#dca5bd' : isLocal ? '#6abf69' : '#7a7a98';

        return (
          <g key={i}>
            {/* Glow for leader */}
            {isLeader && (
              <circle cx={x} cy={y} r={14} fill={color} fillOpacity={0.3}>
                <animate
                  attributeName="opacity"
                  values="0.4;1;0.4"
                  dur="2s"
                  repeatCount="indefinite"
                />
              </circle>
            )}
            <circle cx={x} cy={y} r={8} fill={color} />
            <text
              x={x}
              y={y + 20}
              textAnchor="middle"
              fontFamily="var(--rd-font-mono, monospace)"
              fontSize={7}
              fill={color}
              opacity={0.8}
            >
              V{i}
            </text>
          </g>
        );
      })}
    </svg>
  );
}

async function fetchNodeStatus(): Promise<NodeStatusData | null> {
  try {
    const result = await client.request({
      method: 'kora_nodeStatus' as never,
      params: [] as never,
    });
    const raw = result as Record<string, unknown>;
    return {
      chainId: Number(raw.chainId),
      validatorIndex: Number(raw.validatorIndex),
      validatorCount: Number(raw.validatorCount ?? 3),
      uptimeSecs: Number(raw.uptimeSecs ?? 0),
      currentView: Number(raw.currentView),
      finalizedCount: Number(raw.finalizedCount),
      proposedCount: Number(raw.proposedCount),
      nullifiedCount: Number(raw.nullifiedCount),
      peerCount: Number(raw.peerCount),
      isLeader: Boolean(raw.isLeader),
    };
  } catch {
    return null;
  }
}

export function ValidatorPanel() {
  const [status, setStatus] = useState<NodeStatusData | null>(null);
  const retriesRef = useRef(0);

  useEffect(() => {
    let alive = true;

    async function poll() {
      // Retry up to 5 times quickly to handle Railway load balancing
      // (some validators don't have kora_nodeStatus)
      const result = await fetchNodeStatus();
      if (!alive) return;

      if (result) {
        setStatus(result);
        retriesRef.current = 0;
      } else if (retriesRef.current < 5) {
        retriesRef.current++;
        // Quick retry in 500ms
        setTimeout(poll, 500);
        return;
      }
    }

    poll();
    const id = setInterval(poll, 4_000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);

  const liveBadge = (
    <span
      style={{
        fontSize: '10px',
        color: '#6abf69',
        display: 'flex',
        alignItems: 'center',
        gap: 4,
      }}
    >
      <span
        style={{
          width: 6,
          height: 6,
          borderRadius: '50%',
          background: '#6abf69',
          boxShadow: '0 0 6px #6abf69',
          display: 'inline-block',
        }}
      />
      LIVE
    </span>
  );

  if (!status) {
    return (
      <div>
        <PanelHeader icon="consensus" title="Consensus" />
        <div
          style={{
            padding: '20px 14px',
            fontFamily: 'var(--rd-font-mono)',
            fontSize: '12px',
            color: '#6a5a62',
            textAlign: 'center',
          }}
        >
          Connecting to validators...
        </div>
      </div>
    );
  }

  const finalizationRate =
    status.currentView > 0 ? status.finalizedCount / status.currentView : 0;

  const leaderIndex = status.currentView % (status.validatorCount || 1);

  const metricValueStyle: React.CSSProperties = {
    fontFamily: 'var(--rd-font-mono)',
    fontSize: '14px',
    fontWeight: 400,
    color: '#d8c8c4',
    fontVariantNumeric: 'tabular-nums',
  };

  const metricValueLargeStyle: React.CSSProperties = {
    fontFamily: 'var(--rd-font-mono)',
    fontSize: '18px',
    fontWeight: 600,
    color: '#dca5bd',
    fontVariantNumeric: 'tabular-nums',
  };

  return (
    <div>
      <PanelHeader icon="consensus" title="Consensus" badge={liveBadge} />

      {/* Validator ring visualization */}
      <div style={{ borderBottom: '1px solid rgba(255,255,255,0.06)', padding: '8px 0 4px' }}>
        <ValidatorRing
          validatorCount={status.validatorCount || 3}
          validatorIndex={status.validatorIndex}
          leaderIndex={leaderIndex}
          finalizationRate={finalizationRate}
        />
      </div>

      {/* Metrics */}
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: '1fr 1fr',
          gap: '1px',
          background: 'rgba(255,255,255,0.04)',
        }}
      >
        <div style={{ background: 'rgba(12, 12, 16, 0.9)' }}>
          <Metric
            label="Current View"
            value={
              <AnimatedNumber
                value={status.currentView}
                format={(n) => Math.round(n).toLocaleString()}
                style={metricValueLargeStyle}
              />
            }
            color="#dca5bd"
            large
          />
        </div>
        <div style={{ background: 'rgba(12, 12, 16, 0.9)' }}>
          <Metric
            label="Peers"
            value={
              <>
                <AnimatedNumber
                  value={status.peerCount}
                  format={(n) => Math.round(n).toLocaleString()}
                  style={metricValueLargeStyle}
                />
                {' / '}
                {status.validatorCount}
              </>
            }
            large
          />
        </div>
        <div style={{ background: 'rgba(12, 12, 16, 0.9)' }}>
          <Metric
            label="Finalized"
            value={
              <AnimatedNumber
                value={status.finalizedCount}
                format={(n) => Math.round(n).toLocaleString()}
                style={{ ...metricValueStyle, color: '#6abf69' }}
              />
            }
            color="#6abf69"
          />
        </div>
        <div style={{ background: 'rgba(12, 12, 16, 0.9)' }}>
          <Metric
            label="Finalization Rate"
            value={`${status.currentView > 0 ? ((status.finalizedCount / status.currentView) * 100).toFixed(1) : '0'}%`}
            color="#6abf69"
          />
        </div>
        <div style={{ background: 'rgba(12, 12, 16, 0.9)' }}>
          <Metric
            label="Proposed"
            value={
              <AnimatedNumber
                value={status.proposedCount}
                format={(n) => Math.round(n).toLocaleString()}
                style={metricValueStyle}
              />
            }
          />
        </div>
        <div style={{ background: 'rgba(12, 12, 16, 0.9)' }}>
          <Metric
            label="Nullified"
            value={
              <AnimatedNumber
                value={status.nullifiedCount}
                format={(n) => Math.round(n).toLocaleString()}
                style={{
                  ...metricValueStyle,
                  color: status.nullifiedCount > 0 ? '#e8a84c' : undefined,
                }}
              />
            }
            color={status.nullifiedCount > 0 ? '#e8a84c' : undefined}
          />
        </div>
        <div style={{ background: 'rgba(12, 12, 16, 0.9)' }}>
          <Metric label="Uptime" value={formatUptime(status.uptimeSecs)} />
        </div>
        <div style={{ background: 'rgba(12, 12, 16, 0.9)' }}>
          <Metric label="Chain ID" value={status.chainId} />
        </div>
      </div>
    </div>
  );
}
