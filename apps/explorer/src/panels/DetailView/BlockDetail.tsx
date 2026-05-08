import { useEffect, useState, useCallback, useRef } from 'react';
import { useNavigate, useParams } from 'react-router';
import { GlassPanel } from '@/panels/GlassPanel';
import { HashArt } from './HashArt';
import { useChainStore } from '@/data/store';
import { client } from '@/data/rpc';
import {
  truncateHash,
  formatEth,
  formatGas,
  formatTimestamp,
  formatNumber,
} from '@/lib/format';
import type { ChainBlock, ChainTransaction, Hex } from '@/data/types';
import styles from '@/design/glass.module.css';

// ---------------------------------------------------------------------------
// Data fetching hook
// ---------------------------------------------------------------------------

interface BlockDetailData {
  block: ChainBlock | null;
  loading: boolean;
  error: string | null;
}

function useBlockData(numberOrHash: string | undefined): BlockDetailData {
  const blocks = useChainStore((s) => s.blocks);
  const [block, setBlock] = useState<ChainBlock | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!numberOrHash) return;
    let cancelled = false;

    async function fetchBlock() {
      setLoading(true);
      setError(null);

      try {
        const isNumber = /^\d+$/.test(numberOrHash!);

        // Try cache first for block numbers
        if (isNumber) {
          const num = BigInt(numberOrHash!);
          const cached = blocks.get(num);
          if (cached) {
            if (!cancelled) {
              setBlock(cached);
              setLoading(false);
            }
            return;
          }
        }

        // Fetch from RPC via viem
        const result = isNumber
          ? await client.getBlock({
              blockNumber: BigInt(numberOrHash!),
              includeTransactions: true,
            })
          : await client.getBlock({
              blockHash: numberOrHash as Hex,
              includeTransactions: true,
            });

        if (!cancelled && result) {
          // Map viem block to our ChainBlock type
          const mapped: ChainBlock = {
            number: result.number,
            hash: result.hash,
            parentHash: result.parentHash,
            timestamp: result.timestamp,
            gasUsed: result.gasUsed,
            gasLimit: result.gasLimit,
            baseFeePerGas: result.baseFeePerGas ?? 0n,
            stateRoot: result.stateRoot,
            transactionsRoot: result.transactionsRoot,
            receiptsRoot: result.receiptsRoot,
            transactions: (result.transactions ?? []).map((tx) => {
              if (typeof tx === 'string') {
                return { hash: tx } as unknown as ChainTransaction;
              }
              return {
                hash: tx.hash,
                from: tx.from,
                to: tx.to ?? null,
                value: tx.value,
                gas: tx.gas,
                gasPrice: tx.gasPrice ?? 0n,
                input: tx.input,
                nonce: tx.nonce,
                blockNumber: tx.blockNumber ?? 0n,
                blockHash: tx.blockHash ?? ('0x' as Hex),
                transactionIndex: tx.transactionIndex ?? 0,
                type: tx.type === 'eip1559' ? 2 : tx.type === 'eip2930' ? 1 : 0,
              } satisfies ChainTransaction;
            }),
            _activityLevel:
              result.gasLimit > 0n
                ? Number(result.gasUsed) / Number(result.gasLimit)
                : 0,
            _arrivalTime: Date.now(),
          };
          setBlock(mapped);
        } else if (!cancelled) {
          setError('Block not found');
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : 'Failed to fetch block');
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    }

    fetchBlock();
    return () => { cancelled = true; };
  }, [numberOrHash, blocks]);

  return { block, loading, error };
}

// ---------------------------------------------------------------------------
// Copy to clipboard helper
// ---------------------------------------------------------------------------

