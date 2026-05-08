import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useEffect, useState, useCallback } from 'react';
import { useNavigate, useParams } from 'react-router';
import { GlassPanel } from '@/panels/GlassPanel';
import { useChainStore } from '@/data/store';
import { client } from '@/data/rpc';
import { truncateHash, formatEth, formatGas, formatNumber, } from '@/lib/format';
import styles from '@/design/glass.module.css';
// ---------------------------------------------------------------------------
// Data fetching
// ---------------------------------------------------------------------------
function useAddressData(address) {
    const blocks = useChainStore((s) => s.blocks);
    const [info, setInfo] = useState({
        address: (address ?? '0x'),
        balance: 0n,
        isContract: false,
        codeSize: 0,
        recentTxs: [],
        loading: true,
        error: null,
    });
    useEffect(() => {
        if (!address)
            return;
        let cancelled = false;
        async function fetchAddress() {
            setInfo((prev) => ({ ...prev, loading: true, error: null }));
            try {
                const [balance, code] = await Promise.all([
                    client.getBalance({ address: address }),
                    client.getCode({ address: address }),
                ]);
                // Scan cached blocks for recent transactions involving this address
                const recentTxs = [];
                const lowerAddr = address.toLowerCase();
                for (const block of blocks.values()) {
                    if (!block.transactions)
                        continue;
                    for (const tx of block.transactions) {
                        if (tx.from.toLowerCase() === lowerAddr ||
                            (tx.to && tx.to.toLowerCase() === lowerAddr)) {
                            recentTxs.push(tx);
                        }
                    }
                    if (recentTxs.length >= 10)
                        break;
                }
                recentTxs.sort((a, b) => {
                    if (b.blockNumber > a.blockNumber)
                        return 1;
                    if (b.blockNumber < a.blockNumber)
                        return -1;
                    return b.transactionIndex - a.transactionIndex;
                });
                const isContract = !!code && code !== '0x';
                const codeSize = isContract ? (code.length - 2) / 2 : 0;
                if (!cancelled) {
                    setInfo({
                        address: address,
                        balance,
                        isContract,
                        codeSize,
                        recentTxs: recentTxs.slice(0, 10),
                        loading: false,
                        error: null,
                    });
                }
            }
            catch (err) {
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
function TypeBadge({ isContract }) {
    const cls = isContract ? styles.badgeContract : styles.badgeEoa;
    return (_jsx("span", { className: `${styles.badge} ${cls}`, children: isContract ? 'CONTRACT' : 'EOA' }));
}
function TxRow({ tx, highlightAddr, onTxClick, }) {
    const isSender = tx.from.toLowerCase() === highlightAddr.toLowerCase();
    return (_jsxs("div", { className: styles.listRow, onClick: () => onTxClick(tx.hash), role: "button", tabIndex: 0, onKeyDown: (e) => { if (e.key === 'Enter')
            onTxClick(tx.hash); }, children: [_jsx("code", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-xs)',
                    color: 'var(--rd-text-dim)',
                    flex: '0 0 80px',
                }, children: truncateHash(tx.hash, 3) }), _jsx("span", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-xs)',
                    color: isSender ? 'var(--rd-danger)' : 'var(--rd-success)',
                    flex: '0 0 30px',
                }, children: isSender ? 'OUT' : 'IN' }), _jsx("span", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-xs)',
                    color: 'var(--rd-text-dim)',
                    flex: 1,
                }, children: isSender
                    ? tx.to
                        ? truncateHash(tx.to, 3)
                        : 'CREATE'
                    : truncateHash(tx.from, 3) }), _jsxs("span", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-sm)',
                    color: 'var(--rd-bone)',
                    flex: '0 0 90px',
                    textAlign: 'right',
                }, children: [formatEth(tx.value), " ETH"] }), _jsx("span", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-xs)',
                    color: 'var(--rd-text-ghost)',
                    flex: '0 0 60px',
                    textAlign: 'right',
                }, children: formatGas(tx.gas) })] }));
}
// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------
export function AddressDetail() {
    const { address } = useParams();
    const navigate = useNavigate();
    const info = useAddressData(address);
    const handleClose = useCallback(() => navigate(-1), [navigate]);
    const goToTx = useCallback((hash) => navigate(`/tx/${hash}`), [navigate]);
    if (info.loading) {
        return (_jsx(GlassPanel, { position: "right", width: "480px", title: "ADDRESS", onClose: handleClose, children: _jsx("div", { className: styles.panelBody, children: _jsx("span", { style: { color: 'var(--rd-text-dim)' }, children: "Loading..." }) }) }));
    }
    if (info.error) {
        return (_jsx(GlassPanel, { position: "right", width: "480px", title: "ADDRESS", onClose: handleClose, children: _jsx("div", { className: styles.panelBody, children: _jsx("span", { style: { color: 'var(--rd-danger)' }, children: info.error }) }) }));
    }
    return (_jsx(GlassPanel, { position: "right", width: "480px", title: "ADDRESS", onClose: handleClose, active: true, resizable: true, ariaLabel: `Address ${info.address} detail`, testId: "address-detail", children: _jsxs("div", { className: styles.panelBody, children: [_jsxs("div", { style: {
                        display: 'flex',
                        alignItems: 'center',
                        justifyContent: 'space-between',
                        marginBottom: 'var(--rd-space-md)',
                    }, children: [_jsx("code", { style: {
                                fontFamily: 'var(--rd-font-mono)',
                                fontSize: 'var(--rd-text-sm)',
                                color: 'var(--rd-text)',
                                wordBreak: 'break-all',
                            }, "aria-label": `Address: ${info.address}`, children: info.address }), _jsx("div", { style: { marginLeft: 'var(--rd-space-sm)', flexShrink: 0 }, children: _jsx(TypeBadge, { isContract: info.isContract }) })] }), _jsxs("div", { style: { marginBottom: 'var(--rd-space-lg)' }, children: [_jsx("div", { className: styles.statLabel, children: "BALANCE" }), _jsxs("div", { style: {
                                fontFamily: 'var(--rd-font-display)',
                                fontStyle: 'italic',
                                fontSize: 'var(--rd-text-hero)',
                                color: 'var(--rd-bone-bright)',
                                lineHeight: 1.1,
                            }, children: [formatEth(info.balance), ' ', _jsx("span", { style: {
                                        fontSize: 'var(--rd-text-lg)',
                                        color: 'var(--rd-text-dim)',
                                    }, children: "ETH" })] })] }), info.isContract && (_jsxs("div", { className: styles.statsGrid, style: { marginBottom: 'var(--rd-space-md)' }, children: [_jsxs("div", { className: styles.statCell, children: [_jsx("div", { className: styles.statLabel, children: "CODE SIZE" }), _jsxs("div", { className: styles.statValue, children: [formatNumber(BigInt(info.codeSize)), ' ', _jsx("span", { style: { color: 'var(--rd-text-dim)' }, children: "bytes" })] })] }), _jsxs("div", { className: styles.statCell, children: [_jsx("div", { className: styles.statLabel, children: "TYPE" }), _jsx("div", { className: styles.statValue, style: { color: 'var(--rd-rose)' }, children: "Contract" })] })] })), _jsxs("div", { style: { marginTop: 'var(--rd-space-md)' }, children: [_jsx("div", { className: styles.label, style: { marginBottom: 'var(--rd-space-sm)' }, children: "\u2014\u2014 RECENT TRANSACTIONS" }), info.recentTxs.length === 0 ? (_jsx("div", { style: {
                                fontFamily: 'var(--rd-font-mono)',
                                fontSize: 'var(--rd-text-sm)',
                                color: 'var(--rd-text-ghost)',
                                padding: 'var(--rd-space-md)',
                                textAlign: 'center',
                            }, children: "No transactions found in cached blocks" })) : (_jsxs("div", { className: styles.scrollList, children: [_jsxs("div", { style: {
                                        display: 'flex',
                                        padding: 'var(--rd-space-xs) var(--rd-space-md)',
                                        borderBottom: '1px solid var(--rd-border-strong)',
                                    }, children: [_jsx("span", { className: styles.label, style: { flex: '0 0 80px' }, children: "HASH" }), _jsx("span", { className: styles.label, style: { flex: '0 0 30px' }, children: "DIR" }), _jsx("span", { className: styles.label, style: { flex: 1 }, children: "TO/FROM" }), _jsx("span", { className: styles.label, style: { flex: '0 0 90px', textAlign: 'right' }, children: "VALUE" }), _jsx("span", { className: styles.label, style: { flex: '0 0 60px', textAlign: 'right' }, children: "GAS" })] }), info.recentTxs.map((tx) => (_jsx(TxRow, { tx: tx, highlightAddr: info.address, onTxClick: goToTx }, tx.hash)))] }))] })] }) }));
}
