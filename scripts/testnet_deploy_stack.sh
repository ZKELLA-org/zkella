#!/usr/bin/env bash
# Deploys the six-contract ZKELLA Testnet stack and wires it together.
#
# Order matters: governance must exist before the verifier is initialized,
# because governance becomes the verifier's admin; the token's admin is
# governance so token admin changes go through the timelock.
#
# Testnet only. GOV_WASM must be built with `--features testnet-fast-timelock`
# for a demo timelock (about 5 minutes); a production build uses the standard
# 7-day timelock and must never enable that feature.
#
# Usage:
#   SRC=zkella-testnet-deployer GUARD=zkella-testnet-guardian \
#   GOV_WASM=/path/to/fast/zkella_governance.wasm \
#   [VERIFIER=C...] scripts/testnet_deploy_stack.sh
#
# Set VERIFIER to reuse an already-deployed, not-yet-initialized verifier.
set -euo pipefail

NET=testnet
SRC=${SRC:?set SRC to the deployer identity}
GUARD=${GUARD:?set GUARD to the guardian identity}
WASM_DIR=${WASM_DIR:-contracts/target/wasm32v1-none/release}
GOV_WASM=${GOV_WASM:?set GOV_WASM to the governance WASM}
TAG=${TAG:-zk-final}

ADMIN=$(stellar keys address "$SRC")
GUARDIAN=$(stellar keys address "$GUARD")

deploy() {
  local wasm=$1 alias=$2
  stellar contract deploy --wasm "$wasm" --source "$SRC" --network "$NET" --alias "$alias" 2>/dev/null | tail -1
}

invoke() {
  local id=$1; shift
  stellar contract invoke --id "$id" --source "$SRC" --network "$NET" -- "$@"
}

ledger_now() {
  curl -s "https://horizon-testnet.stellar.org/ledgers?order=desc&limit=1" \
    | python3 -c "import json,sys;print(json.load(sys.stdin)['_embedded']['records'][0]['sequence'])"
}

echo "deployer=$ADMIN guardian=$GUARDIAN"

# RESUME=1 reuses the contracts named in VERIFIER, GOV, TOKEN, SWAP, COMPLIANCE
# and VIEWING_KEYS, which must already be deployed and initialized.
if [ -z "${RESUME:-}" ]; then
  VERIFIER=${VERIFIER:-$(deploy "$WASM_DIR/zkella_verifier.wasm" "${TAG}_verifier")}
  GOV=$(deploy "$GOV_WASM" "${TAG}_governance")
  TOKEN=$(deploy "$WASM_DIR/zkella_token.wasm" "${TAG}_token")
  SWAP=$(deploy "$WASM_DIR/zkella_swap.wasm" "${TAG}_swap")
  COMPLIANCE=$(deploy "$WASM_DIR/zkella_compliance.wasm" "${TAG}_compliance")
  VIEWING_KEYS=$(deploy "$WASM_DIR/zkella_viewing_keys.wasm" "${TAG}_viewing_keys")

  echo "verifier=$VERIFIER governance=$GOV token=$TOKEN swap=$SWAP compliance=$COMPLIANCE viewing_keys=$VIEWING_KEYS"

  invoke "$VERIFIER" initialize --admin "$GOV"
  invoke "$GOV" initialize --admin "$ADMIN" --verifier "$VERIFIER" --guardian "$GUARDIAN" --token "$TOKEN"
  invoke "$TOKEN" initialize --admin "$GOV" --verifier "$VERIFIER"
  invoke "$SWAP" initialize --admin "$ADMIN" --verifier "$VERIFIER" --token_contract "$TOKEN"
  invoke "$COMPLIANCE" initialize --admin "$ADMIN" --verifier "$VERIFIER"
fi

VK_DIR=${VK_DIR:?set VK_DIR to the directory holding <circuit>.hex verifying keys}
# Root of the empty sanctions list (sentinels only), from sdk/src/prover/compliance.ts.
EMPTY_SANCTIONS_ROOT_HEX=${EMPTY_SANCTIONS_ROOT_HEX:?set EMPTY_SANCTIONS_ROOT_HEX}
queue_vk() {
  local circuit=$1 vk_name=$2
  invoke "$GOV" queue_vk_update --circuit "$circuit" --new_vk "$(cat "$VK_DIR/$vk_name.hex")"
}
queue_vk 0 shield
queue_vk 1 transfer_2in2out
queue_vk 2 unshield
queue_vk 4 transfer_4in4out
queue_vk 5 swap
queue_vk 3 compliance

QUEUED_AT=$(ledger_now)
echo "queued VK updates at ledger $QUEUED_AT; waiting for the timelock"
while [ "$(ledger_now)" -le $((QUEUED_AT + 61)) ]; do sleep 15; done

exec_vk() {
  invoke "$GOV" execute_vk_update --circuit "$1"
}
exec_vk 0
exec_vk 1
exec_vk 2
exec_vk 4
exec_vk 5
exec_vk 3

NATIVE_SAC=$(stellar contract id asset --asset native --network "$NET")
QUEUED_AT=$(ledger_now)
invoke "$GOV" queue_token_action --action "{\"AssetApproval\": [\"$NATIVE_SAC\", true]}"
while [ "$(ledger_now)" -le $((QUEUED_AT + 61)) ]; do sleep 15; done
invoke "$GOV" execute_token_action

invoke "$COMPLIANCE" set_sanctions_root --root "$EMPTY_SANCTIONS_ROOT_HEX"

echo "deployment complete"
echo "verifier=$VERIFIER"
echo "governance=$GOV"
echo "token=$TOKEN"
echo "swap=$SWAP"
echo "compliance=$COMPLIANCE"
echo "viewing_keys=$VIEWING_KEYS"
