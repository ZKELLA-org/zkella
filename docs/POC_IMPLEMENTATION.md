# ZKELLA PoC Implementation Status

This document is the dedicated PoC/current-implementation status note for ZKELLA. It explains the code that exists today, the protocol components already present in the repository, and the areas scheduled for completion during the delivery roadmap.

## Full specification

The full ZKELLA protocol specification is documented separately in:

- `docs/TECHNICAL_SPEC.md` — full protocol design, cryptographic primitives, circuit inputs, contract interfaces, and architecture
- `docs/CIRCUIT_SPEC.md` — circuit-level design and proof structure
- `docs/INTEGRATION_GUIDE.md` — SDK and integration details

This document does not replace the full spec. It only describes current PoC implementation status so reviewers and contributors can distinguish existing code from the remaining delivery scope.

## Current PoC implementation foundation

The current repository includes an initial shield PoC implementation foundation:

- `contracts/ct20` shield contract logic
- native Poseidon2 and Merkle tree support in Rust
- note commitment computation and duplicate-commitment protection
- incremental Merkle tree insertion and root tracking
- shielded supply accounting
- encrypted note bundle handling
- TypeScript SDK support for note construction and note encryption
- unit tests covering current computation and contract behavior

The full ZKELLA product is specified in the architecture and technical specification documents. The delivery roadmap completes the remaining proof verification, transfer, unshield, viewing-key, indexer, swap, SDK, and mainnet deployment work.

## Testnet deployment evidence

Date: June 13, 2026  
Network: Stellar Testnet (`Test SDF Network ; September 2015`)  
Deployer account: `GB2HC2NLXR7LHKXGS2IZL4F5LZVQVKRBKCWONQQW4WIYUXDILHORWQPZ`

### Deployed addresses

- Optimized CT20 PoC contract: `CCYH6YZLJBFP6QLEQIWN7NHZCVM462L6ADEENWML6OTD3VOWR4UOEMBP`
  - Lab link: `https://lab.stellar.org/r/testnet/contract/CCYH6YZLJBFP6QLEQIWN7NHZCVM462L6ADEENWML6OTD3VOWR4UOEMBP`
- Native XLM Stellar Asset Contract used for PoC shield testing: `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC`
- Earlier non-optimized CT20 deployment, superseded by the optimized instance: `CC5AXRY3PO7PBQKXTWLEL2ECVHLLVDMREZXUQIGSJZCDOIMBS5CKGAUQ`

### Successful testnet transactions

| Action | Transaction hash | Link |
| --- | --- | --- |
| Upload optimized CT20 WASM | `7f40aa87c515bc1364e22882dc82868a3043a4cdf05e8d7233ef54bb1beafbb6` | `https://stellar.expert/explorer/testnet/tx/7f40aa87c515bc1364e22882dc82868a3043a4cdf05e8d7233ef54bb1beafbb6` |
| Deploy optimized CT20 contract | `ec8e90bc04b44a3cbcfbf8e61e266ffb7843cf66712d67cef5bfa2384792d50b` | `https://stellar.expert/explorer/testnet/tx/ec8e90bc04b44a3cbcfbf8e61e266ffb7843cf66712d67cef5bfa2384792d50b` |
| Initialize CT20 with deployer admin and placeholder verifying key | `0d84883577da8aa562ed7bc9748751a48c923973b1c2960bdab1a482046c2382` | `https://stellar.expert/explorer/testnet/tx/0d84883577da8aa562ed7bc9748751a48c923973b1c2960bdab1a482046c2382` |
| Pause CT20 as admin | `9bfea719225beb0d597719ff10a90f497ec34243dacfaec6515ec26f1b5bce6a` | `https://stellar.expert/explorer/testnet/tx/9bfea719225beb0d597719ff10a90f497ec34243dacfaec6515ec26f1b5bce6a` |
| Unpause CT20 as admin | `1e14e66fe2d790b93fcbc6fa029f30b2e8c3982f8db7db0a56b075664bff281d` | `https://stellar.expert/explorer/testnet/tx/1e14e66fe2d790b93fcbc6fa029f30b2e8c3982f8db7db0a56b075664bff281d` |