function useCopyToClipboard() {
  const [copiedField, setCopiedField] = useState<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  const copy = useCallback((text: string, field: string) => {
    navigator.clipboard.writeText(text);
    setCopiedField(field);
    clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => setCopiedField(null), 1500);
  }, []);

  return { copiedField, copy };
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function HashField({
  label,
  hash,
  copiedField,
  onCopy,
}: {
  label: string;
  hash: Hex;
  copiedField: string | null;
  onCopy: (text: string, field: string) => void;
}) {
  const isCopied = copiedField === label;

  return (
    <div style={{ marginBottom: 'var(--rd-space-sm)' }}>
      <div className={styles.statLabel}>{label}</div>
      <div
        className={`${styles.hashRow} ${isCopied ? styles.copied : ''}`}
        onClick={() => onCopy(hash, label)}
        title={hash}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') onCopy(hash, label);
        }}
      >
        <code>{truncateHash(hash, 8)}</code>
        <span className={styles.copyBtn}>{isCopied ? 'copied' : 'copy'}</span>
      </div>
    </div>
  );
}

function GasBar({ used, limit }: { used: bigint; limit: bigint }) {
  const pct = limit > 0n ? Number((used * 100n) / limit) : 0;

  return (
    <div>
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-sm)',
        }}
      >
        <span style={{ color: 'var(--rd-text)' }}>{formatGas(used)}</span>
        <span style={{ color: 'var(--rd-text-dim)' }}>/ {formatGas(limit)}</span>
      </div>
      <div className={styles.gasBar}>
        <div className={styles.gasBarFill} style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}

