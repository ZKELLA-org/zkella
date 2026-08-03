#!/usr/bin/env bash
# Compile a ZKELLA circuit, run a local dev Groth16 trusted setup, and export
# its verification key. This supersedes the earlier per-circuit compile.sh
# scripts (only circuits/shield/ had one, and its hardcoded pot18 reference
# was already wrong for that circuit's real constraint count) with one
# script shared by every circuit.
#
# THIS IS A DEV CEREMONY: single local contributor, /dev/urandom entropy.
# Every artifact this produces is suitable for testnet/development use only
# — see docs/POC_IMPLEMENTATION.md for the real trusted-setup caveat that
# applies to every proof/VK in this repository so far.
#
# Prerequisites: circom 2.x, snarkjs, and a Powers of Tau file covering the
# circuit's constraint count (`.ptau/pot<N>_final.ptau`, 2^N >= constraints).
# Download one with, e.g.:
#   mkdir -p .ptau && curl -L \
#     https://hermez.s3-eu-west-1.amazonaws.com/powersOfTau28_hez_final_16.ptau \
#     -o .ptau/pot16_final.ptau
#
# Usage: circuits/build.sh <circuit> [--ptau /path/to/pot.ptau]
#   circuit: shield | unshield | transfer_2in2out | transfer_4in4out | swap | compliance
#
# Outputs into circuits/<circuit>/build/:
#   <entry>.r1cs           — R1CS constraint system
#   <entry>_js/             — WASM witness generator
#   <entry>.zkey             — Groth16 proving key (after setup + contribution)
#   verification_key.json     — Groth16 verification key (consumed by the SDK and contracts)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

CIRCUIT="${1:-}"
if [[ -z "$CIRCUIT" ]]; then
  echo "Usage: $0 <circuit> [--ptau /path/to/pot.ptau]" >&2
  echo "  circuit: shield | unshield | transfer_2in2out | transfer_4in4out | swap | compliance" >&2
  exit 1
fi
shift || true

PTAU=""
if [[ "${1:-}" == "--ptau" ]]; then
  PTAU="${2:-}"
  shift 2 || true
fi

case "$CIRCUIT" in
  shield)            ENTRY="shield" ;;
  unshield)           ENTRY="unshield" ;;
  transfer_2in2out)    ENTRY="transfer" ;;
  transfer_4in4out)     ENTRY="transfer" ;;
  swap)                  ENTRY="swap_fairness" ;;
  compliance)             ENTRY="non_membership" ;;
  *)
    echo "Unknown circuit: $CIRCUIT" >&2
    echo "  expected one of: shield unshield transfer_2in2out transfer_4in4out swap compliance" >&2
    exit 1
    ;;
esac

CIRCUIT_DIR="$REPO_ROOT/circuits/$CIRCUIT"
CIRCUIT_FILE="$CIRCUIT_DIR/$ENTRY.circom"
BUILD_DIR="$CIRCUIT_DIR/build"

if [[ ! -f "$CIRCUIT_FILE" ]]; then
  echo "Circuit source not found: $CIRCUIT_FILE" >&2
  exit 1
fi

mkdir -p "$BUILD_DIR"

echo "==> Compiling $CIRCUIT_FILE ..."
circom "$CIRCUIT_FILE" --r1cs --wasm --sym --O2 --output "$BUILD_DIR"

if [[ -z "$PTAU" ]]; then
  CONSTRAINTS="$(npx --yes snarkjs r1cs info "$BUILD_DIR/$ENTRY.r1cs" 2>/dev/null \
    | grep -i "# of Constraints" | grep -oE '[0-9]+' || echo 0)"
  echo "==> $CONSTRAINTS constraints — picking the smallest available .ptau that covers it ..."
  for size in 12 13 14 15 16 17 18 19 20; do
    candidate="$REPO_ROOT/.ptau/pot${size}_final.ptau"
    if [[ -f "$candidate" ]] && (( CONSTRAINTS < (1 << size) )); then
      PTAU="$candidate"
      break
    fi
  done
  if [[ -z "$PTAU" ]]; then
    echo "No suitable .ptau file found under $REPO_ROOT/.ptau for $CONSTRAINTS constraints." >&2
    echo "Download one covering at least $CONSTRAINTS constraints and pass it with --ptau." >&2
    exit 1
  fi
fi

if [[ ! -f "$PTAU" ]]; then
  echo "Powers of Tau file not found: $PTAU" >&2
  exit 1
fi
echo "==> Using ptau: $PTAU"

echo "==> Groth16 setup (phase 2 from ptau) ..."
npx --yes snarkjs groth16 setup "$BUILD_DIR/$ENTRY.r1cs" "$PTAU" "$BUILD_DIR/${ENTRY}_0.zkey"

echo "==> Contribute randomness (non-interactive, dev entropy only) ..."
npx --yes snarkjs zkey contribute \
  "$BUILD_DIR/${ENTRY}_0.zkey" \
  "$BUILD_DIR/${ENTRY}_1.zkey" \
  --name="ZKELLA dev contribution" \
  -e="$(head -c 32 /dev/urandom | xxd -p)"

cp "$BUILD_DIR/${ENTRY}_1.zkey" "$BUILD_DIR/$ENTRY.zkey"

echo "==> Export verification key ..."
npx --yes snarkjs zkey export verificationkey \
  "$BUILD_DIR/$ENTRY.zkey" \
  "$BUILD_DIR/verification_key.json"

echo ""
echo "Done. Artifacts in $BUILD_DIR:"
ls -lh "$BUILD_DIR"
