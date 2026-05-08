/**
 * Pure formatting utilities for the Kora Explorer.
 */
/** Truncate an address to `0x1234...5678` form. */
export function truncateAddress(addr, chars = 4) {
    if (addr.length <= chars * 2 + 2)
        return addr;
    return `${addr.slice(0, chars + 2)}...${addr.slice(-chars)}`;
}
/** Truncate a hex hash: "0xabcdef...012345" */
export function truncateHash(hash, chars = 6) {
    if (!hash || hash.length <= chars * 2 + 4)
        return hash ?? '';
    return `${hash.slice(0, chars + 2)}...${hash.slice(-chars)}`;
}
/** Convert wei (bigint or string) to ETH string with fixed decimals. */
export function formatEth(wei, decimals = 4) {
    const value = typeof wei === "string" ? BigInt(wei) : wei;
    if (value === 0n)
        return "0";
    const negative = value < 0n;
    const absWei = negative ? -value : value;
    const ethWhole = absWei / 10n ** 18n;
    const ethFrac = absWei % 10n ** 18n;
    const fracStr = ethFrac.toString().padStart(18, "0").slice(0, decimals);
    const trimmed = fracStr.replace(/0+$/, "");
    const sign = negative ? "-" : "";
    if (!trimmed)
        return `${sign}${formatNumber(ethWhole)}`;
    return `${sign}${formatNumber(ethWhole)}.${trimmed}`;
}
/** Format gas as a human-readable string with comma separators. */
export function formatGas(gas) {
    return Number(gas).toLocaleString("en-US");
}
/** Format a unix timestamp (seconds or bigint) into a locale date-time string. */
export function formatTimestamp(ts) {
    return new Date(Number(ts) * 1000).toLocaleString("en-US", {
        dateStyle: "medium",
        timeStyle: "short",
    });
}
/** Convert a hex string (with or without 0x prefix) to a number. */
export function hexToNumber(hex) {
    return Number(hex.startsWith("0x") ? hex : `0x${hex}`);
}
/** Format a block number with comma separators. */
export function formatBlockNumber(n) {
    return Number(n).toLocaleString("en-US");
}
/** Format a number (or bigint) with comma separators. */
export function formatNumber(n) {
    return Number(n).toLocaleString("en-US");
}
