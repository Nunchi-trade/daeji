import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useEffect, useState, useCallback } from 'react';
import { useNavigate, useParams } from 'react-router';
import { GlassPanel } from '@/panels/GlassPanel';
import { client } from '@/data/rpc';
import { truncateHash, formatEth, formatGas, formatNumber, } from '@/lib/format';
import styles from '@/design/glass.module.css';
function useTxData(hash) {
    const [tx, setTx] = useState(null);
    const [receipt, setReceipt] = useState(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState(null);
    useEffect(() => {
        if (!hash)
            return;
        let cancelled = false;
        async function fetchTx() {
            setLoading(true);
            setError(null);
            try {
                const [txResult, receiptResult] = await Promise.all([
                    client.getTransaction({ hash: hash }),
                    client.getTransactionReceipt({ hash: hash }).catch(() => null),
                ]);
                if (!cancelled) {
                    if (txResult) {
                        setTx({
                            hash: txResult.hash,
                            from: txResult.from,
                            to: (txResult.to ?? null),
                            value: txResult.value,
                            gas: txResult.gas,
                            gasPrice: txResult.gasPrice ?? 0n,
                            input: txResult.input,
                            nonce: txResult.nonce,
                            blockNumber: txResult.blockNumber ?? 0n,
                            blockHash: txResult.blockHash ?? '0x',
                            transactionIndex: txResult.transactionIndex ?? 0,
                            type: txResult.type === 'eip1559' ? 2 : txResult.type === 'eip2930' ? 1 : 0,
                        });
                    }
                    else {
                        setError('Transaction not found');
                    }
                    if (receiptResult) {
                        setReceipt({
                            transactionHash: receiptResult.transactionHash,
                            status: receiptResult.status === 'success' ? 'success' : 'reverted',
                            blockNumber: receiptResult.blockNumber,
                            blockHash: receiptResult.blockHash,
                            from: receiptResult.from,
                            to: (receiptResult.to ?? null),
                            gasUsed: receiptResult.gasUsed,
                            cumulativeGasUsed: receiptResult.cumulativeGasUsed,
                            contractAddress: (receiptResult.contractAddress ?? null),
                            logs: receiptResult.logs.map((log) => ({
                                address: log.address,
                                topics: log.topics,
                                data: log.data,
                                blockNumber: log.blockNumber ?? 0n,
                                transactionHash: log.transactionHash ?? '0x',
                                logIndex: log.logIndex ?? 0,
                            })),
                        });
                    }
                }
            }
            catch (err) {
                if (!cancelled) {
                    setError(err instanceof Error ? err.message : 'Failed to fetch transaction');
                }
            }
            finally {
                if (!cancelled)
                    setLoading(false);
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
function FlowDiagram({ from, to, value, onAddressClick, }) {
    return (_jsxs("div", { style: {
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            padding: 'var(--rd-space-md)',
            borderBottom: '1px solid var(--rd-border)',
            gap: 'var(--rd-space-sm)',
        }, children: [_jsxs("div", { style: { textAlign: 'center', cursor: 'pointer' }, onClick: () => onAddressClick(from), role: "button", tabIndex: 0, onKeyDown: (e) => { if (e.key === 'Enter')
                    onAddressClick(from); }, children: [_jsx("code", { style: {
                            fontFamily: 'var(--rd-font-mono)',
                            fontSize: 'var(--rd-text-sm)',
                            color: 'var(--rd-bone-bright)',
                        }, children: truncateHash(from, 4) }), _jsx("div", { className: styles.label, children: "SENDER" })] }), _jsxs("div", { style: { textAlign: 'center', flex: 1 }, children: [_jsxs("div", { style: {
                            fontFamily: 'var(--rd-font-mono)',
                            fontSize: 'var(--rd-text-sm)',
                            color: 'var(--rd-bone)',
                            marginBottom: 'var(--rd-space-xs)',
                        }, children: [formatEth(value), " ETH"] }), _jsx("div", { style: { height: 1, background: 'var(--rd-bone-dim)', position: 'relative' }, children: _jsx("div", { style: {
                                position: 'absolute',
                                right: -4,
                                top: -3,
                                width: 0,
                                height: 0,
                                borderLeft: '6px solid var(--rd-bone-dim)',
                                borderTop: '3px solid transparent',
                                borderBottom: '3px solid transparent',
                            } }) })] }), _jsxs("div", { style: { textAlign: 'center', cursor: to ? 'pointer' : 'default' }, onClick: () => to && onAddressClick(to), role: "button", tabIndex: to ? 0 : -1, onKeyDown: (e) => { if (e.key === 'Enter' && to)
                    onAddressClick(to); }, children: [_jsx("code", { style: {
                            fontFamily: 'var(--rd-font-mono)',
                            fontSize: 'var(--rd-text-sm)',
                            color: to ? 'var(--rd-bone-bright)' : 'var(--rd-rose-bright)',
                        }, children: to ? truncateHash(to, 4) : 'CONTRACT CREATE' }), _jsx("div", { className: styles.label, children: to ? 'RECEIVER' : 'CREATE' })] })] }));
}
function StatusBadge({ status }) {
    const isSuccess = status === 'success';
    const cls = isSuccess ? styles.badgeSuccess : styles.badgeReverted;
    return (_jsxs("span", { className: `${styles.badge} ${cls}`, children: [_jsx("span", { style: {
                    width: 5,
                    height: 5,
                    borderRadius: '50%',
                    background: 'currentColor',
                    display: 'inline-block',
                } }), status.toUpperCase()] }));
}
function LogEntry({ log, index }) {
    return (_jsxs("div", { style: {
            padding: 'var(--rd-space-sm) var(--rd-space-md)',
            borderBottom: '1px solid var(--rd-border)',
        }, children: [_jsxs("div", { style: {
                    display: 'flex',
                    alignItems: 'center',
                    gap: 'var(--rd-space-sm)',
                    marginBottom: 'var(--rd-space-xs)',
                }, children: [_jsxs("span", { className: styles.label, children: ["LOG ", index] }), _jsx("code", { style: {
                            fontFamily: 'var(--rd-font-mono)',
                            fontSize: 'var(--rd-text-xs)',
                            color: 'var(--rd-text-dim)',
                        }, children: truncateHash(log.address, 4) })] }), log.topics.map((topic, i) => (_jsxs("div", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-xs)',
                    color: i === 0 ? 'var(--rd-dream)' : 'var(--rd-text-dim)',
                    marginLeft: 'var(--rd-space-md)',
                    marginBottom: 2,
                    wordBreak: 'break-all',
                }, children: ["[", i, "] ", truncateHash(topic, 8)] }, i))), log.data !== '0x' && (_jsxs("div", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-xs)',
                    color: 'var(--rd-text-ghost)',
                    marginLeft: 'var(--rd-space-md)',
                    marginTop: 'var(--rd-space-xs)',
                    wordBreak: 'break-all',
                    maxHeight: 60,
                    overflow: 'hidden',
                }, children: ["data: ", truncateHash(log.data, 16)] }))] }));
}
// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------
export function TxDetail() {
    const { hash } = useParams();
    const navigate = useNavigate();
    const { tx, receipt, loading, error } = useTxData(hash);
    const handleClose = useCallback(() => navigate(-1), [navigate]);
    const goToAddress = useCallback((addr) => navigate(`/address/${addr}`), [navigate]);
    const goToBlock = useCallback((num) => navigate(`/block/${num}`), [navigate]);
    if (loading) {
        return (_jsx(GlassPanel, { position: "right", width: "520px", title: "TRANSACTION", onClose: handleClose, children: _jsx("div", { className: styles.panelBody, children: _jsx("span", { style: { color: 'var(--rd-text-dim)' }, children: "Loading..." }) }) }));
    }
    if (error || !tx) {
        return (_jsx(GlassPanel, { position: "right", width: "520px", title: "TRANSACTION", onClose: handleClose, children: _jsx("div", { className: styles.panelBody, children: _jsx("span", { style: { color: 'var(--rd-danger)' }, children: error ?? 'Transaction not found' }) }) }));
    }
    return (_jsx(GlassPanel, { position: "right", width: "520px", title: "TRANSACTION", onClose: handleClose, active: true, resizable: true, ariaLabel: `Transaction ${tx.hash} detail`, testId: "tx-detail", children: _jsxs("div", { className: styles.panelBody, children: [_jsxs("div", { style: {
                        display: 'flex',
                        alignItems: 'center',
                        justifyContent: 'space-between',
                        marginBottom: 'var(--rd-space-md)',
                    }, children: [_jsx("code", { style: {
                                fontFamily: 'var(--rd-font-mono)',
                                fontSize: 'var(--rd-text-sm)',
                                color: 'var(--rd-text)',
                            }, "aria-label": `Transaction hash: ${tx.hash}`, children: truncateHash(tx.hash, 8) }), receipt && _jsx(StatusBadge, { status: receipt.status })] }), _jsx(FlowDiagram, { from: tx.from, to: tx.to, value: tx.value, onAddressClick: goToAddress }), _jsxs("div", { className: styles.statsGrid, style: { marginTop: 'var(--rd-space-md)' }, children: [_jsxs("div", { className: styles.statCell, children: [_jsx("div", { className: styles.statLabel, children: "VALUE" }), _jsxs("div", { className: styles.statValue, children: [formatEth(tx.value), " ETH"] })] }), _jsxs("div", { className: styles.statCell, children: [_jsx("div", { className: styles.statLabel, children: "GAS PRICE" }), _jsxs("div", { className: styles.statValue, children: [formatGas(tx.gasPrice), " gwei"] })] }), _jsxs("div", { className: styles.statCell, children: [_jsx("div", { className: styles.statLabel, children: "GAS LIMIT" }), _jsx("div", { className: styles.statValue, children: formatGas(tx.gas) })] }), _jsxs("div", { className: styles.statCell, children: [_jsx("div", { className: styles.statLabel, children: "GAS USED" }), _jsx("div", { className: styles.statValue, children: receipt ? formatGas(receipt.gasUsed) : '--' })] }), _jsxs("div", { className: styles.statCell, children: [_jsx("div", { className: styles.statLabel, children: "NONCE" }), _jsx("div", { className: styles.statValue, children: tx.nonce })] }), _jsxs("div", { className: styles.statCell, children: [_jsx("div", { className: styles.statLabel, children: "TYPE" }), _jsxs("div", { className: styles.statValue, children: ["0x", tx.type.toString(16), ' ', _jsxs("span", { style: { color: 'var(--rd-text-dim)' }, children: ["(", tx.type === 2 ? '1559' : tx.type === 1 ? '2930' : 'legacy', ")"] })] })] })] }), receipt && (_jsxs("div", { style: { margin: 'var(--rd-space-md) 0' }, children: [_jsx("div", { className: styles.statLabel, children: "GAS USAGE" }), _jsx("div", { className: styles.gasBar, style: { marginTop: 'var(--rd-space-xs)' }, children: _jsx("div", { className: styles.gasBarFill, style: {
                                    width: `${tx.gas > 0n ? Number((receipt.gasUsed * 100n) / tx.gas) : 0}%`,
                                } }) })] })), tx.input && tx.input !== '0x' && (_jsxs("div", { style: { marginTop: 'var(--rd-space-md)' }, children: [_jsx("div", { className: styles.statLabel, children: "INPUT DATA" }), _jsx("div", { style: {
                                fontFamily: 'var(--rd-font-mono)',
                                fontSize: 'var(--rd-text-xs)',
                                color: 'var(--rd-text-dim)',
                                background: 'var(--rd-void-light)',
                                padding: 'var(--rd-space-sm)',
                                marginTop: 'var(--rd-space-xs)',
                                maxHeight: 120,
                                overflow: 'auto',
                                wordBreak: 'break-all',
                            }, children: tx.input })] })), _jsxs("div", { style: {
                        marginTop: 'var(--rd-space-md)',
                        paddingTop: 'var(--rd-space-md)',
                        borderTop: '1px solid var(--rd-border)',
                    }, children: [_jsx("div", { className: styles.statLabel, children: "BLOCK" }), _jsx("button", { onClick: () => goToBlock(tx.blockNumber), style: {
                                background: 'none',
                                border: 'none',
                                color: 'var(--rd-rose)',
                                cursor: 'pointer',
                                fontFamily: 'var(--rd-font-mono)',
                                fontSize: 'var(--rd-text-base)',
                                padding: 0,
                                textDecoration: 'none',
                            }, children: formatNumber(tx.blockNumber) })] }), receipt && receipt.logs.length > 0 && (_jsxs("div", { style: { marginTop: 'var(--rd-space-lg)' }, children: [_jsxs("div", { className: styles.label, style: { marginBottom: 'var(--rd-space-sm)' }, children: ["\u2014\u2014 LOGS (", receipt.logs.length, ")"] }), _jsx("div", { className: styles.scrollList, children: receipt.logs.map((log, i) => (_jsx(LogEntry, { log: log, index: i }, `${log.transactionHash}-${log.logIndex}`))) })] }))] }) }));
}
