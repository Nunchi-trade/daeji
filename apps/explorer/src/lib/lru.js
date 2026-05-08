/**
 * Generic LRU cache backed by Map's insertion-order iteration.
 * All operations are O(1).
 */
export class LRUMap {
    map = new Map();
    capacity;
    constructor(capacity) {
        if (capacity < 1)
            throw new RangeError("LRUMap capacity must be >= 1");
        this.capacity = capacity;
    }
    get size() {
        return this.map.size;
    }
    has(key) {
        return this.map.has(key);
    }
    /** Get a value, promoting it to most-recently-used. */
    get(key) {
        const value = this.map.get(key);
        if (value === undefined)
            return undefined;
        // Delete and re-insert to move to end (most recent)
        this.map.delete(key);
        this.map.set(key, value);
        return value;
    }
    /** Set a value, evicting the oldest entry if at capacity. */
    set(key, value) {
        if (this.map.has(key)) {
            this.map.delete(key);
        }
        else if (this.map.size >= this.capacity) {
            // Evict oldest (first key in iteration order)
            const oldest = this.map.keys().next().value;
            this.map.delete(oldest);
        }
        this.map.set(key, value);
        return this;
    }
    delete(key) {
        return this.map.delete(key);
    }
    clear() {
        this.map.clear();
    }
    keys() {
        return this.map.keys();
    }
    values() {
        return this.map.values();
    }
    entries() {
        return this.map.entries();
    }
}
