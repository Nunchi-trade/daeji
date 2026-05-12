#!/usr/bin/env bash
# One-click deploy of Nunchi contracts-core onto the devnet.
#
# What it does:
#   1. Clone (or update) Nunchi-trade/contracts-core into target/contracts-core
#   2. Run `forge soldeer install` and build the agents + exchange Foundry profiles
#   3. Invoke contracts-core's existing raw-RPC deployer
#      (`script/deploy_devnet_raw.py`) with paths + RPC + key overridden for this run
#   4. Tee the full deploy log to deployments/devnet-<timestamp>.log
#
# Required env vars:
#   DEPLOYER_PRIVATE_KEY   0x-prefixed hex of the EOA that signs every deploy tx
#
# Optional env vars:
#   NUNCHI_DEVNET_RPC_URL  defaults to http://localhost:8545/
#   CONTRACTS_REPO         defaults to https://github.com/Nunchi-trade/contracts-core.git
#   CONTRACTS_BRANCH       defaults to main
#   CONTRACTS_REF          if set, checks out this commit/tag instead of CONTRACTS_BRANCH
#
# Known limitation:
#   Cannon-deployed ClearingHouse + MarketRegistry (Phase B/E/F partial) are still
#   placeholders on devnet; the underlying Python deployer doesn't deploy them.
#   See script/deploy_devnet_raw.py header in contracts-core for the full list.

set -euo pipefail

: "${DEPLOYER_PRIVATE_KEY:?DEPLOYER_PRIVATE_KEY env var required (0x-prefixed hex)}"
: "${NUNCHI_DEVNET_RPC_URL:=http://localhost:8545/}"
CONTRACTS_REPO="${CONTRACTS_REPO:-https://github.com/Nunchi-trade/contracts-core.git}"
CONTRACTS_BRANCH="${CONTRACTS_BRANCH:-main}"

DAEJI_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECKOUT="${DAEJI_ROOT}/target/contracts-core"
LOG_DIR="${DAEJI_ROOT}/deployments"
TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
LOG_FILE="${LOG_DIR}/devnet-${TIMESTAMP}.log"

mkdir -p "${LOG_DIR}"

command -v forge   >/dev/null 2>&1 || { echo "error: forge not found (install foundry: https://book.getfoundry.sh/getting-started/installation)" >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "error: python3 not found" >&2; exit 1; }
command -v git     >/dev/null 2>&1 || { echo "error: git not found" >&2; exit 1; }

# Python deps required by contracts-core/script/deploy_devnet_raw.py
python3 - <<'PY' || { echo "error: install python deps first: pip install eth-account eth-utils eth-abi rlp requests" >&2; exit 1; }
import importlib, sys
for mod in ("eth_account", "eth_utils", "eth_abi", "rlp", "requests"):
    importlib.import_module(mod)
PY

# --- 1. Clone or update contracts-core ---------------------------------------
if [[ -d "${CHECKOUT}/.git" ]]; then
  echo ">>> Updating ${CHECKOUT}"
  git -C "${CHECKOUT}" fetch --tags --prune origin
  git -C "${CHECKOUT}" reset --hard "origin/${CONTRACTS_BRANCH}"
else
  echo ">>> Cloning ${CONTRACTS_REPO} -> ${CHECKOUT}"
  git clone --branch "${CONTRACTS_BRANCH}" "${CONTRACTS_REPO}" "${CHECKOUT}"
fi

if [[ -n "${CONTRACTS_REF:-}" ]]; then
  echo ">>> Checking out pinned ref ${CONTRACTS_REF}"
  git -C "${CHECKOUT}" checkout --detach "${CONTRACTS_REF}"
fi

COMMIT_SHA="$(git -C "${CHECKOUT}" rev-parse HEAD)"
echo "    contracts-core @ ${COMMIT_SHA}"

# --- 2. Foundry deps + build -------------------------------------------------
pushd "${CHECKOUT}" >/dev/null
echo ">>> forge soldeer install"
forge soldeer install
echo ">>> forge build (agents)"
FOUNDRY_PROFILE=agents   forge build --ast
echo ">>> forge build (exchange)"
FOUNDRY_PROFILE=exchange forge build --ast
popd >/dev/null

# --- 3. Stage the private key in a 600-perm tempfile -------------------------
# deploy_devnet_raw.py reads from PK_FILE, not env.
PK_TMP="$(mktemp -t deploy_devnet_pk.XXXXXX)"
SCRIPT_TMP="$(mktemp -t deploy_devnet_raw.XXXXXX.py)"
cleanup() { rm -f "${PK_TMP}" "${SCRIPT_TMP}"; }
trap cleanup EXIT
chmod 600 "${PK_TMP}"
printf '%s\n' "${DEPLOYER_PRIVATE_KEY}" > "${PK_TMP}"

# --- 4. Patch the deployer's hard-coded constants for this run --------------
python3 - "$CHECKOUT" "$NUNCHI_DEVNET_RPC_URL" "$PK_TMP" "$SCRIPT_TMP" <<'PY'
import pathlib, re, sys
checkout, rpc, pk_file, out_path = sys.argv[1:5]
src = pathlib.Path(checkout, "script", "deploy_devnet_raw.py").read_text()
subs = {
    r'^RPC\s*=.*$':           f'RPC = "{rpc}"',
    r'^PK_FILE\s*=.*$':       f'PK_FILE = "{pk_file}"',
    r'^ART_AGENTS\s*=.*$':    f'ART_AGENTS = __import__("pathlib").Path("{checkout}/out/agents")',
    r'^ART_EXCHANGE\s*=.*$':  f'ART_EXCHANGE = __import__("pathlib").Path("{checkout}/out/exchange")',
}
for pattern, replacement in subs.items():
    new_src, n = re.subn(pattern, replacement, src, count=1, flags=re.M)
    if n != 1:
        sys.exit(f"error: failed to patch pattern {pattern!r} in deploy_devnet_raw.py")
    src = new_src
pathlib.Path(out_path).write_text(src)
PY

# --- 5. Deploy ---------------------------------------------------------------
echo
echo ">>> Deploying contracts-core@${COMMIT_SHA:0:12} -> ${NUNCHI_DEVNET_RPC_URL}"
echo ">>> Log: ${LOG_FILE}"
echo
python3 "${SCRIPT_TMP}" 2>&1 | tee "${LOG_FILE}"

echo
echo ">>> Done. Full log: ${LOG_FILE}"
