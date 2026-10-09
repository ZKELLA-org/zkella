<div align="center">

# ZKELLA Protocol

**ZK-native confidential finance infrastructure for the Stellar Soroban ecosystem.**

Shielded balances · Confidential transfers · Auditor viewing keys · Sanctions non-membership proofs · Private swap · Timelocked governance

[![CI](https://github.com/ZKELLA-org/zkella/actions/workflows/ci.yml/badge.svg?branch=compliance-governance-security-testnet-release)](https://github.com/ZKELLA-org/zkella/actions/workflows/ci.yml)
[![npm](https://img.shields.io/npm/v/%40zkella%2Fsdk.svg)](https://www.npmjs.com/package/@zkella/sdk)
[![Tests](https://img.shields.io/badge/tests-425%20passing-brightgreen)](#status-at-a-glance)
[![Network](https://img.shields.io/badge/network-Stellar%20Testnet-7D00FF)](#live-testnet-deployment)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)

</div>

---

## Table of contents

- [Overview](#overview)
- [Status at a glance](#status-at-a-glance)
- [Quick start](#quick-start)
- [Live Testnet deployment](#live-testnet-deployment)
- [Architecture](#architecture)
- [Components](#components)
- [Technology stack](#technology-stack)
- [Repository structure](#repository-structure)
- [Development](#development)
- [Documentation](#documentation)
- [Roadmap to mainnet](#roadmap-to-mainnet)
- [Security](#security)
- [Contributing](#contributing)
- [License](#license)

---

## Overview

Stellar is transparent by default. Protocol 25 (X-Ray) put BN254 pairing, Poseidon hashing, and Groth16 verification into the Soroban host — the cryptographic primitives for confidential finance now exist on-chain. ZKELLA is the infrastructure built on top of them: a shielded token, a compliance layer regulators can actually use, an indexer that recovers wallet state past Stellar RPC's short retention window, a private swap, and the timelocked governance to operate all of it safely over time.

Eight Soroban contracts, five Circom circuits, a TypeScript SDK, and a reference indexer — built and hardened across three development phases, each ending in a real deployment to Stellar Testnet rather than a local-only milestone. Every claim in this README is backed by a real transaction or a real, currently-passing test, linked rather than asserted — see [`docs/TRANCHE1_DELIVERABLES.md`](docs/TRANCHE1_DELIVERABLES.md) through [`docs/TRANCHE3_DELIVERABLES.md`](docs/TRANCHE3_DELIVERABLES.md) for the full record.

**What sets it apart.** Commit-reveal private swaps exist elsewhere — Railgun's Relay Adapt on Ethereum, Penumbra's protocol-native batched DEX, Aztec Connect before its 2024 shutdown — but none of that is native to Stellar/Soroban today. ZKELLA's swap reuses `ShieldedToken`'s own shield/unshield circuits directly instead of a separate bridge contract, and a relayer fronts output liquidity rather than the contract calling a DEX itself (see [`docs/TECHNICAL_SPEC.md`](docs/TECHNICAL_SPEC.md) §9). Combined with viewing-key-based selective disclosure and on-chain sanctions non-membership proofs, it's positioned as compliance-aware infrastructure other Stellar builders can wrap, not a generic shielded-token clone.

**Scope.** Testnet only — see [Roadmap to mainnet](#roadmap-to-mainnet) for exactly what stands between here and a mainnet release.

---

## Status at a glance

| | |
| --- | --- |
| **Contracts** | 8 Soroban crates — token, verifier, governance, compliance, viewing_keys, swap (+ two `#[contractclient]`-only interface crates) |
| **Circuits** | 5 Groth16/BN254 circuits (shield, transfer 2-in-2-out, transfer 4-in-4-out, unshield, swap fairness) + 1 compliance non-membership circuit |
| **Rust tests** | 237 passing, 0 failing — `cargo test --workspace --release` |
| **JS/TS tests** | 188 passing, 0 skipped, 33 suites — `npm test` (Postgres-backed indexer tests included, not mocked) |
| **Fuzz targets** | 9 `cargo-fuzz` targets, minimized corpora committed, run in CI on every push |
| **CI** | 3 jobs (contracts, SDK + circuits, fuzz smoke) on every push to `main` and this branch |
| **SDK** | [`@zkella/sdk@0.1.0`](https://www.npmjs.com/package/@zkella/sdk) published to npm |
| **Live network** | Stellar Testnet — 6 contracts deployed, addresses below |
| **Coverage** | Token crate: 99.59% regions / 99.55% lines / 92.94% functions, independently reviewed for *what* it covers, not just *how much* — see [`docs/COVERAGE.md`](docs/COVERAGE.md) |

The full, line-item record of every deliverable against its original funding success criteria, with a GitHub link and an on-chain transaction link for every claim, lives in [`docs/TRANCHE1_DELIVERABLES.md`](docs/TRANCHE1_DELIVERABLES.md), [`docs/TRANCHE2_DELIVERABLES.md`](docs/TRANCHE2_DELIVERABLES.md), and [`docs/TRANCHE3_DELIVERABLES.md`](docs/TRANCHE3_DELIVERABLES.md).

---

## Quick start

```bash
npm install @zkella/sdk@0.1.0
```

```js
const { ZKELLAKeys, ZKELLAWallet, TESTNET_CONTRACTS, TESTNET_SOROBAN_RPC } = require('@zkella/sdk')

const keys = await ZKELLAKeys.fromSeed(mySeed) // or ZKELLAKeys.generate()
const wallet = new ZKELLAWallet({
  keys: keys.spendingKey,
  network: 'testnet',
  sorobanRpc: TESTNET_SOROBAN_RPC,
  indexerUrl: 'http://localhost:8080', // run one from indexer/, see indexer/README.md
  tokenAddress: TESTNET_CONTRACTS.token,
  stellarSecret: 'S...', // the Stellar account that signs and pays for transactions
  shieldCircuit: { wasmPath: 'circuits/shield/build/shield_js/shield.wasm', zkeyPath: 'circuits/shield/build/shield.zkey' },
})

await wallet.sync()
// asset is a SEP-41 contract address, e.g. native XLM's own Stellar Asset Contract:
//   stellar contract id asset --asset native --network testnet
const { submit } = await wallet.shield({ asset: nativeAssetContract, amount: 10_000_000n })
await submit()
```

Every on-chain SDK call follows this shape: the method builds the transaction and generates the proof, `submit()` sends it and waits for confirmation. See [`docs/SDK_DEVELOPER.md`](docs/SDK_DEVELOPER.md) for the full API (`ZKELLAWallet`, `ZKELLASwap`, `ZKELLACompliance`, `ZKELLAAuditor`, `IndexerClient`) and [`examples/`](examples/) for one runnable script per flow — keys, shield, indexer query, viewing-key audit, transfer, unshield, swap.

---

## Live Testnet deployment

Current stack (`testnet_final` in [`deployments.json`](deployments.json), deployed 2026-10-08):

| Contract | Address | Explorer |
| --- | --- | --- |
| Token (`ShieldedToken`) | `CA5TFEVODC25SSEZII2XHB2XMCKFNXNLXRNFTKWPKMT5PCWYZUMLPRUZ` | [view](https://stellar.expert/explorer/testnet/contract/CA5TFEVODC25SSEZII2XHB2XMCKFNXNLXRNFTKWPKMT5PCWYZUMLPRUZ) |
| Verifier | `CC2LQPXH3L5YKRP7YJ6UIC57AOGJXBQN4DEKNRU4Y32ABXJZOENCDAX3` | [view](https://stellar.expert/explorer/testnet/contract/CC2LQPXH3L5YKRP7YJ6UIC57AOGJXBQN4DEKNRU4Y32ABXJZOENCDAX3) |
| Governance | `CDTJLTBEKBXRJJKHVI32A5UMBB4UC6VBMDOF7WR43H2SKCCRAVRJWY5Q` | [view](https://stellar.expert/explorer/testnet/contract/CDTJLTBEKBXRJJKHVI32A5UMBB4UC6VBMDOF7WR43H2SKCCRAVRJWY5Q) |
| Compliance | `CDP5SRSUFDVEYHUCUX53SM4PZVTOIHDZR3Z5C7G4TFFKAQSLX64FOZVJ` | [view](https://stellar.expert/explorer/testnet/contract/CDP5SRSUFDVEYHUCUX53SM4PZVTOIHDZR3Z5C7G4TFFKAQSLX64FOZVJ) |
| Viewing keys | `CDT776JLXU5GWRIY6WXLZGVKZ5V4TG32HAITNPFZVX5UCJSMMFHNMEEE` | [view](https://stellar.expert/explorer/testnet/contract/CDT776JLXU5GWRIY6WXLZGVKZ5V4TG32HAITNPFZVX5UCJSMMFHNMEEE) |
| Swap | `CBN7JJEPAEA5NCKOECPPGHETAK4CCCUFOBUJCDZ7K7HPIV7Y6ILOC524` | [view](https://stellar.expert/explorer/testnet/contract/CBN7JJEPAEA5NCKOECPPGHETAK4CCCUFOBUJCDZ7K7HPIV7Y6ILOC524) |

Governance is the admin of the token and verifier contracts (so pausing or rotating a verifying key goes through the timelock); this stack runs with the `testnet-fast-timelock` feature (60 ledgers, ~5 minutes), not the 7-day production timelock. The full operational detail — admin/guardian addresses, verifying-key hashes, the sanctions root, how to reach each one — is in [`docs/TESTNET_DEPLOYMENT.md`](docs/TESTNET_DEPLOYMENT.md) and [`docs/RUNBOOK.md`](docs/RUNBOOK.md).

A live, running health check (`scripts/testnet_health_check.sh`, cron every 15 minutes) watches this stack and alerts on RPC, indexer-lag, or governance misconfiguration — exercised twice: once with injected faults, once as a real indexer outage with a real alert delivered and confirmed by a person. See "Drill record" in [`docs/RUNBOOK.md`](docs/RUNBOOK.md).

---

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                     @zkella/sdk (TypeScript)                     │
│   Keys · Groth16 proving (snarkjs/WASM) · Wallet · Swap · Audit  │
└───────┬───────────────┬───────────────┬───────────────┬──────────┘
        │               │               │               │
        ▼               ▼               ▼               ▼
 ┌────────────┐  ┌─────────────┐  ┌────────────┐  ┌─────────────┐
 │  Shielded  │  │  Viewing    │  │ Compliance │  │  Shielded   │
 │  Token     │  │  Keys       │  │ (non-      │  │  Swap       │
 │  Contract  │  │  Registry   │  │ membership)│  │  Primitive  │
 └─────┬──────┘  └─────────────┘  └────────────┘  └──────┬──────┘
       │                                                  │
       │          ┌──────────────────────┐                │
       └─────────▶│  Verifier Registry   │◀───────────────┘
                  │  (Groth16/BN254 VKs) │
                  └──────────┬───────────┘
                             │ timelocked rotation
                             ▼
                  ┌──────────────────────┐
                  │     Governance       │
                  │  (timelock, pause,   │
                  │   guardian cancel)   │
                  └──────────────────────┘

                  ┌──────────────────────┐
                  │   Indexer (Node.js)  │◀── polls token events
                  │  SQLite / PostgreSQL │
                  │  bearer-auth HTTP API│──▶ wallet/auditor sync
                  └──────────────────────┘

              Soroban host: BN254 pairing · Poseidon2 · Groth16 (Protocol 25)
```

The verifier is a single shared registry for all six circuits; governance is its only admin and the token's, so a verifying-key rotation or a pause of either contract always goes through the timelock (and, for the token specifically, through governance's own forwarding call — see `docs/GOVERNANCE.md`, "Reachability"). The indexer is read-only infrastructure: it never signs transactions, only serves notes and Merkle paths the wallet and auditor need because Stellar RPC only retains events for a short window.

---

## Components

### 1. Shielded confidential token (`contracts/token`)

`ShieldedToken` wraps any SEP-41 asset (XLM, USDC, any issued token) with Pedersen-committed balances. Every spend is a Groth16 proof that the amount is valid and the sender has sufficient balance, without revealing either. `shield()` / `transfer()` (2-in-2-out) / `transfer4()` (4-in-4-out, with a relayer fee) / `unshield()` (with an optional change note), verified against Soroban's native BN254 pairing host functions.

### 2. Auditor viewing keys (`contracts/viewing_keys`)

Each wallet derives a spending key (never shared) and a viewing key, scoped to an epoch and exportable to a named auditor. Rotating the epoch revokes the old viewing key's ability to decrypt new activity without touching spending capability. `ZKELLAAuditor` in the SDK does the decrypt-on-request side: sync from a granted viewing key, recover real transaction history, resume from the last synced ledger instead of re-reporting what it already decrypted.

### 3. Compliance (`contracts/compliance`)

Sanctions-list non-membership as a ZK proof: a Poseidon2 sorted Merkle tree over a published sanctions list, with a circuit that proves an address is not a member without revealing the address. `publish_compliance_proof` verifies and stores the result on-chain before it can be relied on. Travel-Rule-style disclosure without a public counterparty list.

### 4. Shielded swap (`contracts/swap`)

A commit-reveal private swap, not just a proof-verification demo: `commit_swap` escrows the input as a real `unshield` cross-call (which also proves note ownership) and commits to the output note's randomness up front; `execute_swap` requires a relayer to front real SEP-41 liquidity; `reveal_and_claim` checks a ZK fairness proof against the committed terms, re-shields the output as a genuinely new note, and only accepts the exact output note committed at commit time — closing a real fund-destruction path a security pass found (see [`docs/TRANCHE3_DELIVERABLES.md`](docs/TRANCHE3_DELIVERABLES.md)). `cancel_swap` and `reclaim_expired_swap` refund both sides if a swap is never executed or claimed. No DEX call is built into the contract today — a relayer sources the output liquidity however it chooses, including off-chain or through the existing Stellar DEX; see `docs/TECHNICAL_SPEC.md` §9 for why, and the open question for wiring in on-chain execution directly.

### 5. Governance (`contracts/governance`)

Timelocked verifying-key rotation and admin actions (`MinShieldAmount`, asset approval, relayer allowlisting), each queued, delayed, and only then executable — with a guardian address that can cancel a queued action but not execute one early. Also the only path that can pause or unpause the verifier and token contracts, since both name governance's own contract as their admin.

### 6. Indexer and SDK (`indexer/`, `sdk/`)

The indexer solves Stellar RPC's short event-retention window: it polls the token contract, persists encrypted note commitments and Merkle state to SQLite or PostgreSQL, and serves them over a bearer-authenticated, rate-limited HTTP API — containerized, with a load test and a real outage drill behind it. `@zkella/sdk` is the TypeScript client: key derivation, real `snarkjs`/WASM Groth16 proving for every circuit, transaction construction and submission, and the wallet/swap/auditor/compliance classes listed in [Quick start](#quick-start). A browser reference wallet is not built yet — out of scope for this release, tracked as future work.

---

## Technology stack

| Layer | Technology |
| --- | --- |
| ZK proof system | Groth16 over BN254 (native Soroban host functions, Protocol 25) |
| Circuit language | Circom 2.2, compiled and proved with `snarkjs` |
| Hash function | Poseidon2 (native Soroban host function) |
| Commitment scheme | Pedersen commitments over BN254 |
| Smart contracts | Rust, Soroban SDK 27, `wasm32v1-none` |
| Client-side proving | WASM (`snarkjs`), usable in Node.js or the browser |
| Indexer | Node.js, SQLite or PostgreSQL, bearer auth + rate limiting |
| SDK | TypeScript, published as `@zkella/sdk` |
| Fuzzing | `cargo-fuzz` / `libFuzzer`, 9 targets, minimized corpora in CI |
| Coverage | `cargo-llvm-cov` |

---

## Repository structure

```
zkella/
├── circuits/            # Circom circuits: shield, unshield, transfer (2x2, 4x4), swap, compliance
├── contracts/            # Soroban/Rust workspace (8 crates)
│   ├── token/            #   ShieldedToken: shield/transfer/transfer4/unshield, Merkle + Poseidon2
│   ├── token-interface/  #   #[contractclient]-only crate for cross-contract calls into token
│   ├── verifier/         #   shared Groth16 verifying-key registry
│   ├── verifier-interface/ # #[contractclient]-only crate for cross-contract calls into verifier
│   ├── governance/       #   timelocked VK rotation, admin actions, pause forwarding, guardian cancel
│   ├── viewing_keys/     #   auditor viewing-key registry
│   ├── compliance/       #   sanctions non-membership proof storage
│   ├── swap/             #   shielded commit-reveal swap primitive
│   └── */fuzz/           #   cargo-fuzz targets + minimized corpora (on the token crate)
├── indexer/              # Node.js indexer: event sync, SQLite/PostgreSQL, HTTP API
├── sdk/                  # @zkella/sdk (TypeScript): keys, proving, wallet, swap, auditor, compliance
├── examples/             # One runnable script per flow (keys, shield, transfer, unshield, swap, audit)
├── scripts/              # Testnet deploy/validate/health-check scripts, release verification
├── tests/                # Unit and end-to-end tests (Jest)
├── deployments.json      # Live and historical Testnet deployment records
└── docs/                 # Specifications, runbook, deliverables, and audit records
```

---

## Development

```bash
# Contracts
cd contracts && cargo build --workspace --target wasm32v1-none --release
cargo test --workspace --release        # 237 tests

# SDK and indexer
npm install
npm test                                 # 188 tests (set DATABASE_URL for the PostgreSQL-backed indexer tests)
npm run typecheck

# Fuzzing (needs nightly)
cd contracts/token/fuzz && cargo +nightly fuzz run shield_arbitrary -- -max_total_time=60

# Coverage
cd contracts && cargo llvm-cov -p zkella-token --release --summary-only
```

CI (`.github/workflows/ci.yml`) runs all of this on every push to `main` and to this branch: contract tests in release mode, SDK/circuit tests and a real-browser Web Worker check against a PostgreSQL service container, and a 60-second-per-target fuzz smoke pass across all 9 targets.

---

## Documentation

**Protocol design**
[`docs/TECHNICAL_SPEC.md`](docs/TECHNICAL_SPEC.md) · [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) · [`docs/CIRCUIT_SPEC.md`](docs/CIRCUIT_SPEC.md) · [`docs/GOVERNANCE.md`](docs/GOVERNANCE.md) · [`docs/VIEWING_KEYS.md`](docs/VIEWING_KEYS.md) · [`docs/PERFORMANCE_OPTIMISATION.md`](docs/PERFORMANCE_OPTIMISATION.md)

**Operating it**
[`docs/RUNBOOK.md`](docs/RUNBOOK.md) · [`docs/TESTNET_DEPLOYMENT.md`](docs/TESTNET_DEPLOYMENT.md) · [`docs/SDK_DEVELOPER.md`](docs/SDK_DEVELOPER.md) · [`docs/SDK_RELEASE.md`](docs/SDK_RELEASE.md) · [`docs/INTEGRATION_GUIDE.md`](docs/INTEGRATION_GUIDE.md)

**Security and quality**
[`docs/COVERAGE.md`](docs/COVERAGE.md) · [`docs/SECURITY_TOOLING_REPORT.md`](docs/SECURITY_TOOLING_REPORT.md) · [`docs/SECURITY_AUDIT_TRANCHE1.md`](docs/SECURITY_AUDIT_TRANCHE1.md) · [`SECURITY.md`](SECURITY.md)

**Delivery record** (each deliverable's original description and success criteria, quoted verbatim, followed by evidence with full GitHub and on-chain links)
[`docs/TRANCHE1_DELIVERABLES.md`](docs/TRANCHE1_DELIVERABLES.md) · [`docs/TRANCHE2_DELIVERABLES.md`](docs/TRANCHE2_DELIVERABLES.md) · [`docs/TRANCHE3_DELIVERABLES.md`](docs/TRANCHE3_DELIVERABLES.md) · [`docs/POC_IMPLEMENTATION.md`](docs/POC_IMPLEMENTATION.md) · [`docs/POC_TESTNET_VALIDATION.md`](docs/POC_TESTNET_VALIDATION.md)

**Design history**
[`docs/DESIGN_EXPLORATION.md`](docs/DESIGN_EXPLORATION.md)

---

## Roadmap to mainnet

**Shipped**

- [x] Shielded token, governance, compliance, viewing keys, and shielded swap — eight contracts, 237 Rust + 188 JS/TS tests, all passing
- [x] Live, repeated Stellar Testnet deployments with real Groth16 proofs at every step, not simulated ones
- [x] Internal six-agent security pass over all three phases — found and fixed a critical swap fund-destruction path and an unreachable contract-pause gap, both now live-verified fixed on Testnet (see [`docs/TRANCHE3_DELIVERABLES.md`](docs/TRANCHE3_DELIVERABLES.md))
- [x] `@zkella/sdk` published to npm, with a running indexer, CI, fuzzing, and an operational runbook exercised in two drills (one a real indexer outage)

**Ahead**

- [ ] Independent third-party security audit — every review to date has been performed by the team building the protocol
- [ ] A real multi-party trusted-setup ceremony per circuit — the Groth16 proving keys in this repository come from a single-contributor dev ceremony, correct for Testnet/CI, not for production
- [ ] Admin multisig custody — governance, swap, and compliance currently run on single-key admins
- [ ] On-chain DEX-routed swap execution, as an alternative to relayer-fronted liquidity (see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) §1.7.5)
- [ ] A browser-based reference wallet application

---

## Security

Report vulnerabilities privately, not as a public GitHub issue — see [`SECURITY.md`](SECURITY.md) for the disclosure process and scope. No third-party audit has been completed yet; see [Roadmap to mainnet](#roadmap-to-mainnet) above.

---

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the development setup, repository layout, and PR expectations.

## License

Apache 2.0 — open for the entire Stellar ecosystem to build on. See [`LICENSE`](LICENSE).
