import { describe, it, expect } from "vitest";
import { truncateAddress, formatEth, formatGas, formatTimestamp, hexToNumber, formatBlockNumber, } from "./format";
describe("truncateAddress", () => {
    it("truncates a standard 42-char address", () => {
        expect(truncateAddress("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045")).toBe("0xd8dA...6045");
    });
    it("supports custom char count", () => {
        expect(truncateAddress("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045", 6)).toBe("0xd8dA6B...A96045");
    });
    it("returns short addresses unchanged", () => {
        expect(truncateAddress("0x1234", 4)).toBe("0x1234");
    });
    it("returns empty string unchanged", () => {
        expect(truncateAddress("")).toBe("");
    });
});
describe("formatEth", () => {
    it("formats wei bigint to ETH", () => {
        expect(formatEth(1000000000000000000n)).toBe("1");
    });
    it("formats wei string to ETH", () => {
        expect(formatEth("1000000000000000000")).toBe("1");
    });
    it("supports custom decimal places", () => {
        expect(formatEth(1500000000000000000n, 2)).toBe("1.5");
    });
    it("handles zero", () => {
        expect(formatEth(0n)).toBe("0");
    });
    it("formats fractional ETH values", () => {
        expect(formatEth(1234567890000000000n)).toBe("1.2345");
    });
    it("formats sub-ETH values", () => {
        expect(formatEth(500000000000000000n)).toBe("0.5");
    });
});
describe("formatGas", () => {
    it("formats gas with commas", () => {
        expect(formatGas(21000)).toBe("21,000");
    });
    it("formats large gas values", () => {
        expect(formatGas(30_000_000)).toBe("30,000,000");
    });
    it("formats bigint gas", () => {
        expect(formatGas(21000n)).toBe("21,000");
    });
    it("formats string gas", () => {
        expect(formatGas("21000")).toBe("21,000");
    });
});
describe("formatTimestamp", () => {
    it("formats a unix timestamp to locale string", () => {
        const result = formatTimestamp(0);
        // Should include "1970" (epoch)
        expect(result).toContain("1970");
    });
    it("formats a recent timestamp", () => {
        const result = formatTimestamp(1700000000);
        expect(result).toContain("2023");
    });
});
describe("hexToNumber", () => {
    it("converts 0x-prefixed hex", () => {
        expect(hexToNumber("0xff")).toBe(255);
    });
    it("converts bare hex", () => {
        expect(hexToNumber("ff")).toBe(255);
    });
    it("converts zero", () => {
        expect(hexToNumber("0x0")).toBe(0);
    });
});
describe("formatBlockNumber", () => {
    it("formats a block number with commas", () => {
        expect(formatBlockNumber(19_000_000)).toBe("19,000,000");
    });
    it("formats bigint block number", () => {
        expect(formatBlockNumber(19000000n)).toBe("19,000,000");
    });
    it("formats small number", () => {
        expect(formatBlockNumber(1)).toBe("1");
    });
});
