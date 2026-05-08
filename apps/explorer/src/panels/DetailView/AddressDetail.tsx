import { useEffect, useState, useCallback } from 'react';
import { useNavigate, useParams } from 'react-router';
import { GlassPanel } from '@/panels/GlassPanel';
import { useChainStore } from '@/data/store';
import { client } from '@/data/rpc';
import {
  truncateHash,
  formatEth,
  formatGas,
  formatNumber,
} from '@/lib/format';
import type { ChainTransaction, Hex } from '@/data/types';
import styles from '@/design/glass.module.css';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface AddressInfo {
  address: Hex;
  balance: bigint;
  isContract: boolean;
  codeSize: number;
  recentTxs: ChainTransaction[];
  loading: boolean;
  error: string | null;
}

// ---------------------------------------------------------------------------
// Data fetching
// ---------------------------------------------------------------------------

function useAddressData(address: string | undefined): AddressInfo {
  const blocks = useChainStore((s) => s.blocks);
  const [info, setInfo] = useState<AddressInfo>({
    address: (address ?? '0x') as Hex,
    balance: 0n,
    isContract: false,
    codeSize: 0,
    recentTxs: [],
    loading: true,
    error: null,
  });

  useEffect(() => {
    if (!address) return;
    let cancelled = false;

    async function fetchAddress() {
      setInfo((prev) => ({ ...prev, loading: true, error: null }));

      try {
        const [balance, code] = await Promise.all([
          client.getBalance({ address: address as Hex }),
          client.getCode({ address: address as Hex }),
        ]);

        // Scan cached blocks for recent transactions involving this address
        const recentTxs: ChainTransaction[] = [];
        const lowerAddr = address!.toLowerCase();

        for (const block of blocks.values()) {
          if (!block.transactions) continue;
          for (const tx of block.transactions) {
            if (
              tx.from.toLowerCase() === lowerAddr ||
              (tx.to && tx.to.toLowerCase() === lowerAddr)
            ) {
              recentTxs.push(tx);
            }
          }
          if (recentTxs.length >= 10) break;
        }

        recentTxs.sort((a, b) => {
          if (b.blockNumber > a.blockNumber) return 1;
          if (b.blockNumber < a.blockNumber) return -1;
          return b.transactionIndex - a.transactionIndex;
        });

        const isContract = !!code && code !== '0x';
        const codeSize = isContract ? (code!.length - 2) / 2 : 0;

        if (!cancelled) {
          setInfo({
            address: address as Hex,
            balance,
            isContract,
            codeSize,
            recentTxs: recentTxs.slice(0, 10),
            loading: false,
            error: null,
          });
        }
      } catch (err) {
        if (!cancelled) {
          setInfo((prev) => ({
            ...prev,
            loading: false,
            error: err instanceof Error ? err.message : 'Failed to fetch address',
          }));
        }
      }
    }

    fetchAddress();
    return () => { cancelled = true; };
  }, [address, blocks]);

  return info;
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function TypeBadge({ isContract }: { isContract: boolean }) {
  const cls = isContract ? styles.badgeContract : styles.badgeEoa;
  return (
    <span className={`${styles.badge} ${cls}`}>
      {isContract ? 'CONTRACT' : 'EOA'}
    </span>
  );
}

function TxRow({
  tx,
  highlightAddr,
  onTxClick,
}: {
  tx: ChainTransaction;
  highlightAddr: string;
  onTxClick: (hash: Hex) => void;
}) {
  const isSender = tx.from.toLowerCase() === highlightAddr.toLowerCase();

  return (
    <div
      className={styles.listRow}
      onClick={() => onTxClick(tx.hash)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => { if (e.key === 'Enter') onTxClick(tx.hash); }}
    >
      <code
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-xs)',
          color: 'var(--rd-text-dim)',
          flex: '0 0 80px',
        }}
      >
        {truncateHash(tx.hash, 3)}
      </code>
      <span
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-xs)',
          color: isSender ? 'var(--rd-danger)' : 'var(--rd-success)',
          flex: '0 0 30px',
        }}
      >
        {isSender ? 'OUT' : 'IN'}
      </span>
      <span
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-xs)',
          color: 'var(--rd-text-dim)',
          flex: 1,
        }}
      >
        {isSender
          ? tx.to
            ? truncateHash(tx.to, 3)
            : 'CREATE'
          : truncateHash(tx.from, 3)}
      </span>
      <span
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-sm)',
          color: 'var(--rd-bone)',
          flex: '0 0 90px',
          textAlign: 'right',
        }}
      >
        {formatEth(tx.value)} ETH
      </span>
      <span
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-xs)',
          color: 'var(--rd-text-ghost)',
          flex: '0 0 60px',
          textAlign: 'right',
        }}
      >
        {formatGas(tx.gas)}
      </span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export function AddressDetail() {
  const { address } = useParams<{ address: string }>();
  const navigate = useNavigate();
  const info = useAddressData(address);

  const handleClose = useCallback(() => navigate(-1), [navigate]);

  const goToTx = useCallback(
    (hash: Hex) => navigate(`/tx/${hash}`),
    [navigate],
  );

  if (info.loading) {
    return (
      <GlassPanel position="right" width="480px" title="ADDRESS" onClose={handleClose}>
        <div className={styles.panelBody}>
          <span style={{ color: 'var(--rd-text-dim)' }}>Loading...</span>
        </div>
      </GlassPanel>
    );
  }

  if (info.error) {
    return (
      <GlassPanel position="right" width="480px" title="ADDRESS" onClose={handleClose}>
        <div className={styles.panelBody}>
          <span style={{ color: 'var(--rd-danger)' }}>{info.error}</span>
        </div>
      </GlassPanel>
    );
  }

  return (
    <GlassPanel
      position="right"
      width="480px"
      title="ADDRESS"
      onClose={handleClose}
      active
      resizable
      ariaLabel={`Address ${info.address} detail`}
      testId="address-detail"
    >
      <div className={styles.panelBody}>
        {/* Header: address + type badge */}
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
              wordBreak: 'break-all',
            }}
            aria-label={`Address: ${info.address}`}
          >
            {info.address}
          </code>
          <div style={{ marginLeft: 'var(--rd-space-sm)', flexShrink: 0 }}>
            <TypeBadge isContract={info.isContract} />
          </div>
        </div>

        {/* Balance (hero number) */}
        <div style={{ marginBottom: 'var(--rd-space-lg)' }}>
          <div className={styles.statLabel}>BALANCE</div>
          <div
            style={{
              fontFamily: 'var(--rd-font-display)',
              fontStyle: 'italic',
              fontSize: 'var(--rd-text-hero)',
              color: 'var(--rd-bone-bright)',
              lineHeight: 1.1,
            }}
          >
            {formatEth(info.balance)}{' '}
            <span
              style={{
                fontSize: 'var(--rd-text-lg)',
                color: 'var(--rd-text-dim)',
              }}
            >
              ETH
            </span>
          </div>
        </div>

        {/* Contract info (if contract) */}
        {info.isContract && (
          <div className={styles.statsGrid} style={{ marginBottom: 'var(--rd-space-md)' }}>
            <div className={styles.statCell}>
              <div className={styles.statLabel}>CODE SIZE</div>
              <div className={styles.statValue}>
                {formatNumber(BigInt(info.codeSize))}{' '}
                <span style={{ color: 'var(--rd-text-dim)' }}>bytes</span>
              </div>
            </div>
            <div className={styles.statCell}>
              <div className={styles.statLabel}>TYPE</div>
              <div className={styles.statValue} style={{ color: 'var(--rd-rose)' }}>
                Contract
              </div>
            </div>
          </div>
        )}

        {/* Recent transactions */}
        <div style={{ marginTop: 'var(--rd-space-md)' }}>
          <div className={styles.label} style={{ marginBottom: 'var(--rd-space-sm)' }}>
            &mdash;&mdash; RECENT TRANSACTIONS
          </div>

          {info.recentTxs.length === 0 ? (
            <div
              style={{
                fontFamily: 'var(--rd-font-mono)',
                fontSize: 'var(--rd-text-sm)',
                color: 'var(--rd-text-ghost)',
                padding: 'var(--rd-space-md)',
                textAlign: 'center',
              }}
            >
              No transactions found in cached blocks
            </div>
          ) : (
            <div className={styles.scrollList}>
              {/* Table header */}
              <div
                style={{
                  display: 'flex',
                  padding: 'var(--rd-space-xs) var(--rd-space-md)',
                  borderBottom: '1px solid var(--rd-border-strong)',
                }}
              >
                <span className={styles.label} style={{ flex: '0 0 80px' }}>HASH</span>
                <span className={styles.label} style={{ flex: '0 0 30px' }}>DIR</span>
                <span className={styles.label} style={{ flex: 1 }}>TO/FROM</span>
                <span className={styles.label} style={{ flex: '0 0 90px', textAlign: 'right' }}>VALUE</span>
                <span className={styles.label} style={{ flex: '0 0 60px', textAlign: 'right' }}>GAS</span>
              </div>
              {info.recentTxs.map((tx) => (
                <TxRow
                  key={tx.hash}
                  tx={tx}
                  highlightAddr={info.address}
                  onTxClick={goToTx}
                />
              ))}
            </div>
          )}
        </div>
      </div>
    </GlassPanel>
  );
}
