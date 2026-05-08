# 07 — Config Hygiene

## Problem

`deploy/railway/init-config.env` says `CHAIN_ID=13370` but the deployed node
reports chain ID `1337` (`0x539`). This creates confusion for anyone reading
the config vs querying the node.

## Root Cause

The init-config script generates validator configs, but the chain ID that
actually gets used at runtime may come from a different source — likely a
hardcoded default in the runner, a genesis config file, or command-line
argument that overrides the env var.

The canonical chain ID is **1337** (confirmed by user).

## Design

Update the config file to match reality. This is a one-line documentation fix.

```diff
- CHAIN_ID=13370
+ CHAIN_ID=1337
```

Also audit the init-config script to verify the `CHAIN_ID` env var is actually
read and propagated to the runner. If the runner ignores it and uses a
hardcoded value, that's worth noting but doesn't need to be fixed in this
workstream — the immediate issue is that the config file is wrong.

### Audit Checklist

1. Does `init-config.sh` use `$CHAIN_ID`? Where does it pass it?
2. Does `entrypoint.sh` pass chain ID to the kora binary? How?
3. Does the kora binary accept chain ID as a flag / env var?
4. Is there a genesis.json or similar that sets chain ID?

If the audit reveals that `CHAIN_ID` is dead config (never actually used by the
runtime), remove it entirely rather than leaving a misleading value.

## Files to Change

| File | Change |
|------|--------|
| `deploy/railway/init-config.env` | Change `CHAIN_ID=13370` to `CHAIN_ID=1337` |

## Verification

```bash
grep CHAIN_ID deploy/railway/init-config.env
# Expected: CHAIN_ID=1337
```
