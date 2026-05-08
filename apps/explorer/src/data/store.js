import { create } from 'zustand';
import { devtools, persist } from 'zustand/middleware';
import { LRUMap } from './cache';
import { bus } from './bus';
// ===================================================================
// CHAIN STORE
// ===================================================================
const MAX_BLOCKS = 128;
const MAX_TRANSACTIONS = 2048;
const MAX_RECEIPTS = 1024;
const MAX_ADDRESSES = 512;
const MAX_FEE_HISTORY = 64;
let activityDecayRaf = null;
let activityLevel = 0;
function decayActivity() {
    activityLevel *= 0.95;
    if (activityLevel < 0.001) {
        activityLevel = 0;
        activityDecayRaf = null;
        if (typeof document !== 'undefined') {
            document.documentElement.style.setProperty('--rd-activity', '0');
        }
        return;
    }
    if (typeof document !== 'undefined') {
        document.documentElement.style.setProperty('--rd-activity', String(activityLevel));
    }
    activityDecayRaf = requestAnimationFrame(decayActivity);
}
export const useChainStore = create()(devtools((set, get) => ({
    blocks: new Map(),
    blockOrder: [],
    latestBlock: 0n,
    transactions: new LRUMap(MAX_TRANSACTIONS),
    receipts: new LRUMap(MAX_RECEIPTS),
    addresses: new LRUMap(MAX_ADDRESSES),
    feeHistory: [],
    connectionState: 'disconnected',
    rpcLatency: 0,
    addBlock: (block) => {
        const state = get();
        const blocks = new Map(state.blocks);
        blocks.set(block.number, block);
        const blockOrder = [block.number, ...state.blockOrder];
        let latestBlock = state.latestBlock;
        if (block.number > latestBlock) {
            latestBlock = block.number;
        }
        // Ring buffer eviction
        while (blockOrder.length > MAX_BLOCKS) {
            const evicted = blockOrder.pop();
            blocks.delete(evicted);
        }
        // Index transactions from the block into the tx cache
        for (const tx of block.transactions) {
            state.transactions.set(tx.hash, tx);
        }
        // Update activity CSS variable for rose wash
        if (typeof document !== 'undefined') {
            activityLevel = block._activityLevel * 0.15;
            document.documentElement.style.setProperty('--rd-activity', String(activityLevel));
            if (activityDecayRaf !== null) {
                cancelAnimationFrame(activityDecayRaf);
            }
            activityDecayRaf = requestAnimationFrame(decayActivity);
        }
        set({ blocks, blockOrder, latestBlock });
    },
    handleReorg: (_oldHead, newHead) => {
        const state = get();
        const blocks = new Map(state.blocks);
        const blockOrder = state.blockOrder.filter((n) => {
            if (n > newHead) {
                blocks.delete(n);
                return false;
            }
            return true;
        });
        set({ blocks, blockOrder, latestBlock: newHead });
    },
    addTransaction: (tx) => {
        get().transactions.set(tx.hash, tx);
        set({});
    },
    addReceipt: (hash, receipt) => {
        get().receipts.set(hash, receipt);
        set({});
    },
    addAddressMeta: (meta) => {
        get().addresses.set(meta.address, meta);
        set({});
    },
    pushFeeData: (points) => {
        const state = get();
        let feeHistory = [...state.feeHistory, ...points];
        if (feeHistory.length > MAX_FEE_HISTORY) {
            feeHistory = feeHistory.slice(-MAX_FEE_HISTORY);
        }
        set({ feeHistory });
    },
    setConnectionState: (connectionState) => {
        set({ connectionState });
    },
    updateRpcLatency: (sample) => {
        const prev = get().rpcLatency;
        set({
            rpcLatency: prev === 0 ? sample : prev * 0.8 + sample * 0.2,
        });
    },
}), { name: 'chain-store', enabled: import.meta.env.DEV }));
// -- Chain selectors --
export const useLatestBlockNumber = () => useChainStore((s) => s.latestBlock);
export const useLatestBlock = () => useChainStore((s) => {
    if (s.latestBlock === 0n)
        return undefined;
    return s.blocks.get(s.latestBlock);
});
export const useBlock = (number) => useChainStore((s) => s.blocks.get(number));
export const useBlockOrder = () => useChainStore((s) => s.blockOrder);
export const useTransaction = (hash) => useChainStore((s) => s.transactions.get(hash));
export const useReceipt = (hash) => useChainStore((s) => s.receipts.get(hash));
export const useAddressMeta = (address) => useChainStore((s) => s.addresses.get(address));
export const useFeeHistory = () => useChainStore((s) => s.feeHistory);
export const useConnectionState = () => useChainStore((s) => s.connectionState);
export const useRpcLatency = () => useChainStore((s) => s.rpcLatency);
export function detectPerformanceTier() {
    if (typeof window === 'undefined' || typeof navigator === 'undefined') {
        return 'light';
    }
    if (/Mobi|Android/i.test(navigator.userAgent))
        return 'mobile';
    if (window.innerWidth < 760)
        return 'mobile';
    const canvas = document.createElement('canvas');
    const gl = canvas.getContext('webgl2');
    if (!gl)
        return 'light';
    const memory = navigator
        .deviceMemory;
    const effectiveMemory = memory ?? 8;
    if (effectiveMemory < 4)
        return 'light';
    const debugInfo = gl.getExtension('WEBGL_debug_renderer_info');
    if (debugInfo) {
        const renderer = gl.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL);
        const isIntegrated = /Intel|Mali|Adreno/i.test(renderer) && !/Arc/i.test(renderer);
        if (isIntegrated && effectiveMemory < 8)
            return 'standard';
    }
    canvas.width = 0;
    canvas.height = 0;
    return 'full';
}
export const useUIStore = create()(devtools(persist((set, get) => ({
    activeScene: 'terrain',
    previousScene: null,
    navigationPhase: 'idle',
    selectedEntity: null,
    searchQuery: '',
    searchResults: [],
    searchOpen: false,
    ambientMode: false,
    ambientOpacity: 1,
    paused: false,
    sidebarCollapsed: false,
    performanceTier: detectPerformanceTier(),
    setScene: (scene) => {
        const prev = get().activeScene;
        if (prev === scene)
            return;
        set({ activeScene: scene, previousScene: prev, selectedEntity: null });
    },
    selectEntity: (entity) => {
        if (entity) {
            set({
                selectedEntity: entity,
                navigationPhase: 'navigating',
                ambientMode: false,
                ambientOpacity: 1,
            });
        }
        else {
            set({ selectedEntity: null, navigationPhase: 'idle' });
        }
    },
    setSearch: (query) => {
        set({ searchQuery: query });
    },
    setSearchResults: (results) => {
        set({ searchResults: results });
    },
    setSearchOpen: (open) => {
        set({
            searchOpen: open,
            ...(open ? {} : { searchQuery: '', searchResults: [] }),
        });
    },
    toggleAmbient: () => {
        set((s) => ({
            ambientMode: !s.ambientMode,
            ambientOpacity: s.ambientMode ? 1 : 0.2,
        }));
    },
    setAmbient: (active) => {
        set({ ambientMode: active, ambientOpacity: active ? 0.2 : 1 });
    },
    toggleSearch: () => {
        const current = get().searchOpen;
        set({
            searchOpen: !current,
            ...(!current ? {} : { searchQuery: '', searchResults: [] }),
            ambientMode: false,
            ambientOpacity: 1,
        });
    },
    togglePause: () => set((s) => ({ paused: !s.paused })),
    setNavigationPhase: (phase) => set({ navigationPhase: phase }),
    clearEntity: () => set({ selectedEntity: null, navigationPhase: 'idle' }),
    toggleSidebar: () => {
        set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed }));
    },
    setPerformanceTier: (tier) => {
        set({ performanceTier: tier });
    },
}), {
    name: 'kora-explorer-ui',
    partialize: (state) => ({
        activeScene: state.activeScene,
        sidebarCollapsed: state.sidebarCollapsed,
    }),
}), { name: 'ui-store', enabled: import.meta.env.DEV }));
// -- UI selectors --
export const useActiveScene = () => useUIStore((s) => s.activeScene);
export const useSelectedEntity = () => useUIStore((s) => s.selectedEntity);
export const useSearchState = () => useUIStore((s) => ({
    query: s.searchQuery,
    results: s.searchResults,
    open: s.searchOpen,
}));
export const useAmbientMode = () => useUIStore((s) => s.ambientMode);
export const useAmbientOpacity = () => useUIStore((s) => s.ambientOpacity);
export const usePaused = () => useUIStore((s) => s.paused);
export const useSidebarCollapsed = () => useUIStore((s) => s.sidebarCollapsed);
export const usePerformanceTier = () => useUIStore((s) => s.performanceTier);
// ===================================================================
// CONSENSUS STORE
// ===================================================================
const MAX_ROUND_HISTORY = 32;
function nextPhase(event) {
    switch (event.type) {
        case 'notarize':
            return 'notarizing';
        case 'notarization':
            return 'certifying';
        case 'certification':
        case 'finalize':
            return 'finalizing';
        case 'finalization':
            return 'finalized';
        case 'nullify':
            return 'nullifying';
        case 'nullification':
            return 'nullified';
        default:
            return null;
    }
}
function stampRound(round, event) {
    switch (event.type) {
        case 'notarize':
            round.notarizeAt = event.timestampMs;
            break;
        case 'notarization':
            round.notarizationAt = event.timestampMs;
            break;
        case 'certification':
            round.certificationAt = event.timestampMs;
            break;
        case 'finalize':
            round.finalizeAt = event.timestampMs;
            break;
        case 'finalization':
            round.finalizationAt = event.timestampMs;
            break;
        case 'nullify':
            round.nullifyAt = event.timestampMs;
            break;
        case 'nullification':
            round.nullificationAt = event.timestampMs;
            break;
    }
}
function createRound(view, validatorCount) {
    return {
        view,
        leader: validatorCount > 0 ? view % validatorCount : 0,
        phase: 'proposing',
        payload: null,
        startedAt: Date.now(),
        notarizeAt: null,
        notarizationAt: null,
        certificationAt: null,
        finalizeAt: null,
        finalizationAt: null,
        nullifyAt: null,
        nullificationAt: null,
    };
}
function computeAggregates(history) {
    if (history.length === 0) {
        return { avgFinalizationMs: 0, nullificationRate: 0 };
    }
    let finalizationTotal = 0;
    let finalizationCount = 0;
    let nullifiedCount = 0;
    for (const round of history) {
        if (round.phase === 'finalized' && round.finalizationAt !== null) {
            finalizationTotal += round.finalizationAt - round.startedAt;
            finalizationCount++;
        }
        if (round.phase === 'nullified') {
            nullifiedCount++;
        }
    }
    return {
        avgFinalizationMs: finalizationCount > 0 ? finalizationTotal / finalizationCount : 0,
        nullificationRate: nullifiedCount / history.length,
    };
}
export const useConsensusStore = create()(devtools((set, get) => ({
    currentRound: null,
    roundHistory: [],
    validators: [],
    validatorCount: 0,
    threshold: 0,
    safetyEvents: [],
    unacknowledgedCount: 0,
    currentPhase: 'idle',
    currentLeader: 0,
    avgFinalizationMs: 0,
    nullificationRate: 0,
    subscribed: false,
    addEvent: (event) => {
        const state = get();
        const validatorCount = state.validatorCount;
        // Extract view from the event
        let eventView = null;
        if ('view' in event) {
            eventView = event.view;
        }
        let currentRound = state.currentRound
            ? { ...state.currentRound }
            : null;
        let roundHistory = [...state.roundHistory];
        // If no current round or event's view is ahead, start a new round.
        if (eventView !== null &&
            (currentRound === null || eventView > currentRound.view)) {
            // Archive the previous round if it existed and was past proposing.
            if (currentRound !== null && currentRound.phase !== 'proposing') {
                roundHistory = [currentRound, ...roundHistory];
                if (roundHistory.length > MAX_ROUND_HISTORY) {
                    roundHistory = roundHistory.slice(0, MAX_ROUND_HISTORY);
                }
            }
            currentRound = createRound(eventView, validatorCount);
        }
        if (currentRound === null)
            return;
        // Apply phase transition.
        const phase = nextPhase(event);
        let currentPhase = state.currentPhase;
        if (phase !== null) {
            currentRound.phase = phase;
            currentPhase = phase;
        }
        // Apply timestamp.
        stampRound(currentRound, event);
        // Set payload if the event carries one.
        if ('payload' in event && typeof event.payload === 'string') {
            currentRound.payload = event.payload;
        }
        const currentLeader = currentRound.leader;
        let avgFinalizationMs = state.avgFinalizationMs;
        let nullificationRate = state.nullificationRate;
        // On terminal phases, archive the round.
        if (phase === 'finalized' || phase === 'nullified') {
            roundHistory = [{ ...currentRound }, ...roundHistory];
            if (roundHistory.length > MAX_ROUND_HISTORY) {
                roundHistory = roundHistory.slice(0, MAX_ROUND_HISTORY);
            }
            const agg = computeAggregates(roundHistory);
            avgFinalizationMs = agg.avgFinalizationMs;
            nullificationRate = agg.nullificationRate;
        }
        set({
            currentRound,
            roundHistory,
            currentPhase,
            currentLeader,
            avgFinalizationMs,
            nullificationRate,
        });
    },
    addSafetyEvent: (event) => {
        const state = get();
        set({
            safetyEvents: [...state.safetyEvents, event],
            unacknowledgedCount: event.acknowledged
                ? state.unacknowledgedCount
                : state.unacknowledgedCount + 1,
        });
    },
    acknowledgeSafetyEvent: (index) => {
        const state = get();
        const event = state.safetyEvents[index];
        if (!event || event.acknowledged)
            return;
        const safetyEvents = [...state.safetyEvents];
        safetyEvents[index] = { ...event, acknowledged: true };
        set({
            safetyEvents,
            unacknowledgedCount: Math.max(0, state.unacknowledgedCount - 1),
        });
    },
    setValidators: (info) => {
        set({
            validators: info.validators,
            validatorCount: info.count,
            threshold: info.threshold,
        });
    },
    updateFromNodeStatus: (status) => {
        const state = get();
        const updates = {};
        // Populate validator info if not already set.
        if (state.validatorCount === 0) {
            // NodeStatus from the RPC spec has limited fields;
            // use what's available to bootstrap.
            updates.validatorCount = 1; // minimum
            updates.threshold = 1;
            updates.validators = [
                { index: status.validatorIndex, isLocal: true },
            ];
        }
        // If no WS subscription, use poll data to approximate round state.
        if (!state.subscribed) {
            const vc = updates.validatorCount ?? state.validatorCount;
            if (state.currentRound === null ||
                status.currentView > state.currentRound.view) {
                updates.currentRound = createRound(status.currentView, vc);
            }
            updates.currentLeader = vc > 0 ? status.currentView % vc : 0;
        }
        set(updates);
    },
    setSubscribed: (subscribed) => {
        set({ subscribed });
    },
}), { name: 'consensus-store', enabled: import.meta.env.DEV }));
// -- Consensus selectors --
export const useCurrentRound = () => useConsensusStore((s) => s.currentRound);
export const useCurrentPhase = () => useConsensusStore((s) => s.currentPhase);
export const useCurrentLeader = () => useConsensusStore((s) => s.currentLeader);
export const useRoundHistory = () => useConsensusStore((s) => s.roundHistory);
export const useValidators = () => useConsensusStore((s) => s.validators);
export const useSafetyEvents = () => useConsensusStore((s) => s.safetyEvents);
export const useUnacknowledgedViolations = () => useConsensusStore((s) => s.unacknowledgedCount);
export const useConsensusMetrics = () => useConsensusStore((s) => ({
    avgFinalizationMs: s.avgFinalizationMs,
    nullificationRate: s.nullificationRate,
}));
// ===================================================================
// STORE INITIALIZATION (bus -> store wiring)
// ===================================================================
const SAFETY_TYPES = new Set([
    'conflictingNotarize',
    'conflictingFinalize',
    'nullifyFinalize',
]);
let initialized = false;
export function initializeStores() {
    if (initialized)
        return;
    initialized = true;
    // Chain events
    bus.on('block:new', (block) => {
        useChainStore.getState().addBlock(block);
    });
    bus.on('block:reorg', ({ oldHead, newHead }) => {
        useChainStore.getState().handleReorg(oldHead, newHead);
    });
    bus.on('tx:new', (tx) => {
        useChainStore.getState().addTransaction(tx);
    });
    bus.on('tx:pending', (tx) => {
        useChainStore.getState().addTransaction(tx);
    });
    bus.on('tx:confirmed', ({ hash, receipt }) => {
        useChainStore.getState().addReceipt(hash, receipt);
    });
    bus.on('connection:state', (state) => {
        useChainStore.getState().setConnectionState(state);
    });
    bus.on('fee:update', (points) => {
        useChainStore.getState().pushFeeData(points);
    });
    // Consensus events
    bus.on('consensus:event', (event) => {
        useConsensusStore.getState().addEvent(event);
        if (SAFETY_TYPES.has(event.type)) {
            const safetyEvent = {
                type: event.type,
                timestampMs: event.timestampMs,
                acknowledged: false,
            };
            useConsensusStore.getState().addSafetyEvent(safetyEvent);
        }
    });
    bus.on('node:status', (status) => {
        useConsensusStore.getState().updateFromNodeStatus(status);
    });
    // Navigation / UI events
    bus.on('scene:navigate', (scene) => {
        useUIStore.getState().setScene(scene);
    });
    bus.on('entity:select', (entity) => {
        useUIStore.getState().selectEntity(entity);
    });
    bus.on('entity:deselect', () => {
        useUIStore.getState().selectEntity(null);
    });
    bus.on('search:query', (query) => {
        useUIStore.getState().setSearch(query);
    });
    bus.on('search:result', (results) => {
        useUIStore.getState().setSearchResults(results);
    });
}
