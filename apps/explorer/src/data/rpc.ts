import {
  createPublicClient,
  http,
  type PublicClient,
  type HttpTransport,
  type Chain,
} from 'viem';
import { defineChain } from 'viem';
import type { NodeStatus } from './types';

// ================================================================
// CHAIN DEFINITION
// ================================================================

export const kora: Chain = defineChain({
  id: Number(import.meta.env.VITE_CHAIN_ID ?? 1337),
  name: 'Kora',
  nativeCurrency: {
    name: 'Ether',
    symbol: 'ETH',
    decimals: 18,
  },
  rpcUrls: {
    default: {
      http: [import.meta.env.VITE_RPC_URL ?? 'http://localhost:8545'],
    },
  },
});

// ================================================================
// HTTP CLIENT
// ================================================================

export const client: PublicClient<HttpTransport, typeof kora> =
  createPublicClient({
    chain: kora,
    transport: http(import.meta.env.VITE_RPC_URL ?? 'http://localhost:8545', {
      retryCount: 3,
      retryDelay: 1000,
      timeout: 10_000,
    }),
  });

// ================================================================
// CUSTOM ACTIONS: kora_nodeStatus
// ================================================================

export async function getNodeStatus(): Promise<NodeStatus> {
  const result = await client.request({
    method: 'kora_nodeStatus' as never,
    params: [] as never,
  });

  const raw = result as Record<string, unknown>;

  return {
    chainId: Number(raw.chainId),
    validatorIndex: Number(raw.validatorIndex),
    validatorCount: Number(raw.validatorCount ?? 0),
    uptimeSecs: Number(raw.uptimeSecs ?? 0),
    currentView: Number(raw.currentView),
    finalizedCount: Number(raw.finalizedCount),
    proposedCount: Number(raw.proposedCount),
    nullifiedCount: Number(raw.nullifiedCount),
    peerCount: Number(raw.peerCount),
    isLeader: Boolean(raw.isLeader),
  };
}

// ================================================================
// CUSTOM ACTIONS: hdc_* methods
// ================================================================

export async function hdcHammingDistance(
  a: `0x${string}`,
  b: `0x${string}`,
): Promise<bigint> {
  const result = await client.request({
    method: 'hdc_hammingDistance' as never,
    params: [a, b] as never,
  });
  return BigInt(result as string);
}

export async function hdcSimilarity(
  a: `0x${string}`,
  b: `0x${string}`,
): Promise<number> {
  const result = await client.request({
    method: 'hdc_similarity' as never,
    params: [a, b] as never,
  });
  return Number(result);
}

export async function hdcBind(
  a: `0x${string}`,
  b: `0x${string}`,
): Promise<`0x${string}`> {
  const result = await client.request({
    method: 'hdc_bind' as never,
    params: [a, b] as never,
  });
  return result as `0x${string}`;
}

export async function hdcBundle(
  vectors: `0x${string}`[],
): Promise<`0x${string}`> {
  const result = await client.request({
    method: 'hdc_bundle' as never,
    params: [vectors] as never,
  });
  return result as `0x${string}`;
}

export async function hdcSearch(
  query: `0x${string}`,
  topK: number,
): Promise<Array<{ id: `0x${string}`; distance: number }>> {
  const result = await client.request({
    method: 'hdc_search' as never,
    params: [query, topK] as never,
  });
  return (result as Array<{ id: string; distance: number }>).map((r) => ({
    id: r.id as `0x${string}`,
    distance: r.distance,
  }));
}

export async function hdcVectorId(
  vector: `0x${string}`,
): Promise<`0x${string}`> {
  const result = await client.request({
    method: 'hdc_vectorId' as never,
    params: [vector] as never,
  });
  return result as `0x${string}`;
}

export async function hdcEncode(
  text: string,
): Promise<`0x${string}`> {
  const result = await client.request({
    method: 'hdc_encode' as never,
    params: [text] as never,
  });
  return result as `0x${string}`;
}