### Verified live state

- `leaf_count()` on the optimized CT20 contract returned `0`.
- `shielded_supply(native XLM asset contract)` returned `0`.
- The contract is initialized and unpaused after the successful unpause transaction.

### Shield transaction finding

A valid PoC note was generated from the repository SDK using `sdk/dist/notes/builder.js` and `sdk/dist/notes/encrypt.js`:

- amount: `1000000` stroops
- asset: `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC`
- commitment: `88680fcb3c35673634c252517288f4229cbd1d51c721f170fcab363df643eb0a`
- encrypted note length: `176` bytes

Submitting `shield()` against the optimized CT20 contract failed during Stellar testnet simulation with `HostError: Error(Budget, ExceededLimit)`, even with `--instruction-leeway 100000000` and `--resource-fee 1000000`. No shield transaction hash exists because the transaction was rejected at simulation before submission.

This is an important PoC engineering finding: deployment, initialization, admin control, and simple state reads are live on testnet, but the current on-chain Poseidon/Merkle shield path needs budget profiling and optimization before public testnet shield transactions can complete reliably.

## Implemented components

### Contracts

- `contracts/ct20/src/lib.rs`
  - `initialize()` with admin, verifying key storage, and instance storage TTL handling
  - `shield()` flow: authorization, amount validation, encrypted note sizing, public-input consistency checks, commitment recomputation, duplicate commitment detection, Merkle insertion, shielded supply update, event emission, and token transfer
  - stubbed `transfer()` and `unshield()` functions returning `NotImplemented`

- `contracts/ct20/src/poseidon.rs`
  - Poseidon2 hashing implementation used by commitment and Merkle operations

- `contracts/ct20/src/merkle.rs`
  - incremental binary Merkle tree insertion with persistent storage
  - empty subtree root handling and current root computation
  - Merkle path generation and verification helpers

- `contracts/ct20/src/types.rs`
  - storage key definitions
  - public input and event definitions
  - explicit error codes including `NotImplemented`

### SDK

- `sdk/src/notes/builder.ts`
  - note generation, commitment math, and asset binding

- `sdk/src/notes/encrypt.ts`
  - encrypted note bundle encoding and decryption helpers
  - clear M2 stubs for BN254 ECDH and scalar multiplication

- `sdk/src/keys/keys.ts`
  - current key derivation and transmission key stubs
  - documented as unsafe for mainnet until BN254 scalar operations are implemented

- `sdk/src/wallet/wallet.ts`
  - partial shield flow scaffolding with placeholder transaction construction
  - stubbed Soroban RPC and proof generation behavior

### Tests and vectors

- existing unit tests in `tests/unit` validate:
  - commitment computation
  - Merkle tree insertion and root calculation
  - encryption helpers and note serialization

- `tests/e2e/shield.test.ts` is intended to demonstrate the shield flow end-to-end for the current implementation foundation

## What remains in the delivery roadmap

These capabilities are not yet implemented in the current repository and remain part of the delivery roadmap:

- on-chain Groth16 proof verification using BN254 pairing host functions (`bn254_multi_pairing_check`)
- actual shield `proof` verification in `contracts/ct20/src/lib.rs`
- BN254 `verifying_key` validation during initialization
- private `transfer()` and `unshield()` execution logic
- viewing key registry and auditor disclosure contract implementation
- persistent note indexer and note-state recovery client
- shielded swap primitive
- reference wallet application
- production key agreement and real BN254 ECDH for encrypted note transfer

## Implementation boundaries

This repository is best understood as:

- a full technical specification and architecture for the ZKELLA protocol
- an initial shield implementation foundation
- a codebase that schedules later protocol phases for delivery-roadmap completion

It is not yet a complete implementation of the full ZKELLA specification.

## How to use this document

Use this doc when you want to understand which repository files are currently implemented and which features remain in the delivery roadmap. Use `docs/TECHNICAL_SPEC.md` and `docs/ARCHITECTURE.md` for the full protocol semantics, cryptographic design, and system architecture.
