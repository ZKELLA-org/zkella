#!/usr/bin/env bash
set -euo pipefail

# verify_release.sh
# Compare local build artifacts checksums with a published checksums file.
# Usage: ./scripts/verify_release.sh [artifacts_dir] [published_checksums_file]

ARTIFACTS_DIR=${1:-artifacts}
PUBLISHED=${2:-${ARTIFACTS_DIR}/checksums.txt}

if [ ! -d "$ARTIFACTS_DIR" ]; then
  echo "Artifacts directory not found: $ARTIFACTS_DIR" >&2
  exit 1
fi

if [ ! -f "$PUBLISHED" ]; then
  echo "Published checksums file not found: $PUBLISHED" >&2
  exit 1
fi

echo "Computing local checksums in $ARTIFACTS_DIR..."
TMPFILE=$(mktemp)
find "$ARTIFACTS_DIR" -type f \( -name '*.wasm' -o -name '*.zkey' -o -name '*.wasm.gz' -o -name '*.tar.gz' \) \
  | sort \
  | xargs sha256sum > "$TMPFILE"

echo "Comparing with published checksums: $PUBLISHED"
if diff -u "$PUBLISHED" "$TMPFILE"; then
  echo "OK: local artifacts match published checksums"
  rm -f "$TMPFILE"
  exit 0
else
  echo "ERROR: checksum mismatch between local artifacts and $PUBLISHED" >&2
  echo "See diff above."
  rm -f "$TMPFILE"
  exit 2
fi

# NOTE: On-chain verification depends on the target chain tooling (Soroban CLI / RPC).
# Example manual steps to verify deployed bytecode (fill in appropriate commands):
# 1) Get deployed contract wasm/hash from chain (using soroban or rpc)
# 2) Compare its hash to the published artifact checksum
# Example placeholder (requires soroban CLI):
# soroban contract code <CONTRACT_ADDRESS> --output wasm > deployed.wasm
# sha256sum deployed.wasm
