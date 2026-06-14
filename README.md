# ZKELLA Protocol

**ZK-native confidential finance infrastructure for the Stellar Soroban ecosystem.**

ZKELLA delivers production-oriented confidential finance infrastructure for Soroban: a CT-20 token standard, auditor viewing keys, Travel Rule-aligned disclosure workflows, persistent note indexing, and a shielded swap primitive.

---

## Overview

Stellar is transparent by default. Protocol 25 (X-Ray, January 2026) changed the underlying capability: BN254 elliptic curve host functions, Poseidon hash, and Groth16 zk-SNARK verification are now live on Soroban. The cryptographic primitives exist. What does not exist is production-grade infrastructure built on top of them.

ZKELLA fills that gap. It is a suite of open-source contracts, circuits, and developer tooling that any Soroban developer can use to add confidentiality to tokens, payments, and swaps — without rebuilding the ZK layer from scratch.

---

## Repository Scope

This repository contains the ZKELLA protocol specification, architecture, Soroban contracts, circuits, SDK modules, and tests that together define the confidential finance stack.

**Current implementation status:** the contracts and SDK code in this repository are an initial implementation for a soft PoC. They are not final protocol contracts, not mainnet-ready release artifacts, and must be reviewed, profiled, hardened, and improved before they are used in production or relied on for real user funds.

Current implementation foundation:

- `contracts/ct20` shield flow: commitment computation, duplicate-checking, Merkle insertion, and event emission
- `contracts/ct20/src/poseidon.rs` and `contracts/ct20/src/merkle.rs` for native Poseidon2 and incremental Merkle tree logic
- TypeScript SDK note construction and note encryption helpers in `sdk/src/notes`
- SDK wallet scaffolding, test vectors, and unit tests for the initial shield implementation

Planned implementation scope:

- On-chain Groth16 verifier integration and real BN254 proof verification
- Private transfer / unshield contract execution
- Viewing key / auditor disclosure registry
- Persistent note indexer and state recovery sync
- Shielded swap execution and reference wallet app
- review and improvement of all existing PoC contracts, SDK modules, and circuit integrations before any production deployment

See `docs/TECHNICAL_SPEC.md` and `docs/ARCHITECTURE.md` for the full protocol design. See `docs/POC_IMPLEMENTATION.md` for the dedicated PoC/current implementation status.

---

## Solution

ZKELLA is structured as five layered components, each independently useful and collectively forming a complete privacy infrastructure stack.

### Component 1: CT-20 Confidential Token Standard

A Soroban token standard where balances are stored as Pedersen commitments. Transfers require a Groth16 range proof — proving the amount is valid and the sender has sufficient balance without revealing either value.

- Circom circuits: 2-input/2-output and 4-input/4-output configurations
- Multi-asset support: one contract handles multiple token types simultaneously
- Wrap any existing Stellar asset (USDC, XLM, any SEP-41 token) into a CT-20 shielded version
- Functions: `shield()`, `transfer()`, `unshield()`

### Component 2: Auditor Viewing Key System

A compliance layer designed for institutional and regulated use cases.

- Each account generates a **spending key** (private) and a **viewing key** (shareable with auditors)
- Viewing key holders decrypt transaction history without spending capability
- Auditor API: regulated institutions verify counterparty transaction history on request
- Proof-of-compliance endpoint: ZK proof that an address is not on a sanctions list, using a Merkle inclusion/exclusion proof over a published sanctions list — without revealing the address
- Travel Rule-aligned: issuers can support required disclosures without exposing all counterparty data publicly

### Component 3: Persistent State Manager

Solves the 7-day Stellar RPC event retention problem that breaks the Nethermind prototype for new users.

- Lightweight indexer node operators can run to store encrypted note commitments beyond the RPC window
- Client-side WASM library that reconstructs wallet state from the indexer
- Encrypted note bundle export as user-controlled backup fallback
- This component becomes shared infrastructure for every privacy project on Stellar

### Component 4: Shielded Swap Primitive

A Stellar-native private swap interface.

- Users swap Token A for Token B through the Stellar DEX without revealing amounts on-chain
- Commit-reveal scheme: user commits to an encrypted swap intent; a relayer executes it; a ZK proof confirms the execution was fair (correct price, no front-running)
- Wraps the existing Stellar DEX — no new AMM required, usable immediately
- Scope is the primitive; a full private AMM is the next phase

### Component 5: Developer SDK and Reference Application

- TypeScript SDK (`zkella-sdk`): circuit proving (WASM), key management, Soroban calls, and note indexer in a unified API
- Reference wallet application (web, open-source): shield, transfer, unshield, and viewing key export flows
- Full documentation: circuit specifications, API reference, security assumptions, integration guides

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    zkella-sdk (TypeScript)               │
│         Key mgmt · Proof generation · Note sync         │
└────────────────────────┬────────────────────────────────┘
                         │
         ┌───────────────┼───────────────┐
         ▼               ▼               ▼
  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐
  │  CT-20      │ │  Viewing    │ │  Shielded   │
  │  Token      │ │  Key System │ │  Swap       │
  │  Contract   │ │  + Audit    │ │  Primitive  │
  └─────────────┘ └─────────────┘ └─────────────┘
         │               │               │
         └───────────────┼───────────────┘
                         ▼
              ┌─────────────────────┐
              │  Persistent State   │
              │  Manager / Indexer  │
              └─────────────────────┘
                         │
                         ▼
              ┌─────────────────────┐
              │  Soroban / BN254    │
              │  Groth16 Verifier   │
              │  (Protocol 25)      │
              └─────────────────────┘
```

---

## Technology Stack

| Layer | Technology |
|---|---|
| ZK proof system | Groth16 (via BN254 Soroban host functions) |
| Circuit language | Circom 2.0 |
| Hash function | Poseidon2 (native Soroban host function) |
| Commitment scheme | Pedersen commitments over BN254 |
| Smart contracts | Rust / Soroban SDK |
| Client proving | WASM (snarkjs) |
| SDK | TypeScript |
| Reference app | React + Stellar Wallets Kit |

---

## Repository Structure

```
ZKELLA/
├── circuits/
│   ├── transfer_2in2out/     # Circom circuit: 2-input/2-output
│   ├── transfer_4in4out/     # Circom circuit: 4-input/4-output
│   └── swap/                 # Shielded swap commit-reveal circuit
├── contracts/
│   ├── ct20/                 # Confidential token standard
│   ├── viewing_keys/         # Auditor viewing key system
│   └── swap/                 # Shielded swap primitive
├── indexer/                  # Persistent note state manager (planned)
├── sdk/                      # TypeScript zkella-sdk
├── app/                      # Reference wallet application (planned)
└── docs/                     # Specifications and guides
```

---

## License

Apache 2.0 — open for the entire Stellar ecosystem to build on.
