import { useChainStore, useUIStore } from '@/data/store';
import { kora } from '@/data/rpc';
import { formatBlockNumber } from '@/lib/format';
import { FeeWaveform } from './FeeWaveform';
import { ActivityPulse } from './ActivityPulse';
import { NetworkHealth } from './NetworkHealth';
import { RpcLatency } from './RpcLatency';
import styles from '@/design/glass.module.css';

// ---------------------------------------------------------------------------
// Connection LED
// ---------------------------------------------------------------------------

function ConnectionLed() {
  const connectionState = useChainStore((s) => s.connectionState);

  const ledColor =
    connectionState === 'connected'
      ? 'var(--rd-led-connected)'
      : connectionState === 'reconnecting'
        ? 'var(--rd-led-warning)'
        : 'var(--rd-led-error)';

  const label =
    connectionState === 'connected'
      ? 'CONNECTED'
      : connectionState === 'reconnecting'
        ? 'RECONNECTING'
        : 'DISCONNECTED';

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 'var(--rd-space-xs)',
      }}
      aria-live="polite"
    >
      <span
        className={styles.led}
        style={{
          background: ledColor,
          boxShadow: `0 0 6px ${ledColor}`,
          animation:
            connectionState === 'reconnecting'
              ? 'pulse 1.2s ease-in-out infinite'
              : undefined,
        }}
      />
      <span className={styles.label}>{label}</span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Chain ID badge
// ---------------------------------------------------------------------------

function ChainIdBadge() {
  return (
    <span
      style={{
        fontFamily: 'var(--rd-font-mono)',
        fontSize: 'var(--rd-text-xs)',
        color: 'var(--rd-text-dim)',
        letterSpacing: 'var(--rd-tracking-normal)',
      }}
    >
      {kora.id}
    </span>
  );
}

// ---------------------------------------------------------------------------
// Block counter with pulse on new block
// ---------------------------------------------------------------------------

function BlockCounter() {
  const latestBlockNum = useChainStore((s) => s.latestBlock);
  const blocks = useChainStore((s) => s.blocks);
  const latestBlock = latestBlockNum > 0n ? blocks.get(latestBlockNum) : undefined;

  let blockTimeLabel = '';
  if (latestBlock && latestBlock._arrivalTime) {
    const elapsed = Date.now() - latestBlock._arrivalTime;
    if (elapsed < 60_000) {
      blockTimeLabel = `${Math.round(elapsed / 1000)}s ago`;
    }
  }

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 'var(--rd-space-sm)',
      }}
    >
      <span
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-sm)',
          color: 'var(--rd-rose)',
          letterSpacing: 'var(--rd-tracking-normal)',
          fontVariantNumeric: 'tabular-nums',
        }}
      >
        #{formatBlockNumber(latestBlockNum)}
      </span>
      {blockTimeLabel && (
        <span
          style={{
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-xs)',
            color: 'var(--rd-text-dim)',
          }}
        >
          {blockTimeLabel}
        </span>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Gas price display
// ---------------------------------------------------------------------------

function GasPrice() {
  const latestBlockNum = useChainStore((s) => s.latestBlock);
  const blocks = useChainStore((s) => s.blocks);
  const latestBlock = latestBlockNum > 0n ? blocks.get(latestBlockNum) : undefined;

  if (!latestBlock) return null;

  const gweiStr = (Number(latestBlock.baseFeePerGas) / 1e9).toFixed(2);

  return (
    <span
      style={{
        fontFamily: 'var(--rd-font-mono)',
        fontSize: 'var(--rd-text-xs)',
        color: 'var(--rd-warning)',
        fontVariantNumeric: 'tabular-nums',
      }}
    >
      {gweiStr}{' '}
      <span style={{ color: 'var(--rd-text-dim)' }}>gwei</span>
    </span>
  );
}

// ---------------------------------------------------------------------------
// Peer count (from consensus store node status updates)
// ---------------------------------------------------------------------------

function PeerCount() {
  // Peer count comes through consensus store's updateFromNodeStatus;
  // the validator count serves as a proxy when full peer count isn't exposed.
  const validatorCount = useChainStore(() => 0); // placeholder

  if (!validatorCount) return null;

  return (
    <span
      style={{
        fontFamily: 'var(--rd-font-mono)',
        fontSize: 'var(--rd-text-xs)',
        color: 'var(--rd-text-dim)',
        fontVariantNumeric: 'tabular-nums',
      }}
    >
      {validatorCount}{' '}
      <span style={{ letterSpacing: 'var(--rd-tracking-label)' }}>PEERS</span>
    </span>
  );
}

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export function StatusBar() {
  const ambientMode = useUIStore((s) => s.ambientMode);

  return (
    <div
      style={{
        position: 'fixed',
        left: 0,
        right: 0,
        top: 0,
        height: 48,
        background: 'var(--rd-void-surface)',
        borderBottom: '1px solid var(--rd-border)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        padding: '0 var(--rd-space-md)',
        gap: 'var(--rd-space-md)',
        zIndex: 'var(--rd-z-panels)' as unknown as number,
        opacity: ambientMode ? 0.2 : 1,
        transition: 'opacity 2s ease-out',
        pointerEvents: 'auto',
      }}
      role="status"
      aria-label="Explorer status bar"
    >
      {/* Left: connection + chain ID */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--rd-space-sm)',
        }}
      >
        <ConnectionLed />
        <ChainIdBadge />
      </div>

      {/* Center: block number + fee waveform + gas price */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--rd-space-md)',
        }}
      >
        <BlockCounter />
        <ActivityPulse />
        <FeeWaveform width={120} height={24} />
        <GasPrice />
      </div>

      {/* Right: peer count */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--rd-space-md)' }}>
        <NetworkHealth />
        <RpcLatency />
        <PeerCount />
      </div>
    </div>
  );
}
