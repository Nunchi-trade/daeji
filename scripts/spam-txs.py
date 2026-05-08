#!/usr/bin/env python3 -u
"""Sends random ETH transfers every N seconds to generate explorer activity.

Usage: python3 scripts/spam-txs.py [rpc_url] [interval_seconds]
"""

import json
import random
import sys
import time
import urllib.request
from datetime import datetime

from eth_account import Account

RPC = sys.argv[1] if len(sys.argv) > 1 else "https://kora-production-e104.up.railway.app"
INTERVAL = float(sys.argv[2]) if len(sys.argv) > 2 else 10.0
CHAIN_ID = 1337

# Loadgen keys: 32 bytes, last byte = seed (1..10)
keys = [bytes(31) + bytes([i + 1]) for i in range(10)]
accounts = [Account.from_key(k) for k in keys]

# Track nonces locally
nonces = {}


def rpc_call(method, params=None):
    payload = json.dumps({"jsonrpc": "2.0", "method": method, "params": params or [], "id": 1}).encode()
    req = urllib.request.Request(RPC, data=payload, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=10) as resp:
        result = json.loads(resp.read())
    if "error" in result:
        raise Exception(result["error"].get("message", str(result["error"])))
    return result.get("result")


def get_nonce(address):
    if address not in nonces:
        result = rpc_call("eth_getTransactionCount", [address, "latest"])
        nonces[address] = int(result, 16)
    return nonces[address]


def send_tx():
    sender_idx = random.randrange(len(accounts))
    receiver_idx = (sender_idx + 1 + random.randrange(len(accounts) - 1)) % len(accounts)

    sender = accounts[sender_idx]
    receiver = accounts[receiver_idx]

    # Random amount: 0.001 - 0.5 ETH
    amount_wei = random.randint(10**15, 5 * 10**17)
    amount_eth = amount_wei / 10**18

    nonce = get_nonce(sender.address)

    tx = {
        "to": receiver.address,
        "value": amount_wei,
        "gas": 21000,
        "gasPrice": 0,
        "nonce": nonce,
        "chainId": CHAIN_ID,
    }

    signed = sender.sign_transaction(tx)
    raw_tx = "0x" + signed.raw_transaction.hex()

    now = datetime.now().strftime("%H:%M:%S")

    try:
        tx_hash = rpc_call("eth_sendRawTransaction", [raw_tx])
        nonces[sender.address] = nonce + 1
        print(f"[{now}] {sender.address[:10]}... → {receiver.address[:10]}... | {amount_eth:.4f} ETH | {tx_hash[:18]}...")
    except Exception as e:
        # Reset nonce cache on error
        nonces.pop(sender.address, None)
        print(f"[{now}] Error: {e}")


print(f"Spamming txs every {INTERVAL}s to {RPC}")
print(f"Using {len(accounts)} accounts:")
for i, a in enumerate(accounts):
    print(f"  [{i}] {a.address}")
print("\nPress Ctrl+C to stop\n")

try:
    while True:
        send_tx()
        time.sleep(INTERVAL)
except KeyboardInterrupt:
    print("\nStopped.")
