# Contributing to ZKELLA

Thanks for your interest in ZKELLA — confidential finance infrastructure for Stellar/Soroban. This is a soft PoC implementation (see `README.md`'s "Repository Scope" and `docs/POC_IMPLEMENTATION.md`), so contributions that improve correctness, test coverage, and documentation accuracy are especially valuable right now.

## Before you start

For anything non-trivial (a new feature, a contract interface change, a circuit change), please open an issue first to discuss the approach — this repository's contracts and circuits are cryptographic protocol code, where a design that looks reasonable can still be unsound in a way that's expensive to discover after the fact.

For security-sensitive findings (soundness gaps, authorization bugs, anything that could affect fund safety), see `SECURITY.md` instead of opening a public issue or PR.

## Repository layout

- `contracts/` — Soroban contracts (Rust). `token` (`ShieldedToken`, the confidential token contract), `verifier` (Groth16 verifying-key registry), `governance` (timelocked key rotation), `viewing_keys`/`compliance` (disclosure), `swap` (shielded swap primitive), plus `*-interface` crates for cross-contract calls.
- `circuits/` — Circom circuits (`shield`, `unshield`, `transfer_2in2out`, `transfer_4in4out`, `swap`, `compliance`) and shared templates under `circuits/common/`.
- `sdk/` — TypeScript SDK (`@zkella/sdk`): keys, notes, proving, wallet, indexer client.
- `indexer/` — reference indexer service (Node/TypeScript, `node:sqlite`).
- `tests/` — SDK-level unit and end-to-end tests (Jest).
- `docs/` — architecture, technical spec, circuit spec, integration guide, and deployment records.

## Development setup

```bash
npm install                 # installs root + sdk workspace dependencies
npm test                    # SDK/indexer unit tests
npm run typecheck           # TypeScript type-check
cd contracts && cargo test --workspace   # Soroban contract tests
```

Building circuits requires `circom` 2.x and `snarkjs` (see `circuits/build.sh`). Building contracts to WASM requires the `wasm32v1-none` Rust target.

## Making changes

- **Contracts**: add or update tests alongside any behavior change (`contracts/<crate>/src/lib.rs`'s `#[cfg(test)] mod tests`). Run `cargo test --workspace` from `contracts/` before opening a PR. Follow the checks-effects-interactions pattern already used throughout — state updates before external token transfers or cross-contract calls.
- **Circuits**: changing a circuit's public/private signal shape is a breaking change for every contract and SDK module that constructs its witness or verifies its proof — grep for the circuit name across `contracts/`, `sdk/src/prover/`, and `docs/` before changing one.
- **SDK**: keep `sdk/src/prover/*` in sync with the circuit's actual signal names/order; a mismatch fails silently at proving time with a confusing error, not a type error.
- **Docs**: if you change a contract interface, circuit signal list, or SDK API, update the corresponding section in `docs/TECHNICAL_SPEC.md`/`docs/CIRCUIT_SPEC.md`/`docs/INTEGRATION_GUIDE.md` in the same PR — stale docs are worse than no docs for a project like this.

## Pull requests

- Keep PRs focused — one logical change per PR is easier to review for cryptographic/financial code than a large bundled change.
- Include what you tested and how (unit tests, a local Testnet transaction, etc.) — for contract/circuit changes, "it compiles" is not sufficient evidence of correctness.
- CI (`.github/workflows/ci.yml`) runs the Rust and TypeScript test suites on every PR; please make sure it's green.

## Code style

- Rust: standard `rustfmt` formatting (see `contracts/rustfmt.toml`).
- TypeScript: no comments explaining *what* code does (names should do that); comments are reserved for non-obvious *why* (a subtle invariant, a workaround, a cross-file constraint).

## License

By contributing, you agree that your contributions will be licensed under the Apache License 2.0 (see `LICENSE`).
