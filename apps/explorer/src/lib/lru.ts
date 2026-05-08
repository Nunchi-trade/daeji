/**
 * Generic LRU cache backed by Map's insertion-order iteration.
 * All operations are O(1).
 */
export class LRUMap<K, V> {
  private readonly map = new Map<K, V>();
  readonly capacity: number;

  constructor(capacity: number) {
    if (capacity < 1) throw new RangeError("LRUMap capacity must be >= 1");
    this.capacity = capacity;
  }

  get size(): number {
    return this.map.size;
  }

  has(key: K): boolean {
    return this.map.has(key);
  }

  /** Get a value, promoting it to most-recently-used. */
  get(key: K): V | undefined {
    const value = this.map.get(key);
    if (value === undefined) return undefined;
    // Delete and re-insert to move to end (most recent)
    this.map.delete(key);
    this.map.set(key, value);
    return value;
  }

  /** Set a value, evicting the oldest entry if at capacity. */
  set(key: K, value: V): this {
    if (this.map.has(key)) {
      this.map.delete(key);
    } else if (this.map.size >= this.capacity) {
      // Evict oldest (first key in iteration order)
      const oldest = this.map.keys().next().value as K;
      this.map.delete(oldest);
    }
    this.map.set(key, value);
    return this;
  }

  delete(key: K): boolean {
    return this.map.delete(key);
  }

  clear(): void {
    this.map.clear();
  }

  keys(): IterableIterator<K> {
    return this.map.keys();
  }

  values(): IterableIterator<V> {
    return this.map.values();
  }

  entries(): IterableIterator<[K, V]> {
    return this.map.entries();
  }
}
