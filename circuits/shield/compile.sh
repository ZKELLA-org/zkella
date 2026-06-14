#!/usr/bin/env bash
# Compile the shield circuit, run trusted setup, and export verification key.
# Prerequisites: circom 2.x, snarkjs, and a Powers of Tau file (pot18_final.ptau).
#
# Usage: bash circuits/shield/compile.sh [--ptau /path/to/pot.ptau]
#
# Outputs (into circuits/shield/build/):
#   shield.r1cs          — R1CS constraint system
#   shield_js/           — WASM witness generator
#   shield.zkey          — Groth16 proving key  (after setup + contribution)
#   verification_key.json — Groth16 verification key  (imported by SDK + contract)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_DIR="$SCRIPT_DIR/build"
CIRCUIT="$SCRIPT_DIR/shield.circom"

# Default Powers of Tau file (BN254, 2^18 constraints)
PTAU="${1:-$SCRIPT_DIR/../../.ptau/pot18_final.ptau}"

if [[ ! -f "$PTAU" ]]; then
  echo "Powers of Tau file not found: $PTAU"
  echo "Download with:"
  echo "  mkdir -p .ptau && curl -L https://hermez.s3-eu-west-1.amazonaws.com/powersOfTau28_hez_final_18.ptau -o .ptau/pot18_final.ptau"
  exit 1
fi

mkdir -p "$BUILD_DIR"
cd "$BUILD_DIR"

echo "==> Compiling $CIRCUIT ..."
circom "$CIRCUIT" \
  --r1cs \
  --wasm \
  --sym \
  --O2 \
  --output "$BUILD_DIR"

echo "==> Groth16 setup (phase 2 from ptau) ..."
snarkjs groth16 setup "$BUILD_DIR/shield.r1cs" "$PTAU" "$BUILD_DIR/shield_0.zkey"

echo "==> Contribute randomness (non-interactive with /dev/urandom for dev) ..."
snarkjs zkey contribute \
  "$BUILD_DIR/shield_0.zkey" \
  "$BUILD_DIR/shield_1.zkey" \
  --name="ZKELLA dev contribution" \
  -e="$(head -c 32 /dev/urandom | xxd -p)"

echo "==> Export final zkey ..."
cp "$BUILD_DIR/shield_1.zkey" "$BUILD_DIR/shield.zkey"

echo "==> Export verification key ..."
snarkjs zkey export verificationkey \
  "$BUILD_DIR/shield.zkey" \
  "$BUILD_DIR/verification_key.json"

echo ""
echo "Done. Artifacts in $BUILD_DIR:"
ls -lh "$BUILD_DIR"
echo ""
echo "Next steps:"
echo "  1. Copy verification_key.json into contracts/ct20/verifying_key.json"
echo "  2. Run tests/e2e/shield.test.ts to validate end-to-end"