function TransactionRow({
  tx,
  onClick,
}: {
  tx: ChainTransaction;
  onClick: (hash: Hex) => void;
}) {
  return (
    <div
      className={styles.listRow}
      onClick={() => onClick(tx.hash)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'Enter') onClick(tx.hash);
      }}
    >
      <code
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-sm)',
          color: 'var(--rd-bone-bright)',
          flex: '0 0 120px',
        }}
      >
        {truncateHash(tx.hash, 4)}
      </code>
      <span
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-xs)',
          color: 'var(--rd-text-dim)',
          flex: 1,
        }}
      >
        {truncateHash(tx.from, 4)} &rarr; {tx.to ? truncateHash(tx.to, 4) : 'CREATE'}
      </span>
      <span
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-sm)',
          color: 'var(--rd-bone)',
          flex: '0 0 100px',
          textAlign: 'right',
        }}
      >
        {formatEth(tx.value)} ETH
      </span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export function BlockDetail() {
  const { numberOrHash } = useParams<{ numberOrHash: string }>();
  const navigate = useNavigate();
  const { block, loading, error } = useBlockData(numberOrHash);
  const { copiedField, copy } = useCopyToClipboard();

  const goToBlock = useCallback(
    (num: bigint) => navigate(`/block/${num}`),
    [navigate],
  );

  const goToTx = useCallback(
    (hash: Hex) => navigate(`/tx/${hash}`),
    [navigate],
  );

  const handleClose = useCallback(() => navigate(-1), [navigate]);

  // Keyboard: left/right arrows for block navigation
  useEffect(() => {
    if (!block) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'ArrowLeft' && block.number > 0n) {
        goToBlock(block.number - 1n);
      } else if (e.key === 'ArrowRight') {
        goToBlock(block.number + 1n);
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [block, goToBlock]);

  if (loading) {
    return (
      <GlassPanel position="right" width="480px" title="BLOCK" onClose={handleClose}>
        <div className={styles.panelBody}>
          <span style={{ color: 'var(--rd-text-dim)' }}>Loading...</span>
        </div>
      </GlassPanel>
    );
  }

  if (error || !block) {
    return (
      <GlassPanel position="right" width="480px" title="BLOCK" onClose={handleClose}>
        <div className={styles.panelBody}>
          <span style={{ color: 'var(--rd-danger)' }}>{error ?? 'Block not found'}</span>
        </div>
      </GlassPanel>
    );
  }

  const txCount = block.transactions?.length ?? 0;

  return (
    <GlassPanel
      position="right"
      width="480px"
      title={`BLOCK ${formatNumber(block.number)}`}
      onClose={handleClose}
      active
      resizable
      ariaLabel={`Block ${block.number} detail`}
      testId="block-detail"
    >
      <div className={styles.panelBody}>
        {/* Header: block number (hero) + timestamp */}
        <div
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'baseline',
            marginBottom: 'var(--rd-space-md)',
          }}
        >
          <span
            style={{
              fontFamily: 'var(--rd-font-display)',
              fontStyle: 'italic',
              fontSize: 'var(--rd-text-hero)',
              color: 'var(--rd-bone-bright)',
              lineHeight: 1,
            }}
          >
            {formatNumber(block.number)}
          </span>
          <span
            style={{
              fontFamily: 'var(--rd-font-mono)',
              fontSize: 'var(--rd-text-xs)',
              color: 'var(--rd-text-dim)',
              letterSpacing: 'var(--rd-tracking-normal)',
            }}
          >
            {formatTimestamp(block.timestamp)}
          </span>
        </div>

        {/* Hash Art */}
        <div
          style={{
            display: 'flex',
            justifyContent: 'center',
            marginBottom: 'var(--rd-space-md)',
          }}
        >
          <HashArt hash={block.hash} size={128} />
        </div>

        {/* Hash fields (copyable) */}
        <HashField label="HASH" hash={block.hash} copiedField={copiedField} onCopy={copy} />
        <HashField label="PARENT" hash={block.parentHash} copiedField={copiedField} onCopy={copy} />
        <HashField label="STATE ROOT" hash={block.stateRoot} copiedField={copiedField} onCopy={copy} />
        <HashField label="TRANSACTIONS ROOT" hash={block.transactionsRoot} copiedField={copiedField} onCopy={copy} />
        <HashField label="RECEIPTS ROOT" hash={block.receiptsRoot} copiedField={copiedField} onCopy={copy} />

        {/* Stats grid */}
        <div className={styles.statsGrid} style={{ marginTop: 'var(--rd-space-md)' }}>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>GAS USED / LIMIT</div>
            <GasBar used={block.gasUsed} limit={block.gasLimit} />
          </div>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>TRANSACTIONS</div>
            <div className={styles.statValue}>{txCount}</div>
          </div>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>BASE FEE</div>
            <div className={styles.statValue}>
              {formatGas(block.baseFeePerGas)}{' '}
              <span style={{ color: 'var(--rd-text-dim)' }}>gwei</span>
            </div>
          </div>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>TIMESTAMP</div>
            <div className={styles.statValue}>{block.timestamp.toString()}</div>
          </div>
        </div>

        {/* Transaction list */}
        <div style={{ marginTop: 'var(--rd-space-lg)' }}>
          <div className={styles.label} style={{ marginBottom: 'var(--rd-space-sm)' }}>
            &mdash;&mdash; TRANSACTIONS
          </div>

          {txCount === 0 ? (
            <div
              style={{
                fontFamily: 'var(--rd-font-mono)',
                fontSize: 'var(--rd-text-sm)',
                color: 'var(--rd-text-ghost)',
                padding: 'var(--rd-space-md)',
                textAlign: 'center',
              }}
            >
              No transactions in this block
            </div>
          ) : (
            <div className={styles.scrollList}>
              {block.transactions.map((tx) => (
                <TransactionRow key={tx.hash} tx={tx} onClick={goToTx} />
              ))}
            </div>
          )}
        </div>

        {/* Navigation: prev/next */}
        <div
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            marginTop: 'var(--rd-space-lg)',
            paddingTop: 'var(--rd-space-md)',
            borderTop: '1px solid var(--rd-border)',
          }}
        >
          <button
            className={styles.navArrow}
            onClick={() => goToBlock(block.number - 1n)}
            disabled={block.number <= 0n}
            aria-label="Previous block"
          >
            &larr; BLOCK {formatNumber(block.number - 1n)}
          </button>
          <button
            className={styles.navArrow}
            onClick={() => goToBlock(block.number + 1n)}
            aria-label="Next block"
          >
            BLOCK {formatNumber(block.number + 1n)} &rarr;
          </button>
        </div>
      </div>
    </GlassPanel>
  );
}
