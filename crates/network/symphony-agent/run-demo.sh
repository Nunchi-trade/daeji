#!/usr/bin/env bash
# Spawn 3 symphony-agent processes that "talk" to each other via the chat
# layer (commonware-p2p AEAD-encrypted messages on a shared channel).
#
# Each agent:
# - has its own seed-derived ed25519 identity
# - listens on a dedicated localhost port
# - knows the others as peers (deterministic from CLI seeds)
# - cycles Hello → Status → PartialResult → Vote → Final
# - prints incoming messages from the other two agents

set -euo pipefail

AGENT_BIN="${AGENT_BIN:-$HOME/daeji/target/debug/symphony-agent}"
LOG_DIR="/tmp/symphony-agents"
mkdir -p "$LOG_DIR"

if [[ "${1:-}" == "--kill" ]]; then
  pkill symphony-agent 2>/dev/null || true
  echo "killed all symphony-agent processes"
  exit 0
fi

if [[ ! -x "$AGENT_BIN" ]]; then
  echo "building symphony-agent..."
  ( cd "$HOME/daeji" && cargo build -p symphony-agent )
fi

# Kill any prior instances.
pkill symphony-agent 2>/dev/null || true
sleep 1

# Three agents on localhost ports 4001 / 4002 / 4003 with seeds 1 / 2 / 3.
# Each agent knows the OTHER TWO as bootstrappers (forms a triangle mesh).

echo "spawning 3 symphony agents (alice, bob, carol)..."

"$AGENT_BIN" \
  --name alice \
  --seed 1 \
  --port 4001 \
  --peer-seeds "1,2,3" \
  --bootstrappers "2@127.0.0.1:4002,3@127.0.0.1:4003" \
  --interval-secs 2 \
  --max-ticks 10 \
  > "$LOG_DIR/alice.log" 2>&1 &
ALICE=$!

"$AGENT_BIN" \
  --name bob \
  --seed 2 \
  --port 4002 \
  --peer-seeds "1,2,3" \
  --bootstrappers "1@127.0.0.1:4001,3@127.0.0.1:4003" \
  --interval-secs 2 \
  --max-ticks 10 \
  > "$LOG_DIR/bob.log" 2>&1 &
BOB=$!

"$AGENT_BIN" \
  --name carol \
  --seed 3 \
  --port 4003 \
  --peer-seeds "1,2,3" \
  --bootstrappers "1@127.0.0.1:4001,2@127.0.0.1:4002" \
  --interval-secs 2 \
  --max-ticks 10 \
  > "$LOG_DIR/carol.log" 2>&1 &
CAROL=$!

echo "  alice  pid=$ALICE  port=4001  log=$LOG_DIR/alice.log"
echo "  bob    pid=$BOB    port=4002  log=$LOG_DIR/bob.log"
echo "  carol  pid=$CAROL  port=4003  log=$LOG_DIR/carol.log"
echo ""
echo "watching all three logs (ctrl-c to stop tailing — agents continue)..."
echo ""

# Tail all three with name-prefix lines for easy reading.
tail -F "$LOG_DIR/alice.log" "$LOG_DIR/bob.log" "$LOG_DIR/carol.log" 2>/dev/null
