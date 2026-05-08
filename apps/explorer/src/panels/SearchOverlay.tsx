import {
  useState,
  useCallback,
  useEffect,
  useRef,
  useMemo,
} from 'react';
import { useNavigate } from 'react-router';
import { motion, AnimatePresence } from 'framer-motion';
import { useChainStore, useUIStore } from '@/data/store';
import { client } from '@/data/rpc';
import { truncateHash, formatEth, formatNumber } from '@/lib/format';
import type { Hex } from '@/data/types';
import styles from '@/design/glass.module.css';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface SearchResult {
  type: 'block' | 'transaction' | 'address';
  label: string;
  subtitle: string;
  source: 'cache' | 'rpc';
  navigateTo: string;
}

const RECENT_SEARCHES_KEY = 'kora-explorer-recent-searches';

// ---------------------------------------------------------------------------
// Query parsing
// ---------------------------------------------------------------------------

type QueryKind = 'block_number' | 'tx_hash' | 'address' | 'invalid';

function classifyQuery(query: string): QueryKind {
  const trimmed = query.trim();
  if (!trimmed) return 'invalid';
  if (/^\d+$/.test(trimmed)) return 'block_number';
  if (/^0x[0-9a-fA-F]{64}$/.test(trimmed)) return 'tx_hash';
  if (/^0x[0-9a-fA-F]{40}$/.test(trimmed)) return 'address';
  return 'invalid';
}

// ---------------------------------------------------------------------------
// Recent searches (localStorage)
// ---------------------------------------------------------------------------

function getRecentSearches(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_SEARCHES_KEY);
    if (!raw) return [];
    return JSON.parse(raw) as string[];
  } catch {
    return [];
  }
}

function addRecentSearch(query: string) {
  const recent = getRecentSearches().filter((q) => q !== query);
  recent.unshift(query);
  localStorage.setItem(RECENT_SEARCHES_KEY, JSON.stringify(recent.slice(0, 8)));
}

// ---------------------------------------------------------------------------
// Search hook
// ---------------------------------------------------------------------------

function useSearch() {
  const blocks = useChainStore((s) => s.blocks);
  const [results, setResults] = useState<SearchResult[]>([]);
  const [searching, setSearching] = useState(false);
  const [hint, setHint] = useState<string | null>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  const search = useCallback(
    (query: string) => {
      clearTimeout(debounceRef.current);

      const trimmed = query.trim();
      if (!trimmed) {
        setResults([]);
        setHint(null);
        setSearching(false);
        return;
      }

      const kind = classifyQuery(trimmed);

      if (kind === 'invalid') {
        setResults([]);
        setHint('Enter a block number, transaction hash (0x + 64 hex), or address (0x + 40 hex)');
        setSearching(false);
        return;
      }

      setHint(null);

      // Instant cache search for block numbers
      if (kind === 'block_number') {
        const num = BigInt(trimmed);
        const cached = blocks.get(num);
        if (cached) {
          setResults([
            {
              type: 'block',
              label: `Block ${formatNumber(cached.number)}`,
              subtitle: `${cached.transactions?.length ?? 0} txns`,
              source: 'cache',
              navigateTo: `/block/${cached.number}`,
            },
          ]);
          setSearching(false);
          return;
        }
      }

      // Address: navigate directly
      if (kind === 'address') {
        setResults([
          {
            type: 'address',
            label: truncateHash(trimmed, 6),
            subtitle: 'View address',
            source: 'cache',
            navigateTo: `/address/${trimmed}`,
          },
        ]);
        setSearching(false);
        return;
      }

      // RPC fallback (debounced 150ms)
      setSearching(true);

      debounceRef.current = setTimeout(async () => {
        try {
          if (kind === 'block_number') {
            const block = await client.getBlock({ blockNumber: BigInt(trimmed) });

            if (block) {
              setResults([
                {
                  type: 'block',
                  label: `Block ${formatNumber(BigInt(trimmed))}`,
                  subtitle: 'Fetched from RPC',
                  source: 'rpc',
                  navigateTo: `/block/${trimmed}`,
                },
              ]);
            } else {
              setResults([]);
              setHint('Block not found');
            }
          } else if (kind === 'tx_hash') {
            const tx = await client
              .getTransaction({ hash: trimmed as Hex })
              .catch(() => null);

            if (tx) {
              setResults([
                {
                  type: 'transaction',
                  label: truncateHash(tx.hash, 6),
                  subtitle: `${formatEth(tx.value)} ETH`,
                  source: 'rpc',
                  navigateTo: `/tx/${tx.hash}`,
                },
              ]);
            } else {
              setResults([]);
              setHint('Transaction not found');
            }
          }
        } catch {
          setHint('RPC error -- check connection');
          setResults([]);
        } finally {
          setSearching(false);
        }
      }, 150);
    },
    [blocks],
  );

  return { results, searching, hint, search };
}

