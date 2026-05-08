/**
 * Pure formatting utilities for the Kora Explorer.
 */

/** Truncate an address to `0x1234...5678` form. */
export function truncateAddress(addr: string, chars = 4): string {
  if (addr.length <= chars * 2 + 2) return addr;
  return `${addr.slice(0, chars + 2)}...${addr.slice(-chars)}`;
}

/** Truncate a hex hash: "0xabcdef...012345" */
export function truncateHash(hash: string, chars = 6): string {
  if (!hash || hash.length <= chars * 2 + 4) return hash ?? '';
  return `${hash.slice(0, chars + 2)}...${hash.slice(-chars)}`;
}

/** Convert wei (bigint or string) to ETH string with fixed decimals. */
export function formatEth(wei: bigint | string, decimals = 4): string {
  const value = typeof wei === "string" ? BigInt(wei) : wei;
  if (value === 0n) return "0";

  const negative = value < 0n;
  const absWei = negative ? -value : value;
  const ethWhole = absWei / 10n ** 18n;
  const ethFrac = absWei % 10n ** 18n;
  const fracStr = ethFrac.toString().padStart(18, "0").slice(0, decimals);
  const trimmed = fracStr.replace(/0+$/, "");

  const sign = negative ? "-" : "";
  if (!trimmed) return `${sign}${formatNumber(ethWhole)}`;
  return `${sign}${formatNumber(ethWhole)}.${trimmed}`;
}

/** Format gas as a human-readable string with comma separators. */
export function formatGas(gas: bigint | number | string): string {
  return Number(gas).toLocaleString("en-US");
}

/** Format a unix timestamp (seconds or bigint) into a locale date-time string. */
export function formatTimestamp(ts: number | bigint): string {
  return new Date(Number(ts) * 1000).toLocaleString("en-US", {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

/** Convert a hex string (with or without 0x prefix) to a number. */
export function hexToNumber(hex: string): number {
  return Number(hex.startsWith("0x") ? hex : `0x${hex}`);
}

/** Format a block number with comma separators. */
export function formatBlockNumber(n: number | bigint): string {
  return Number(n).toLocaleString("en-US");
}

/** Format a number (or bigint) with comma separators. */
export function formatNumber(n: number | bigint): string {
  return Number(n).toLocaleString("en-US");
}
