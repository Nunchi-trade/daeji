import { createPublicClient, http, } from 'viem';
import { defineChain } from 'viem';
// ================================================================
// CHAIN DEFINITION
// ================================================================
export const kora = defineChain({
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
export const client = createPublicClient({
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
export async function getNodeStatus() {
    const result = await client.request({
        method: 'kora_nodeStatus',
        params: [],
    });
    const raw = result;
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
export async function hdcHammingDistance(a, b) {
    const result = await client.request({
        method: 'hdc_hammingDistance',
        params: [a, b],
    });
    return BigInt(result);
}
export async function hdcSimilarity(a, b) {
    const result = await client.request({
        method: 'hdc_similarity',
        params: [a, b],
    });
    return Number(result);
}
export async function hdcBind(a, b) {
    const result = await client.request({
        method: 'hdc_bind',
        params: [a, b],
    });
    return result;
}
export async function hdcBundle(vectors) {
    const result = await client.request({
        method: 'hdc_bundle',
        params: [vectors],
    });
    return result;
}
export async function hdcSearch(query, topK) {
    const result = await client.request({
        method: 'hdc_search',
        params: [query, topK],
    });
    return result.map((r) => ({
        id: r.id,
        distance: r.distance,
    }));
}
export async function hdcVectorId(vector) {
    const result = await client.request({
        method: 'hdc_vectorId',
        params: [vector],
    });
    return result;
}
export async function hdcEncode(text) {
    const result = await client.request({
        method: 'hdc_encode',
        params: [text],
    });
    return result;
}
