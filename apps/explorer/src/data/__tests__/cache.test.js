import { describe, it, expect } from 'vitest';
import { LRUMap } from '../cache';
describe('LRUMap', () => {
    it('throws on capacity < 1', () => {
        expect(() => new LRUMap(0)).toThrow();
        expect(() => new LRUMap(-1)).toThrow();
    });
    it('stores and retrieves values', () => {
        const cache = new LRUMap(3);
        cache.set('a', 1);
        cache.set('b', 2);
        cache.set('c', 3);
        expect(cache.get('a')).toBe(1);
        expect(cache.get('b')).toBe(2);
        expect(cache.get('c')).toBe(3);
        expect(cache.size).toBe(3);
    });
    it('returns undefined for missing keys', () => {
        const cache = new LRUMap(3);
        expect(cache.get('missing')).toBeUndefined();
    });
    it('evicts oldest entry when over capacity', () => {
        const cache = new LRUMap(3);
        cache.set('a', 1);
        cache.set('b', 2);
        cache.set('c', 3);
        cache.set('d', 4); // should evict 'a'
        expect(cache.has('a')).toBe(false);
        expect(cache.has('b')).toBe(true);
        expect(cache.has('c')).toBe(true);
        expect(cache.has('d')).toBe(true);
        expect(cache.size).toBe(3);
    });
    it('promotes accessed entries (get prevents eviction)', () => {
        const cache = new LRUMap(3);
        cache.set('a', 1);
        cache.set('b', 2);
        cache.set('c', 3);
        // Access 'a' to promote it
        cache.get('a');
        // Insert 'd' -- should evict 'b' (now oldest), not 'a'
        cache.set('d', 4);
        expect(cache.has('a')).toBe(true); // promoted, not evicted
        expect(cache.has('b')).toBe(false); // oldest after promotion
        expect(cache.has('c')).toBe(true);
        expect(cache.has('d')).toBe(true);
    });
    it('updates existing keys without double-counting', () => {
        const cache = new LRUMap(3);
        cache.set('a', 1);
        cache.set('b', 2);
        cache.set('a', 10); // update, not insert
        expect(cache.size).toBe(2);
        expect(cache.get('a')).toBe(10);
    });
    it('has() does not promote', () => {
        const cache = new LRUMap(3);
        cache.set('a', 1);
        cache.set('b', 2);
        cache.set('c', 3);
        // has('a') should NOT promote
        expect(cache.has('a')).toBe(true);
        // Insert 'd' -- should evict 'a' (still oldest)
        cache.set('d', 4);
        expect(cache.has('a')).toBe(false);
    });
    it('delete removes entries', () => {
        const cache = new LRUMap(3);
        cache.set('a', 1);
        cache.set('b', 2);
        expect(cache.delete('a')).toBe(true);
        expect(cache.delete('a')).toBe(false);
        expect(cache.size).toBe(1);
        expect(cache.has('a')).toBe(false);
    });
    it('clear removes all entries', () => {
        const cache = new LRUMap(3);
        cache.set('a', 1);
        cache.set('b', 2);
        cache.clear();
        expect(cache.size).toBe(0);
    });
    it('reports capacity', () => {
        const cache = new LRUMap(42);
        expect(cache.capacity).toBe(42);
    });
    it('iterates values newest to oldest', () => {
        const cache = new LRUMap(5);
        cache.set('a', 1);
        cache.set('b', 2);
        cache.set('c', 3);
        const values = [...cache.values()];
        expect(values).toEqual([3, 2, 1]);
    });
    it('iterates entries newest to oldest', () => {
        const cache = new LRUMap(5);
        cache.set('a', 1);
        cache.set('b', 2);
        cache.set('c', 3);
        const entries = [...cache.entries()];
        expect(entries).toEqual([
            ['c', 3],
            ['b', 2],
            ['a', 1],
        ]);
    });
    it('iterates keys newest to oldest', () => {
        const cache = new LRUMap(5);
        cache.set('a', 1);
        cache.set('b', 2);
        cache.set('c', 3);
        const keys = [...cache.keys()];
        expect(keys).toEqual(['c', 'b', 'a']);
    });
    it('works with bigint keys (block cache scenario)', () => {
        const cache = new LRUMap(3);
        cache.set(1n, 'block1');
        cache.set(2n, 'block2');
        cache.set(3n, 'block3');
        cache.set(4n, 'block4'); // evicts 1n
        expect(cache.has(1n)).toBe(false);
        expect(cache.get(4n)).toBe('block4');
        expect(cache.size).toBe(3);
    });
    it('handles capacity of 1', () => {
        const cache = new LRUMap(1);
        cache.set('a', 1);
        expect(cache.get('a')).toBe(1);
        cache.set('b', 2);
        expect(cache.has('a')).toBe(false);
        expect(cache.get('b')).toBe(2);
        expect(cache.size).toBe(1);
    });
});
