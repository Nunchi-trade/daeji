import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useEffect, useState, useCallback, useRef } from 'react';
import { useNavigate, useParams } from 'react-router';
import { GlassPanel } from '@/panels/GlassPanel';
import { HashArt } from './HashArt';
import { useChainStore } from '@/data/store';
import { client } from '@/data/rpc';
import { truncateHash, formatEth, formatGas, formatTimestamp, formatNumber, } from '@/lib/format';
import styles from '@/design/glass.module.css';
function useBlockData(numberOrHash) {
    const blocks = useChainStore((s) => s.blocks);
    const [block, setBlock] = useState(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState(null);
    useEffect(() => {
        if (!numberOrHash)
            return;
        let cancelled = false;
        async function fetchBlock() {
            setLoading(true);
            setError(null);
            try {
                const isNumber = /^\d+$/.test(numberOrHash);
                // Try cache first for block numbers
                if (isNumber) {
                    const num = BigInt(numberOrHash);
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
                        blockNumber: BigInt(numberOrHash),
                        includeTransactions: true,
                    })
                    : await client.getBlock({
                        blockHash: numberOrHash,
                        includeTransactions: true,
                    });
                if (!cancelled && result) {
                    // Map viem block to our ChainBlock type
                    const mapped = {
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
                                return { hash: tx };
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
                                blockHash: tx.blockHash ?? '0x',
                                transactionIndex: tx.transactionIndex ?? 0,
                                type: tx.type === 'eip1559' ? 2 : tx.type === 'eip2930' ? 1 : 0,
                            };
                        }),
                        _activityLevel: result.gasLimit > 0n
                            ? Number(result.gasUsed) / Number(result.gasLimit)
                            : 0,
                        _arrivalTime: Date.now(),
                    };
                    setBlock(mapped);
                }
                else if (!cancelled) {
                    setError('Block not found');
                }
            }
            catch (err) {
                if (!cancelled) {
                    setError(err instanceof Error ? err.message : 'Failed to fetch block');
                }
            }
            finally {
                if (!cancelled)
                    setLoading(false);
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
    const [copiedField, setCopiedField] = useState(null);
    const timerRef = useRef(undefined);
    const copy = useCallback((text, field) => {
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
function HashField({ label, hash, copiedField, onCopy, }) {
    const isCopied = copiedField === label;
    return (_jsxs("div", { style: { marginBottom: 'var(--rd-space-sm)' }, children: [_jsx("div", { className: styles.statLabel, children: label }), _jsxs("div", { className: `${styles.hashRow} ${isCopied ? styles.copied : ''}`, onClick: () => onCopy(hash, label), title: hash, role: "button", tabIndex: 0, onKeyDown: (e) => {
                    if (e.key === 'Enter' || e.key === ' ')
                        onCopy(hash, label);
                }, children: [_jsx("code", { children: truncateHash(hash, 8) }), _jsx("span", { className: styles.copyBtn, children: isCopied ? 'copied' : 'copy' })] })] }));
}
function GasBar({ used, limit }) {
    const pct = limit > 0n ? Number((used * 100n) / limit) : 0;
    return (_jsxs("div", { children: [_jsxs("div", { style: {
                    display: 'flex',
                    justifyContent: 'space-between',
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-sm)',
                }, children: [_jsx("span", { style: { color: 'var(--rd-text)' }, children: formatGas(used) }), _jsxs("span", { style: { color: 'var(--rd-text-dim)' }, children: ["/ ", formatGas(limit)] })] }), _jsx("div", { className: styles.gasBar, children: _jsx("div", { className: styles.gasBarFill, style: { width: `${pct}%` } }) })] }));
}
function TransactionRow({ tx, onClick, }) {
    return (_jsxs("div", { className: styles.listRow, onClick: () => onClick(tx.hash), role: "button", tabIndex: 0, onKeyDown: (e) => {
            if (e.key === 'Enter')
                onClick(tx.hash);
        }, children: [_jsx("code", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-sm)',
                    color: 'var(--rd-bone-bright)',
                    flex: '0 0 120px',
                }, children: truncateHash(tx.hash, 4) }), _jsxs("span", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-xs)',
                    color: 'var(--rd-text-dim)',
                    flex: 1,
                }, children: [truncateHash(tx.from, 4), " \u2192 ", tx.to ? truncateHash(tx.to, 4) : 'CREATE'] }), _jsxs("span", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-sm)',
                    color: 'var(--rd-bone)',
                    flex: '0 0 100px',
                    textAlign: 'right',
                }, children: [formatEth(tx.value), " ETH"] })] }));
}
// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------
export function BlockDetail() {
    const { numberOrHash } = useParams();
    const navigate = useNavigate();
    const { block, loading, error } = useBlockData(numberOrHash);
    const { copiedField, copy } = useCopyToClipboard();
    const goToBlock = useCallback((num) => navigate(`/block/${num}`), [navigate]);
    const goToTx = useCallback((hash) => navigate(`/tx/${hash}`), [navigate]);
    const handleClose = useCallback(() => navigate(-1), [navigate]);
    // Keyboard: left/right arrows for block navigation
    useEffect(() => {
        if (!block)
            return;
        const handler = (e) => {
            if (e.key === 'ArrowLeft' && block.number > 0n) {
                goToBlock(block.number - 1n);
            }
            else if (e.key === 'ArrowRight') {
                goToBlock(block.number + 1n);
            }
        };
        window.addEventListener('keydown', handler);
        return () => window.removeEventListener('keydown', handler);
    }, [block, goToBlock]);
    if (loading) {
        return (_jsx(GlassPanel, { position: "right", width: "480px", title: "BLOCK", onClose: handleClose, children: _jsx("div", { className: styles.panelBody, children: _jsx("span", { style: { color: 'var(--rd-text-dim)' }, children: "Loading..." }) }) }));
    }
    if (error || !block) {
        return (_jsx(GlassPanel, { position: "right", width: "480px", title: "BLOCK", onClose: handleClose, children: _jsx("div", { className: styles.panelBody, children: _jsx("span", { style: { color: 'var(--rd-danger)' }, children: error ?? 'Block not found' }) }) }));
    }
    const txCount = block.transactions?.length ?? 0;
    return (_jsx(GlassPanel, { position: "right", width: "480px", title: `BLOCK ${formatNumber(block.number)}`, onClose: handleClose, active: true, resizable: true, ariaLabel: `Block ${block.number} detail`, testId: "block-detail", children: _jsxs("div", { className: styles.panelBody, children: [_jsxs("div", { style: {
                        display: 'flex',
                        justifyContent: 'space-between',
                        alignItems: 'baseline',
                        marginBottom: 'var(--rd-space-md)',
                    }, children: [_jsx("span", { style: {
                                fontFamily: 'var(--rd-font-display)',
                                fontStyle: 'italic',
                                fontSize: 'var(--rd-text-hero)',
                                color: 'var(--rd-bone-bright)',
                                lineHeight: 1,
                            }, children: formatNumber(block.number) }), _jsx("span", { style: {
                                fontFamily: 'var(--rd-font-mono)',
                                fontSize: 'var(--rd-text-xs)',
                                color: 'var(--rd-text-dim)',
                                letterSpacing: 'var(--rd-tracking-normal)',
                            }, children: formatTimestamp(block.timestamp) })] }), _jsx("div", { style: {
                        display: 'flex',
                        justifyContent: 'center',
                        marginBottom: 'var(--rd-space-md)',
                    }, children: _jsx(HashArt, { hash: block.hash, size: 128 }) }), _jsx(HashField, { label: "HASH", hash: block.hash, copiedField: copiedField, onCopy: copy }), _jsx(HashField, { label: "PARENT", hash: block.parentHash, copiedField: copiedField, onCopy: copy }), _jsx(HashField, { label: "STATE ROOT", hash: block.stateRoot, copiedField: copiedField, onCopy: copy }), _jsx(HashField, { label: "TRANSACTIONS ROOT", hash: block.transactionsRoot, copiedField: copiedField, onCopy: copy }), _jsx(HashField, { label: "RECEIPTS ROOT", hash: block.receiptsRoot, copiedField: copiedField, onCopy: copy }), _jsxs("div", { className: styles.statsGrid, style: { marginTop: 'var(--rd-space-md)' }, children: [_jsxs("div", { className: styles.statCell, children: [_jsx("div", { className: styles.statLabel, children: "GAS USED / LIMIT" }), _jsx(GasBar, { used: block.gasUsed, limit: block.gasLimit })] }), _jsxs("div", { className: styles.statCell, children: [_jsx("div", { className: styles.statLabel, children: "TRANSACTIONS" }), _jsx("div", { className: styles.statValue, children: txCount })] }), _jsxs("div", { className: styles.statCell, children: [_jsx("div", { className: styles.statLabel, children: "BASE FEE" }), _jsxs("div", { className: styles.statValue, children: [formatGas(block.baseFeePerGas), ' ', _jsx("span", { style: { color: 'var(--rd-text-dim)' }, children: "gwei" })] })] }), _jsxs("div", { className: styles.statCell, children: [_jsx("div", { className: styles.statLabel, children: "TIMESTAMP" }), _jsx("div", { className: styles.statValue, children: block.timestamp.toString() })] })] }), _jsxs("div", { style: { marginTop: 'var(--rd-space-lg)' }, children: [_jsx("div", { className: styles.label, style: { marginBottom: 'var(--rd-space-sm)' }, children: "\u2014\u2014 TRANSACTIONS" }), txCount === 0 ? (_jsx("div", { style: {
                                fontFamily: 'var(--rd-font-mono)',
                                fontSize: 'var(--rd-text-sm)',
                                color: 'var(--rd-text-ghost)',
                                padding: 'var(--rd-space-md)',
                                textAlign: 'center',
                            }, children: "No transactions in this block" })) : (_jsx("div", { className: styles.scrollList, children: block.transactions.map((tx) => (_jsx(TransactionRow, { tx: tx, onClick: goToTx }, tx.hash))) }))] }), _jsxs("div", { style: {
                        display: 'flex',
                        justifyContent: 'space-between',
                        marginTop: 'var(--rd-space-lg)',
                        paddingTop: 'var(--rd-space-md)',
                        borderTop: '1px solid var(--rd-border)',
                    }, children: [_jsxs("button", { className: styles.navArrow, onClick: () => goToBlock(block.number - 1n), disabled: block.number <= 0n, "aria-label": "Previous block", children: ["\u2190 BLOCK ", formatNumber(block.number - 1n)] }), _jsxs("button", { className: styles.navArrow, onClick: () => goToBlock(block.number + 1n), "aria-label": "Next block", children: ["BLOCK ", formatNumber(block.number + 1n), " \u2192"] })] })] }) }));
}
