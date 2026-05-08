import { describe, it, expect } from "vitest";
import { LRUMap } from "./lru";

describe("LRUMap", () => {
  it("stores and retrieves values", () => {
    const cache = new LRUMap<string, number>(3);
    cache.set("a", 1);
    cache.set("b", 2);
    expect(cache.get("a")).toBe(1);
    expect(cache.get("b")).toBe(2);
  });

  it("returns undefined for missing keys", () => {
    const cache = new LRUMap<string, number>(3);
    expect(cache.get("x")).toBeUndefined();
  });

  it("reports size correctly", () => {
    const cache = new LRUMap<string, number>(3);
    expect(cache.size).toBe(0);
    cache.set("a", 1);
    expect(cache.size).toBe(1);
    cache.set("b", 2);
    expect(cache.size).toBe(2);
  });

  it("has() returns correct boolean", () => {
    const cache = new LRUMap<string, number>(3);
    cache.set("a", 1);
    expect(cache.has("a")).toBe(true);
    expect(cache.has("b")).toBe(false);
  });

  it("evicts oldest entry when capacity exceeded", () => {
    const cache = new LRUMap<string, number>(2);
    cache.set("a", 1);
    cache.set("b", 2);
    cache.set("c", 3); // should evict "a"
    expect(cache.has("a")).toBe(false);
    expect(cache.get("b")).toBe(2);
    expect(cache.get("c")).toBe(3);
    expect(cache.size).toBe(2);
  });

  it("get() refreshes entry order (prevents eviction)", () => {
    const cache = new LRUMap<string, number>(2);
    cache.set("a", 1);
    cache.set("b", 2);
    cache.get("a"); // refresh "a" -> now "b" is oldest
    cache.set("c", 3); // should evict "b"
    expect(cache.has("a")).toBe(true);
    expect(cache.has("b")).toBe(false);
    expect(cache.has("c")).toBe(true);
  });

  it("set() on existing key updates value and refreshes order", () => {
    const cache = new LRUMap<string, number>(2);
    cache.set("a", 1);
    cache.set("b", 2);
    cache.set("a", 10); // update "a" -> "b" is now oldest
    cache.set("c", 3); // should evict "b"
    expect(cache.get("a")).toBe(10);
    expect(cache.has("b")).toBe(false);
    expect(cache.has("c")).toBe(true);
  });

  it("delete() removes entries", () => {
    const cache = new LRUMap<string, number>(3);
    cache.set("a", 1);
    cache.set("b", 2);
    expect(cache.delete("a")).toBe(true);
    expect(cache.has("a")).toBe(false);
    expect(cache.size).toBe(1);
    expect(cache.delete("x")).toBe(false);
  });

  it("clear() removes all entries", () => {
    const cache = new LRUMap<string, number>(3);
    cache.set("a", 1);
    cache.set("b", 2);
    cache.clear();
    expect(cache.size).toBe(0);
    expect(cache.has("a")).toBe(false);
  });

  it("throws on capacity < 1", () => {
    expect(() => new LRUMap(0)).toThrow(RangeError);
    expect(() => new LRUMap(-1)).toThrow(RangeError);
  });

  it("capacity of 1 works correctly", () => {
    const cache = new LRUMap<string, number>(1);
    cache.set("a", 1);
    cache.set("b", 2);
    expect(cache.size).toBe(1);
    expect(cache.has("a")).toBe(false);
    expect(cache.get("b")).toBe(2);
  });

  it("iterates keys, values, and entries", () => {
    const cache = new LRUMap<string, number>(3);
    cache.set("a", 1);
    cache.set("b", 2);
    cache.set("c", 3);

    expect([...cache.keys()]).toEqual(["a", "b", "c"]);
    expect([...cache.values()]).toEqual([1, 2, 3]);
    expect([...cache.entries()]).toEqual([
      ["a", 1],
      ["b", 2],
      ["c", 3],
    ]);
  });
});
