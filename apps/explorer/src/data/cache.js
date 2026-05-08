// ================================================================
// LRU MAP
// ================================================================
/**
 * Least Recently Used cache with O(1) operations.
 *
 * Relies on Map's insertion-order iteration:
 * - get() moves the entry to the end (most recently used)
 * - set() appends to the end
 * - eviction removes from the beginning (least recently used)
 */
export class LRUMap {
    maxSize;
    map = new Map();
    constructor(maxSize) {
        this.maxSize = maxSize;
        if (maxSize < 1) {
            throw new Error(`LRUMap maxSize must be >= 1, got ${maxSize}`);
        }
    }
    // ================================================================
    // CORE OPERATIONS
    // ================================================================
    /** Retrieve a value and mark it as recently used. */
    get(key) {
        const value = this.map.get(key);
        if (value === undefined)
            return undefined;
        // Move to end (most recently used)
        this.map.delete(key);
        this.map.set(key, value);
        return value;
    }
    /** Insert or update a value. Evicts the oldest entry if at capacity. */
    set(key, value) {
        // If key exists, remove it first (resets position to end)
        if (this.map.has(key)) {
            this.map.delete(key);
        }
        this.map.set(key, value);
        // Evict oldest if over capacity
        if (this.map.size > this.maxSize) {
            const oldestKey = this.map.keys().next().value;
            if (oldestKey !== undefined) {
                this.map.delete(oldestKey);
            }
        }
    }
    /** Check if key exists. Does NOT mark as recently used. */
    has(key) {
        return this.map.has(key);
    }
    /** Remove an entry. Returns true if the entry existed. */
    delete(key) {
        return this.map.delete(key);
    }
    // ================================================================
    // INSPECTION
    // ================================================================
    /** Current number of entries. */
    get size() {
        return this.map.size;
    }
    /** Maximum capacity. */
    get capacity() {
        return this.maxSize;
    }
    /** Remove all entries. */
    clear() {
        this.map.clear();
    }
    // ================================================================
    // ITERATION (newest to oldest)
    // ================================================================
    *values() {
        const entries = [...this.map.values()];
        for (let i = entries.length - 1; i >= 0; i--) {
            yield entries[i];
        }
    }
    *entries() {
        const entries = [...this.map.entries()];
        for (let i = entries.length - 1; i >= 0; i--) {
            yield entries[i];
        }
    }
    *keys() {
        const keys = [...this.map.keys()];
        for (let i = keys.length - 1; i >= 0; i--) {
            yield keys[i];
        }
    }
}
// ================================================================
// PRE-CONFIGURED CACHES FOR THE EXPLORER
// ================================================================
/** Block cache: 500 most recent blocks */
export const blockCache = new LRUMap(500);
/** Transaction cache: 2000 most recent transactions */
export const txCache = new LRUMap(2000);
/** Receipt cache: 1000 most recent receipts */
export const receiptCache = new LRUMap(1000);