// ---------------------------------------------------------------------------
// Result type icon
// ---------------------------------------------------------------------------

function ResultIcon({ type }: { type: SearchResult['type'] }) {
  const color =
    type === 'block'
      ? 'var(--rd-rose)'
      : type === 'transaction'
        ? 'var(--rd-bone-bright)'
        : 'var(--rd-dream)';

  return (
    <span
      style={{
        width: 8,
        height: 8,
        borderRadius: type === 'address' ? '50%' : 0,
        background: color,
        display: 'inline-block',
        flexShrink: 0,
      }}
    />
  );
}

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export function SearchOverlay() {
  const searchOpen = useUIStore((s) => s.searchOpen);
  const setSearchOpen = useUIStore((s) => s.setSearchOpen);
  const navigate = useNavigate();

  const [query, setQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const { results, searching, hint, search } = useSearch();

  // Keyboard shortcut: Cmd+K / Ctrl+K to open, / to open
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const isCmdK = (e.metaKey || e.ctrlKey) && e.key === 'k';
      const isSlash = e.key === '/' && !e.ctrlKey && !e.metaKey;

      if (isSlash && (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement)) {
        return;
      }

      if (isCmdK || isSlash) {
        e.preventDefault();
        setSearchOpen(true);
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [setSearchOpen]);

  // Auto-focus on open
  useEffect(() => {
    if (searchOpen) {
      requestAnimationFrame(() => inputRef.current?.focus());
    } else {
      setQuery('');
      setSelectedIndex(0);
    }
  }, [searchOpen]);

  // Run search on query change
  useEffect(() => {
    search(query);
    setSelectedIndex(0);
  }, [query, search]);

  const close = useCallback(() => {
    setSearchOpen(false);
    setQuery('');
  }, [setSearchOpen]);

  const selectResult = useCallback(
    (result: SearchResult) => {
      addRecentSearch(query);
      close();
      navigate(result.navigateTo);
    },
    [query, close, navigate],
  );

  // Keyboard navigation within overlay
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        close();
      } else if (e.key === 'ArrowDown') {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, results.length - 1));
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === 'Enter' && results[selectedIndex]) {
        e.preventDefault();
        selectResult(results[selectedIndex]);
      }
    },
    [results, selectedIndex, close, selectResult],
  );

  // eslint-disable-next-line react-hooks/exhaustive-deps
  const recentSearches = useMemo(() => getRecentSearches(), [searchOpen]);

  if (!searchOpen) return null;

  return (
    <AnimatePresence>
      <motion.div
        key="search-overlay"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.15 }}
        style={{
          position: 'fixed',
          inset: 0,
          zIndex: 'var(--rd-z-search)' as unknown as number,
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          paddingTop: 120,
        }}
        onClick={(e) => {
          if (e.target === e.currentTarget) close();
        }}
      >
        {/* Backdrop dim */}
        <div
          style={{
            position: 'absolute',
            inset: 0,
            background: 'rgba(6, 6, 8, 0.6)',
          }}
        />

        {/* Search container */}
        <motion.div
          initial={{ y: -12, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          exit={{ y: -12, opacity: 0 }}
          transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
          className={styles.panel}
          style={{
            position: 'relative',
            width: '100%',
            maxWidth: 640,
            margin: '0 var(--rd-space-lg)',
            overflow: 'hidden',
          }}
          role="combobox"
          aria-expanded={results.length > 0}
          aria-haspopup="listbox"
        >
          {/* Input */}
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              padding: 'var(--rd-space-md)',
              gap: 'var(--rd-space-sm)',
              borderBottom: '1px solid var(--rd-border)',
            }}
          >
            <span
              style={{
                color: 'var(--rd-text-dim)',
                fontSize: 'var(--rd-text-lg)',
                flexShrink: 0,
              }}
            >
              &#9906;
            </span>

            <input
              ref={inputRef}
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder="Search blocks, transactions, addresses..."
              aria-label="Search blocks, transactions, addresses"
              style={{
                flex: 1,
                background: 'none',
                border: 'none',
                outline: 'none',
                fontFamily: 'var(--rd-font-mono)',
                fontSize: 'var(--rd-text-base)',
                color: 'var(--rd-text)',
                letterSpacing: 'var(--rd-tracking-normal)',
              }}
            />

            {searching && (
              <span
                style={{
                  color: 'var(--rd-text-dim)',
                  fontSize: 'var(--rd-text-xs)',
                  fontFamily: 'var(--rd-font-mono)',
                }}
              >
                ...
              </span>
            )}

            <span
              style={{
                fontFamily: 'var(--rd-font-mono)',
                fontSize: 'var(--rd-text-xs)',
                color: 'var(--rd-text-ghost)',
                border: '1px solid var(--rd-border)',
                padding: '1px 4px',
                flexShrink: 0,
              }}
            >
              ESC
            </span>
          </div>

          {/* Results list */}
          {results.length > 0 && (
            <div role="listbox" aria-label="Search results">
              {results.map((result, i) => (
                <div
                  key={`${result.type}-${result.navigateTo}`}
                  role="option"
                  aria-selected={i === selectedIndex}
                  className={styles.listRow}
                  style={{
                    background:
                      i === selectedIndex ? 'var(--rd-glass-bg-hover)' : 'transparent',
                  }}
                  onClick={() => selectResult(result)}
                  onMouseEnter={() => setSelectedIndex(i)}
                >
                  <ResultIcon type={result.type} />
                  <span className={styles.label} style={{ flex: '0 0 auto' }}>
                    {result.type.toUpperCase()}
                  </span>
                  <code
                    style={{
                      fontFamily: 'var(--rd-font-mono)',
                      fontSize: 'var(--rd-text-sm)',
                      color: 'var(--rd-text)',
                      flex: 1,
                    }}
                  >
                    {result.label}
                  </code>
                  <span
                    style={{
                      fontFamily: 'var(--rd-font-mono)',
                      fontSize: 'var(--rd-text-xs)',
                      color: 'var(--rd-text-dim)',
                    }}
                  >
                    {result.subtitle}
                  </span>
                </div>
              ))}
            </div>
          )}

          {/* Hint (invalid query / not found) */}
          {hint && (
            <div
              style={{
                padding: 'var(--rd-space-md)',
                fontFamily: 'var(--rd-font-mono)',
                fontSize: 'var(--rd-text-sm)',
                color: 'var(--rd-text-dim)',
                textAlign: 'center',
              }}
            >
              {hint}
            </div>
          )}

          {/* Recent searches (when input is empty) */}
          {!query && recentSearches.length > 0 && (
            <div style={{ padding: 'var(--rd-space-sm) 0' }}>
              <div
                className={styles.label}
                style={{ padding: 'var(--rd-space-xs) var(--rd-space-md)' }}
              >
                RECENT
              </div>
              {recentSearches.map((recent) => (
                <div
                  key={recent}
                  className={styles.listRow}
                  onClick={() => setQuery(recent)}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') setQuery(recent);
                  }}
                >
                  <code
                    style={{
                      fontFamily: 'var(--rd-font-mono)',
                      fontSize: 'var(--rd-text-sm)',
                      color: 'var(--rd-text-dim)',
                    }}
                  >
                    {recent}
                  </code>
                </div>
              ))}
            </div>
          )}
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}
