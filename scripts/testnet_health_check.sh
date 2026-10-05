#!/usr/bin/env bash
# Health check for the Testnet stack: RPC health, indexer health, and contract state.
#
# Exits non-zero if any check fails. Each failure is appended to LOG_FILE and,
# if NOTIFY_WEBHOOK is set, posted to it as JSON. Schedule with cron, e.g.
#   */15 * * * * cd /path/to/zkella && NOTIFY_WEBHOOK=... scripts/testnet_health_check.sh
#
# Environment:
#   SRC            identity used for read-only simulations (default zkella-testnet-deployer)
#   INDEXER_URL    indexer base URL; the indexer check is skipped when unset
#   SOROBAN_RPC    default https://soroban-testnet.stellar.org
#   LOG_FILE       default ./health-check.log
#   NOTIFY_WEBHOOK optional URL to receive failure notifications
set -uo pipefail

SRC=${SRC:-zkella-testnet-deployer}
SOROBAN_RPC=${SOROBAN_RPC:-https://soroban-testnet.stellar.org}
LOG_FILE=${LOG_FILE:-./health-check.log}
DEPLOY=${DEPLOY_JSON:-deployments.json}
failures=0

fail() {
  failures=$((failures + 1))
  local msg="$(date -u +%FT%TZ) FAIL $1"
  echo "$msg" | tee -a "$LOG_FILE" >&2
  if [ -n "${NOTIFY_WEBHOOK:-}" ]; then
    curl -s --max-time 15 -H 'Content-Type: application/json' \
      -d "$(python3 -c 'import json,sys;print(json.dumps({"text": sys.argv[1]}))' "$msg")" \
      "$NOTIFY_WEBHOOK" >/dev/null || echo "$(date -u +%FT%TZ) WARN notification delivery failed" >>"$LOG_FILE"
  fi
}

pass() { echo "$(date -u +%FT%TZ) ok $1" >>"$LOG_FILE"; }

read_addr() {
  python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['testnet_final'][sys.argv[2]])" "$DEPLOY" "$1"
}

# 1. Soroban RPC health
rpc=$(curl -s --max-time 20 -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' "$SOROBAN_RPC" \
  | python3 -c "import json,sys;print(json.load(sys.stdin).get('result',{}).get('status',''))" 2>/dev/null)
if [ "$rpc" = "healthy" ]; then pass "rpc status=$rpc"; else fail "rpc status='${rpc:-unreachable}'"; fi

# 2. Indexer health (skipped when INDEXER_URL is unset)
if [ -n "${INDEXER_URL:-}" ]; then
  lag=$(curl -s --max-time 20 "$INDEXER_URL/health" | python3 -c "import json,sys;print(json.load(sys.stdin).get('lag',''))" 2>/dev/null)
  if [ -n "$lag" ] && [ "$lag" -le 100 ] 2>/dev/null; then pass "indexer lag=$lag"; else fail "indexer lag='${lag:-unreachable}' (limit 100)"; fi
else
  echo "$(date -u +%FT%TZ) skip indexer (INDEXER_URL unset)" >>"$LOG_FILE"
fi

# 3. Contract state on the deployed stack
GOV=$(read_addr governance)
TOKEN=$(read_addr token)
NATIVE_SAC=$(stellar contract id asset --asset native --network testnet 2>/dev/null)
timelock=$(stellar contract invoke --id "$GOV" --source "$SRC" --network testnet --send=no -- timelock_ledgers 2>/dev/null | tail -1)
if [ -n "$timelock" ]; then pass "governance timelock_ledgers=$timelock"; else fail "governance timelock_ledgers unreadable"; fi
approved=$(stellar contract invoke --id "$TOKEN" --source "$SRC" --network testnet --send=no -- is_asset_approved --asset "$NATIVE_SAC" 2>/dev/null | tail -1)
if [ "$approved" = "true" ]; then pass "token native asset approved"; else fail "token native asset approval='${approved:-unreadable}'"; fi

if [ "$failures" -gt 0 ]; then
  echo "$(date -u +%FT%TZ) $failures check(s) failed" >>"$LOG_FILE"
  exit 1
fi
echo "$(date -u +%FT%TZ) all checks passed" >>"$LOG_FILE"
exit 0
