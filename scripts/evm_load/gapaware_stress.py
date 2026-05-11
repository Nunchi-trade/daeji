#!/usr/bin/env python3
"""Gap-aware EVM deploy/call stress harness.

This harness is intentionally conservative about account nonces. It computes the
transaction hash locally before submission, treats RPC timeouts as unknown, and
does not advance to later phases while the current sender has pending receipts.
"""

import argparse
import concurrent.futures
import json
import re
import subprocess
import time
from datetime import datetime, timezone

import requests
from eth_account import Account
from eth_utils import to_checksum_address


def now():
    return datetime.now(timezone.utc).isoformat()


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc-url", default="http://127.0.0.1:8545")
    parser.add_argument("--chain-id", type=int, default=1337)
    parser.add_argument("--private-key", help="Hex private key for the funded sender")
    parser.add_argument("--private-key-file", help="File containing a hex private key; RTF wrappers are tolerated")
    parser.add_argument("--out", default="/tmp/daeji-gapaware-stress.json")
    parser.add_argument("--deploys", type=int, default=2053)
    parser.add_argument("--calls", type=int, default=5000)
    parser.add_argument("--batch-size", type=int, default=250)
    parser.add_argument("--workers", type=int, default=64)
    parser.add_argument("--gas-price", type=int, default=1_000_000_000)
    parser.add_argument("--backfill-gas-price", type=int, default=50_000_000_000)
    parser.add_argument("--receipt-timeout", type=int, default=300)
    parser.add_argument("--backfill-timeout", type=int, default=180)
    return parser.parse_args()


