import { useEffect, useState, useCallback } from 'react';
import { useNavigate, useParams } from 'react-router';
import { GlassPanel } from '@/panels/GlassPanel';
import { client } from '@/data/rpc';
import {
  truncateHash,
  formatEth,
  formatGas,
  formatNumber,
} from '@/lib/format';
import type { ChainTransaction, ChainReceipt, ChainLog, Hex } from '@/data/types';
import styles from '@/design/glass.module.css';

// ---------------------------------------------------------------------------
// Data fetching
// ---------------------------------------------------------------------------

interface TxDetailData {
  tx: ChainTransaction | null;
  receipt: ChainReceipt | null;
  loading: boolean;
  error: string | null;
}

function useTxData(hash: string | undefined): TxDetailData {
  const [tx, setTx] = useState<ChainTransaction | null>(null);
  const [receipt, setReceipt] = useState<ChainReceipt | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!hash) return;
    let cancelled = false;

    async function fetchTx() {
      setLoading(true);
      setError(null);

      try {
        const [txResult, receiptResult] = await Promise.all([
          client.getTransaction({ hash: hash as Hex }),
          client.getTransactionReceipt({ hash: hash as Hex }).catch(() => null),
        ]);

        if (!cancelled) {
          if (txResult) {
            setTx({
              hash: txResult.hash,
              from: txResult.from,
              to: (txResult.to ?? null) as Hex | null,
              value: txResult.value,
              gas: txResult.gas,
              gasPrice: txResult.gasPrice ?? 0n,
              input: txResult.input,
              nonce: txResult.nonce,
              blockNumber: txResult.blockNumber ?? 0n,
              blockHash: txResult.blockHash ?? ('0x' as Hex),
              transactionIndex: txResult.transactionIndex ?? 0,
              type: txResult.type === 'eip1559' ? 2 : txResult.type === 'eip2930' ? 1 : 0,
            });
          } else {
            setError('Transaction not found');
          }

          if (receiptResult) {
            setReceipt({
              transactionHash: receiptResult.transactionHash,
              status: receiptResult.status === 'success' ? 'success' : 'reverted',
              blockNumber: receiptResult.blockNumber,
              blockHash: receiptResult.blockHash,
              from: receiptResult.from,
              to: (receiptResult.to ?? null) as Hex | null,
              gasUsed: receiptResult.gasUsed,
              cumulativeGasUsed: receiptResult.cumulativeGasUsed,
              contractAddress: (receiptResult.contractAddress ?? null) as Hex | null,
              logs: receiptResult.logs.map((log) => ({
                address: log.address,
                topics: log.topics as Hex[],
                data: log.data,
                blockNumber: log.blockNumber ?? 0n,
                transactionHash: log.transactionHash ?? ('0x' as Hex),
                logIndex: log.logIndex ?? 0,
              })),
            });
          }
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : 'Failed to fetch transaction');
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    }

    fetchTx();
    return () => { cancelled = true; };
  }, [hash]);

  return { tx, receipt, loading, error };
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function FlowDiagram({
  from,
  to,
  value,
  onAddressClick,
}: {
  from: Hex;
  to: Hex | null;
  value: bigint;
  onAddressClick: (addr: Hex) => void;
}) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        padding: 'var(--rd-space-md)',
        borderBottom: '1px solid var(--rd-border)',
        gap: 'var(--rd-space-sm)',
      }}
    >
      {/* Sender */}
      <div
        style={{ textAlign: 'center', cursor: 'pointer' }}
        onClick={() => onAddressClick(from)}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => { if (e.key === 'Enter') onAddressClick(from); }}
      >
        <code
          style={{
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-sm)',
            color: 'var(--rd-bone-bright)',
          }}
        >
          {truncateHash(from, 4)}
        </code>
        <div className={styles.label}>SENDER</div>
      </div>

      {/* Arrow + value */}
      <div style={{ textAlign: 'center', flex: 1 }}>
        <div
          style={{
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-sm)',
            color: 'var(--rd-bone)',
            marginBottom: 'var(--rd-space-xs)',
          }}
        >
          {formatEth(value)} ETH
        </div>
        <div style={{ height: 1, background: 'var(--rd-bone-dim)', position: 'relative' }}>
          <div
            style={{
              position: 'absolute',
              right: -4,
              top: -3,
              width: 0,
              height: 0,
              borderLeft: '6px solid var(--rd-bone-dim)',
              borderTop: '3px solid transparent',
              borderBottom: '3px solid transparent',
            }}
          />
        </div>
      </div>

      {/* Receiver */}
      <div
        style={{ textAlign: 'center', cursor: to ? 'pointer' : 'default' }}
        onClick={() => to && onAddressClick(to)}
        role="button"
        tabIndex={to ? 0 : -1}
        onKeyDown={(e) => { if (e.key === 'Enter' && to) onAddressClick(to); }}
      >
        <code
          style={{
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-sm)',
            color: to ? 'var(--rd-bone-bright)' : 'var(--rd-rose-bright)',
          }}
        >
          {to ? truncateHash(to, 4) : 'CONTRACT CREATE'}
        </code>
        <div className={styles.label}>{to ? 'RECEIVER' : 'CREATE'}</div>
      </div>
    </div>
  );
}

