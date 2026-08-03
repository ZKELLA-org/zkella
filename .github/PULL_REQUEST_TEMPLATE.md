## What this changes

## Why

## Testing
<!-- For contracts: `cargo test --workspace` output, and/or a real
     Testnet transaction hash if you exercised this live.
     For SDK/circuits: `npm test` / `npm run typecheck`, and which circuit
     artifacts (if any) you rebuilt and validated. -->

## Checklist
- [ ] `cargo test --workspace` passes (if `contracts/` changed)
- [ ] `npm test` and `npm run typecheck` pass (if `sdk/`, `indexer/`, or `tests/` changed)
- [ ] Docs updated in the same PR if this changes a contract interface, circuit signal shape, or SDK API (`docs/TECHNICAL_SPEC.md`, `docs/CIRCUIT_SPEC.md`, `docs/INTEGRATION_GUIDE.md`)
- [ ] No secrets, private keys, or `.env` files included