class Harness:
    def __init__(self, args):
        self.args = args
        self.account = Account.from_key(self.load_private_key())
        self.bytecode = self.compile_counter()

    def rpc(self, method, params=None, timeout=15):
        response = requests.post(
            self.args.rpc_url,
            json={"jsonrpc": "2.0", "id": 1, "method": method, "params": params or []},
            timeout=timeout,
        )
        response.raise_for_status()
        return response.json()

    def load_private_key(self):
        if self.args.private_key:
            return self.args.private_key
        if not self.args.private_key_file:
            raise SystemExit("pass --private-key or --private-key-file")

        text = open(self.args.private_key_file, encoding="utf-8", errors="ignore").read()
        match = re.search(r"0x[a-fA-F0-9]{64}", text)
        if match:
            return match.group(0)

        cleaned = re.sub(r"\\[a-zA-Z]+-?\d* ?", "", text)
        cleaned = re.sub(r"[{}\\\s]", "", cleaned)
        match = re.search(r"(?:0x)?[a-fA-F0-9]{64}", cleaned)
        if not match:
            raise SystemExit(f"could not parse private key from {self.args.private_key_file}")
        value = match.group(0)
        return value if value.startswith("0x") else "0x" + value

    @staticmethod
    def compile_counter():
        source = """// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;
contract StressCounter {
    uint256 public value;
    function increment() external { unchecked { value += 1; } }
    function get() external view returns (uint256) { return value; }
}
"""
        payload = {
            "language": "Solidity",
            "sources": {"StressCounter.sol": {"content": source}},
            "settings": {
                "optimizer": {"enabled": True, "runs": 1},
                "outputSelection": {"*": {"*": ["evm.bytecode.object"]}},
            },
        }
        result = subprocess.run(
            ["solc", "--standard-json"],
            input=json.dumps(payload),
            text=True,
            capture_output=True,
            check=True,
        )
        output = json.loads(result.stdout)
        errors = [err for err in output.get("errors", []) if err.get("severity") == "error"]
        if errors:
            raise RuntimeError(errors)
        return "0x" + output["contracts"]["StressCounter.sol"]["StressCounter"]["evm"]["bytecode"]["object"]

    def sign_tx(self, tx):
        signed = Account.sign_transaction(tx, self.account.key)
        raw = getattr(signed, "raw_transaction", None) or getattr(signed, "rawTransaction")
        raw_hex = "0x" + raw.hex() if isinstance(raw, (bytes, bytearray)) else raw.hex()
        tx_hash = signed.hash.hex()
        return {
            "nonce": tx["nonce"],
            "raw": raw_hex,
            "hash": "0x" + tx_hash if not tx_hash.startswith("0x") else tx_hash,
            "tx": tx,
        }

    def send_signed(self, signed):
        try:
            result = self.rpc("eth_sendRawTransaction", [signed["raw"]], timeout=20)
            if "result" in result and result["result"]:
                return {"hash": signed["hash"], "status": "accepted", "response": result}
            return {"hash": signed["hash"], "status": "rejected", "response": result}
        except Exception as exc:
            return {"hash": signed["hash"], "status": "unknown_timeout", "error": repr(exc)}

    def get_tx(self, hash_):
        try:
            return self.rpc("eth_getTransactionByHash", [hash_], timeout=10).get("result")
        except Exception:
            return None

    def resolve_unknowns(self, signed_items, send_results, retries=3):
        by_hash = {item["hash"]: item for item in signed_items}
        resolved = []
        for result in send_results:
            if result["status"] != "unknown_timeout":
                resolved.append(result)
                continue

            signed = by_hash[result["hash"]]
            known = None
            for _ in range(retries):
                known = self.get_tx(signed["hash"])
                if known:
                    resolved.append({**result, "status": "known_after_timeout", "tx_by_hash": known})
                    break
                time.sleep(2)
            if known:
                continue

            retry = self.send_signed(signed)
            retry["timeout_retry_for"] = signed["hash"]
            resolved.append(retry)
        return resolved

    def send_window(self, signed_items):
        with concurrent.futures.ThreadPoolExecutor(max_workers=self.args.workers) as executor:
            send_results = list(executor.map(self.send_signed, signed_items))
        resolved = self.resolve_unknowns(signed_items, send_results)
        known_hashes = []
        problems = []
        for item in resolved:
            if item["status"] in ("accepted", "known_after_timeout"):
                known_hashes.append(item["hash"])
            else:
                problems.append(item)
        return known_hashes, resolved, problems

    def wait_receipts(self, hashes, label, timeout):
        pending = set(hashes)
        receipts = {}
        start = time.time()
        last_log = 0
        while pending and time.time() - start < timeout:
            for hash_ in list(pending):
                try:
                    receipt = self.rpc("eth_getTransactionReceipt", [hash_], timeout=10).get("result")
                except Exception:
                    receipt = None
                if receipt:
                    receipts[hash_] = receipt
                    pending.remove(hash_)

            if time.time() - last_log >= 15 or not pending:
                head = self.rpc("eth_blockNumber", timeout=10).get("result")
                print(
                    f"{label}: receipts={len(receipts)}/{len(hashes)} pending={len(pending)} head={head}",
                    flush=True,
                )
                last_log = time.time()
            if pending:
                time.sleep(2)
        return receipts, sorted(pending), round(time.time() - start, 3)

    def submit_backfill(self, nonce):
        tx = {
            "chainId": self.args.chain_id,
            "nonce": nonce,
            "gasPrice": self.args.backfill_gas_price,
            "gas": 21_000,
            "to": to_checksum_address("0x" + "bf" * 20),
            "value": 1,
        }
        signed = self.sign_tx(tx)
        sent = self.resolve_unknowns([signed], [self.send_signed(signed)])
        receipts, pending, waited = self.wait_receipts([signed["hash"]], f"BACKFILL nonce={hex(nonce)}", self.args.backfill_timeout)
        return {
            "nonce": hex(nonce),
            "hash": signed["hash"],
            "send": sent,
            "receipt": receipts.get(signed["hash"]),
            "pending": pending,
            "wait_elapsed_s": waited,
            "tx_by_hash": self.get_tx(signed["hash"]),
            "latest_after": self.rpc("eth_getTransactionCount", [self.account.address, "latest"]),
            "pending_after": self.rpc("eth_getTransactionCount", [self.account.address, "pending"]),
        }

    @staticmethod
    def status_counts(receipts):
        counts = {}
        for receipt in receipts.values():
            status = receipt.get("status")
            counts[status] = counts.get(status, 0) + 1
        return counts

    def save(self, summary):
        summary["updated_at"] = now()
        with open(self.args.out, "w") as f:
            json.dump(summary, f, indent=2, default=str)

    def run(self):
        summary = {
            "timestamp": now(),
            "rpc": self.args.rpc_url,
            "deployer": self.account.address,
            "targets": {"deploys": self.args.deploys, "calls": self.args.calls},
            "batch_size": self.args.batch_size,
            "workers": self.args.workers,
            "preflight": {
                "chain_id": self.rpc("eth_chainId"),
                "client": self.rpc("web3_clientVersion"),
                "head": self.rpc("eth_blockNumber"),
                "latest_nonce": self.rpc("eth_getTransactionCount", [self.account.address, "latest"]),
                "pending_nonce": self.rpc("eth_getTransactionCount", [self.account.address, "pending"]),
                "balance": self.rpc("eth_getBalance", [self.account.address, "latest"]),
            },
            "deploy_batches": [],
            "call_batches": [],
            "backfills": [],
            "contracts": [],
            "stopped_reason": None,
        }
        self.save(summary)

        while len(summary["contracts"]) < self.args.deploys:
            nonce = int(self.rpc("eth_getTransactionCount", [self.account.address, "pending"])["result"], 16)
            count = min(self.args.batch_size, self.args.deploys - len(summary["contracts"]))
            signed_items = [
                self.sign_tx(
                    {
                        "chainId": self.args.chain_id,
                        "nonce": nonce + i,
                        "gasPrice": self.args.gas_price,
                        "gas": 500_000,
                        "value": 0,
                        "data": self.bytecode,
                    }
                )
                for i in range(count)
            ]
            label = f"DEPLOY batch={len(summary['deploy_batches']) + 1}"
            print(f"{label} nonce={nonce} count={count} done={len(summary['contracts'])}/{self.args.deploys}", flush=True)
            hashes, sends, problems = self.send_window(signed_items)
            if problems:
                summary["stopped_reason"] = "unresolved_send_gap_before_advancing"
                summary["send_gap"] = {"label": label, "problems": problems}
                self.save(summary)
                return summary

            receipts, pending, waited = self.wait_receipts(hashes, label, self.args.receipt_timeout)
            contracts = [
                to_checksum_address(receipt["contractAddress"])
                for receipt in receipts.values()
                if receipt.get("status") == "0x1" and receipt.get("contractAddress")
            ]
            summary["contracts"].extend(contracts)
            summary["deploy_batches"].append(
                {
                    "label": label,
                    "nonce_start": hex(nonce),
                    "submitted": len(hashes),
                    "confirmed": len(receipts),
                    "pending": pending,
                    "wait_elapsed_s": waited,
                    "status_counts": self.status_counts(receipts),
                    "contracts": len(contracts),
                }
            )
            self.save(summary)

            if pending:
                latest = int(self.rpc("eth_getTransactionCount", [self.account.address, "latest"])["result"], 16)
                summary["backfills"].append(self.submit_backfill(latest))
                summary["stopped_reason"] = "deploy_phase_pending_after_backfill"
                self.save(summary)
                return summary

        while sum(batch["confirmed"] for batch in summary["call_batches"]) < self.args.calls:
            confirmed_calls = sum(batch["confirmed"] for batch in summary["call_batches"])
            count = min(self.args.batch_size, self.args.calls - confirmed_calls)
            nonce = int(self.rpc("eth_getTransactionCount", [self.account.address, "pending"])["result"], 16)
            signed_items = [
                self.sign_tx(
                    {
                        "chainId": self.args.chain_id,
                        "nonce": nonce + i,
                        "gasPrice": self.args.gas_price,
                        "gas": 120_000,
                        "to": summary["contracts"][(confirmed_calls + i) % len(summary["contracts"])],
                        "value": 0,
                        "data": "0xd09de08a",
                    }
                )
                for i in range(count)
            ]
            label = f"CALL batch={len(summary['call_batches']) + 1}"
            print(f"{label} nonce={nonce} count={count} done={confirmed_calls}/{self.args.calls}", flush=True)
            hashes, sends, problems = self.send_window(signed_items)
            if problems:
                summary["stopped_reason"] = "unresolved_send_gap_before_advancing"
                summary["send_gap"] = {"label": label, "problems": problems}
                self.save(summary)
                return summary

            receipts, pending, waited = self.wait_receipts(hashes, label, self.args.receipt_timeout)
            summary["call_batches"].append(
                {
                    "label": label,
                    "nonce_start": hex(nonce),
                    "submitted": len(hashes),
                    "confirmed": len(receipts),
                    "pending": pending,
                    "wait_elapsed_s": waited,
                    "status_counts": self.status_counts(receipts),
                }
            )
            self.save(summary)

            if pending:
                latest = int(self.rpc("eth_getTransactionCount", [self.account.address, "latest"])["result"], 16)
                summary["backfills"].append(self.submit_backfill(latest))
                summary["stopped_reason"] = "call_phase_pending_after_backfill"
                self.save(summary)
                return summary

        summary["stopped_reason"] = "completed"
        summary["postflight"] = {
            "head": self.rpc("eth_blockNumber"),
            "latest_nonce": self.rpc("eth_getTransactionCount", [self.account.address, "latest"]),
            "pending_nonce": self.rpc("eth_getTransactionCount", [self.account.address, "pending"]),
            "balance": self.rpc("eth_getBalance", [self.account.address, "latest"]),
        }
        self.save(summary)
        return summary


def main():
    summary = Harness(parse_args()).run()
    print(json.dumps({"stopped_reason": summary["stopped_reason"], "out": summary["updated_at"]}, indent=2))


if __name__ == "__main__":
    main()