function StatusBadge({ status }: { status: 'success' | 'reverted' }) {
  const isSuccess = status === 'success';
  const cls = isSuccess ? styles.badgeSuccess : styles.badgeReverted;

  return (
    <span className={`${styles.badge} ${cls}`}>
      <span
        style={{
          width: 5,
          height: 5,
          borderRadius: '50%',
          background: 'currentColor',
          display: 'inline-block',
        }}
      />
      {status.toUpperCase()}
    </span>
  );
}

function LogEntry({ log, index }: { log: ChainLog; index: number }) {
  return (
    <div
      style={{
        padding: 'var(--rd-space-sm) var(--rd-space-md)',
        borderBottom: '1px solid var(--rd-border)',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--rd-space-sm)',
          marginBottom: 'var(--rd-space-xs)',
        }}
      >
        <span className={styles.label}>LOG {index}</span>
        <code
          style={{
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-xs)',
            color: 'var(--rd-text-dim)',
          }}
        >
          {truncateHash(log.address, 4)}
        </code>
      </div>

      {log.topics.map((topic, i) => (
        <div
          key={i}
          style={{
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-xs)',
            color: i === 0 ? 'var(--rd-dream)' : 'var(--rd-text-dim)',
            marginLeft: 'var(--rd-space-md)',
            marginBottom: 2,
            wordBreak: 'break-all',
          }}
        >
          [{i}] {truncateHash(topic, 8)}
        </div>
      ))}

      {log.data !== '0x' && (
        <div
          style={{
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-xs)',
            color: 'var(--rd-text-ghost)',
            marginLeft: 'var(--rd-space-md)',
            marginTop: 'var(--rd-space-xs)',
            wordBreak: 'break-all',
            maxHeight: 60,
            overflow: 'hidden',
          }}
        >
          data: {truncateHash(log.data, 16)}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export function TxDetail() {
  const { hash } = useParams<{ hash: string }>();
  const navigate = useNavigate();
  const { tx, receipt, loading, error } = useTxData(hash);

  const handleClose = useCallback(() => navigate(-1), [navigate]);

  const goToAddress = useCallback(
    (addr: Hex) => navigate(`/address/${addr}`),
    [navigate],
  );

  const goToBlock = useCallback(
    (num: bigint) => navigate(`/block/${num}`),
    [navigate],
  );

  if (loading) {
    return (
      <GlassPanel position="right" width="520px" title="TRANSACTION" onClose={handleClose}>
        <div className={styles.panelBody}>
          <span style={{ color: 'var(--rd-text-dim)' }}>Loading...</span>
        </div>
      </GlassPanel>
    );
  }

  if (error || !tx) {
    return (
      <GlassPanel position="right" width="520px" title="TRANSACTION" onClose={handleClose}>
        <div className={styles.panelBody}>
          <span style={{ color: 'var(--rd-danger)' }}>
            {error ?? 'Transaction not found'}
          </span>
        </div>
      </GlassPanel>
    );
  }

  return (
    <GlassPanel
      position="right"
      width="520px"
      title="TRANSACTION"
      onClose={handleClose}
      active
      resizable
      ariaLabel={`Transaction ${tx.hash} detail`}
      testId="tx-detail"
    >
      <div className={styles.panelBody}>
        {/* Header: hash + status */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            marginBottom: 'var(--rd-space-md)',
          }}
        >
          <code
            style={{
              fontFamily: 'var(--rd-font-mono)',
              fontSize: 'var(--rd-text-sm)',
              color: 'var(--rd-text)',
            }}
            aria-label={`Transaction hash: ${tx.hash}`}
          >
            {truncateHash(tx.hash, 8)}
          </code>
          {receipt && <StatusBadge status={receipt.status} />}
        </div>

        {/* Flow diagram: From -> To */}
        <FlowDiagram
          from={tx.from}
          to={tx.to}
          value={tx.value}
          onAddressClick={goToAddress}
        />

        {/* Data grid */}
        <div className={styles.statsGrid} style={{ marginTop: 'var(--rd-space-md)' }}>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>VALUE</div>
            <div className={styles.statValue}>{formatEth(tx.value)} ETH</div>
          </div>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>GAS PRICE</div>
            <div className={styles.statValue}>{formatGas(tx.gasPrice)} gwei</div>
          </div>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>GAS LIMIT</div>
            <div className={styles.statValue}>{formatGas(tx.gas)}</div>
          </div>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>GAS USED</div>
            <div className={styles.statValue}>
              {receipt ? formatGas(receipt.gasUsed) : '--'}
            </div>
          </div>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>NONCE</div>
            <div className={styles.statValue}>{tx.nonce}</div>
          </div>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>TYPE</div>
            <div className={styles.statValue}>
              0x{tx.type.toString(16)}{' '}
              <span style={{ color: 'var(--rd-text-dim)' }}>
                ({tx.type === 2 ? '1559' : tx.type === 1 ? '2930' : 'legacy'})
              </span>
            </div>
          </div>
        </div>

        {/* Gas usage bar */}
        {receipt && (
          <div style={{ margin: 'var(--rd-space-md) 0' }}>
            <div className={styles.statLabel}>GAS USAGE</div>
            <div className={styles.gasBar} style={{ marginTop: 'var(--rd-space-xs)' }}>
              <div
                className={styles.gasBarFill}
                style={{
                  width: `${tx.gas > 0n ? Number((receipt.gasUsed * 100n) / tx.gas) : 0}%`,
                }}
              />
            </div>
          </div>
        )}

        {/* Input data */}
        {tx.input && tx.input !== '0x' && (
          <div style={{ marginTop: 'var(--rd-space-md)' }}>
            <div className={styles.statLabel}>INPUT DATA</div>
            <div
              style={{
                fontFamily: 'var(--rd-font-mono)',
                fontSize: 'var(--rd-text-xs)',
                color: 'var(--rd-text-dim)',
                background: 'var(--rd-void-light)',
                padding: 'var(--rd-space-sm)',
                marginTop: 'var(--rd-space-xs)',
                maxHeight: 120,
                overflow: 'auto',
                wordBreak: 'break-all',
              }}
            >
              {tx.input}
            </div>
          </div>
        )}

        {/* Block link */}
        <div
          style={{
            marginTop: 'var(--rd-space-md)',
            paddingTop: 'var(--rd-space-md)',
            borderTop: '1px solid var(--rd-border)',
          }}
        >
          <div className={styles.statLabel}>BLOCK</div>
          <button
            onClick={() => goToBlock(tx.blockNumber)}
            style={{
              background: 'none',
              border: 'none',
              color: 'var(--rd-rose)',
              cursor: 'pointer',
              fontFamily: 'var(--rd-font-mono)',
              fontSize: 'var(--rd-text-base)',
              padding: 0,
              textDecoration: 'none',
            }}
          >
            {formatNumber(tx.blockNumber)}
          </button>
        </div>

        {/* Receipt logs */}
        {receipt && receipt.logs.length > 0 && (
          <div style={{ marginTop: 'var(--rd-space-lg)' }}>
            <div className={styles.label} style={{ marginBottom: 'var(--rd-space-sm)' }}>
              &mdash;&mdash; LOGS ({receipt.logs.length})
            </div>
            <div className={styles.scrollList}>
              {receipt.logs.map((log, i) => (
                <LogEntry key={`${log.transactionHash}-${log.logIndex}`} log={log} index={i} />
              ))}
            </div>
          </div>
        )}
      </div>
    </GlassPanel>
  );
}
