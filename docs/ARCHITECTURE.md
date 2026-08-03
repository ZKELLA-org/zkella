# ZKELLA Architecture

This document describes the full architecture of the ZKELLA protocol. It is intended as the single reference for the complete design of the system, including how on-chain contracts, off-chain proving, indexer infrastructure, client SDKs, and compliance capabilities fit together.

The repository currently contains a soft PoC implementation only. Existing contracts and SDK code validate early design assumptions, but they are not final versions of the protocol contracts and must be reviewed, optimized, hardened, and improved before any production deployment.

## 1. Product overview

ZKELLA is a confidential finance infrastructure stack for Stellar. It turns Stellar assets into shielded notes, supports private transfers and shielded swaps, and attaches auditor-friendly disclosure to shielded activity.

The product is built as a layered system:

- Soroban contracts implement the core confidential-token protocol.
- A TypeScript SDK builds notes, proofs, and Soroban transactions.
- A persistent indexer stores encrypted notes beyond the RPC retention window.
- Compliance layers support viewing keys and sanctions-aware proofs.

### 1.1 High-level product diagram

```
                             +-------------------------+
                             |      User Wallets       |
                             |  (browser / mobile /    |
                             |   desktop apps)         |
                             +-----------+-------------+
                                         |
                                         | SDK APIs, note sync,
                                         | proof generation
                                         ▼
             +---------------------------+---------------------------+
             |         zkella-sdk / client-side proving          |
             |  - note creation, encryption, viewing keys       |
             |  - Groth16 proof generation (WASM)               |
             |  - Soroban transaction assembly                 |
             +---------------------------+-------------------+
                                         |
            submit shield/transfer/unshield/swap txs
                                         ▼
             +---------------------------+-------------------+
             |      Soroban / ZKELLA On-chain Layer         |
             |  - CT-20 confidential token contract           |
             |  - viewing key registry contract              |
             |  - shielded swap contract                     |
             |  - governance / verifier management           |
             +---------------------------+-------------------+
                        ▲                  ▲            ▲
                        |                  |            |
      event stream     |                  |            | DEX settlement
                        |                  |            | / SEP-41 transfers
                        ▼                  ▼            ▼
             +-------------------+   +--------------+   +----------------+
             | zkella-indexer    |   | Stellar RPC  |   | Stellar DEX /  |
             | - note history    |   | - ledger api  |   | public swaps   |
             | - Merkle paths    |   +--------------+   +----------------+
             +-------------------+
```

### 1.2 Layered architecture diagram

```
+-----------------------------------------------------------------------+
| Application layer                                                     |
| - reference wallet, integrator apps, regulated treasury workflows      |
+-----------------------------------+-----------------------------------+
                                    |
                                    | TypeScript APIs, wallet actions
                                    v
+-----------------------------------------------------------------------+
| Client protocol layer                                                 |
| - zkella-sdk                                                          |
| - key derivation, note construction, encryption, local proof building  |
| - transaction assembly and state sync                                 |
+-----------------------------------+-----------------------------------+
                                    |
                 +------------------+------------------+
                 |                                     |
                 | Soroban transactions                | note queries
                 v                                     v
+-----------------------------------------+   +-------------------------+
| Soroban contract layer                   |   | Persistent state layer  |
| - CT-20 shielded ledger                  |   | - encrypted note store  |
| - viewing-key registry                   |   | - Merkle path service   |
| - shielded swap controller               |   | - nullifier/root cache  |
| - governance/verifier management         |   +-------------------------+
+------------------+----------------------+
                   |
                   | SEP-41 custody, DEX execution, ledger finality
                   v
+-----------------------------------------------------------------------+
| Stellar settlement layer                                               |
| - SEP-41 token contracts, Soroban RPC, Stellar DEX, ledger history     |
+-----------------------------------------------------------------------+
```

### 1.3 Product positioning

ZKELLA is not a private smart contract VM. It is an application-layer confidential finance stack that:

- preserves Stellar's public settlement guarantees,
- wraps existing SEP-41 assets into confidential notes,
- embeds compliance-aware disclosure, and
- keeps private state recoverable through a dedicated indexer.

### 1.4 Implementation maturity boundary

The architecture in this document describes the intended protocol. The current repository implementation has closed most of the gaps this section used to describe, but is still deliberately narrower than the full target design:

- shield, transfer (2-in/2-out and 4-in/4-out), and unshield are all implemented with real on-chain Groth16/BN254 verification (not stubs), validated both locally and with real transactions on live Stellar Testnet — see `docs/POC_IMPLEMENTATION.md`,
- the shielded swap contract (`contracts/swap`) genuinely moves value — real escrow via a `ct20::unshield` cross-call, real relayer-fronted liquidity, real payout and re-shield — and has been through a senior-auditor pass (three fixed issues) and a full live-Testnet run of its commit → execute → reveal-and-claim lifecycle with real circuit proofs at every stage,
- the verifying-key registry is its own contract (`contracts/verifier`), separate from `contracts/governance` (which timelocks key rotation) — this split didn't exist when this document was first written,
- `contracts/viewing_keys` (viewing-key commitment registry) and `contracts/compliance` (sanctions non-membership proofs, verified against the real verifier) are now two separate contracts, not one,
- the persistent indexer (`indexer/`) is a real, running TypeScript/Node service — not design-only — validated against live Stellar Testnet event data (see `docs/POC_IMPLEMENTATION.md` and `indexer/README.md`),
- the SDK's cryptographic core (`sdk/src/keys`, `sdk/src/notes`, `sdk/src/crypto`, `sdk/src/prover`) and the base wallet flows (`shield()`/`transfer()`/`unshield()` in `sdk/src/wallet/wallet.ts`) are real, exercised against real Soroban RPC — but the higher-level `ZKELLASwap`, `ZKELLAAuditor`, and `ZKELLACompliance` wrapper classes are still stubs (their methods return placeholder values; the underlying Soroban contracts they'd call are real, just not yet wired up at the SDK convenience-API layer),
- all existing contract and SDK modules still require an *external, independent* security review, a real (non-dev, multi-party) trusted-setup ceremony, and further hardening before finalization — the audit work documented in this repository so far was performed by the team building the protocol, not a third party,
- any deployed testnet addresses should be interpreted as evidence of a working implementation, not canonical, permanent, or production infrastructure — they are redeployed when circuit/contract changes require it (see `deployments.json` for the current set).

This boundary is important for contributors: the repository should be read as a technical specification plus a substantially real, but not yet externally audited or production-hardened, implementation — not as a finished protocol release.

### 1.5 Technical stack detail

| Layer | Primary technology | Responsibility |
|---|---|---|
| Settlement | Stellar ledger, SEP-41 assets, Stellar DEX | Public asset custody, final settlement, public liquidity, public account state |
| Smart contracts | Rust, Soroban SDK, Soroban instance/persistent storage | CT-20 shielded ledger, nullifier registry, Merkle roots, viewing-key registry, swap controller, verifier governance |
| ZK verification | Groth16, BN254 host functions, Poseidon/Poseidon2 | Proof verification, commitment hashing, Merkle authentication, value conservation checks |
| Circuits | Circom 2.2, snarkjs artifact pipeline | Shield, transfer (2x2, 4x4), unshield, swap fairness, and sanctions non-membership constraints |
| Client proving | TypeScript, WASM proving artifacts (snarkjs), Node/browser runtimes | Local witness generation, proof construction, note encryption, transaction assembly |
| State recovery | Indexer service (Node/TypeScript), `node:sqlite`, Soroban RPC event ingestion | Long-lived encrypted note history, Merkle path serving, wallet state reconstruction |
| Application | Reference wallet and external integrations | User flows for shield, transfer, unshield, viewing-key export, and shielded swap |
| Operations | Governance contract, verifier-key lifecycle, monitoring | verifier updates, pause controls, deployment runbooks, health checks |

### 1.6 Data and control-plane overview

```
Control plane:

  Governance admin
        |
        | verifier updates, pause controls, relayer permissions
        v
  Governance contract --------------------+
        |                                  |
        v                                  v
  CT-20 contract                  Shielded swap contract

Data plane:

  Wallet/SDK
     | 1. build notes + proofs
     v
  Soroban RPC
     | 2. submit shield/transfer/unshield/swap tx
     v
  CT-20 / swap contracts
     | 3. emit commitments, nullifiers, encrypted-note events
     v
  Indexer
     | 4. serve note history + Merkle paths
     v
  Wallet/SDK
```

### 1.7 Stellar integration

ZKELLA is planned as a native Stellar application layer, not a fork, bridge, sidechain, or separate execution environment. Stellar remains the settlement layer for public assets and liquidity; ZKELLA adds a Soroban-based confidentiality layer that wraps public asset balances into private note commitments.

The planned integration has six concrete touchpoints:

| Stellar surface | Planned ZKELLA use |
|---|---|
| SEP-41 token contracts | Source and sink for public assets entering or leaving CT-20 custody |
| Soroban contracts | Execution layer for CT-20, viewing-key registry, shielded swap controller, and verifier governance |
| Soroban host functions | Native BN254 and Poseidon/Poseidon2 operations for proof verification and commitment/Merkle hashing |
| Soroban RPC | Transaction submission, simulation, state reads, event streaming, and wallet/indexer synchronization |
| Stellar DEX | Public liquidity and execution venue for shielded swap settlement |
| Stellar ledger history | Finality anchor, event ordering source, trusted setup beacon source, and recovery checkpoint source |

This integration is still planned architecture. The current repository contains only soft PoC contracts and SDK scaffolding, so every integration point below must be reviewed and implemented before it is treated as final.

#### 1.7.1 SEP-41 asset custody model

CT-20 is designed to wrap existing Stellar assets without creating a separate asset universe. A shielded note is backed by real SEP-41 token units held or controlled by the CT-20 contract.

Planned custody flow:

1. User selects a SEP-41 asset contract, such as XLM's Stellar Asset Contract or a stablecoin asset contract.
2. Wallet/SDK builds a shield note with `asset_id = SEP-41 contract address`.
3. Public SEP-41 units move into CT-20 custody during `shield()`.
4. CT-20 records only the note commitment, encrypted note payload, asset identifier, and shielded supply accounting.
5. During `unshield()`, CT-20 verifies the spend proof, marks the nullifier as spent, and releases SEP-41 units to a public Stellar address.

```
Public Stellar balance
        |
        | SEP-41 transfer / contract invocation
        v
+---------------------+        private note commitment
| CT-20 custody       | --------------------------------+
| - asset balance     |                                 |
| - shielded supply   |                                 v
+----------+----------+                       +------------------+
           |                                  | Wallet note set  |
           | unshield proof                   | - value          |
           v                                  | - asset_id       |
Public Stellar balance                        | - rho / rcm      |
                                              +------------------+
```

Custody invariants:

- shielded supply for each asset must never exceed the CT-20 contract's backing SEP-41 balance,
- each note commitment must bind `asset_id` so notes cannot be replayed across assets,
- unshield must reveal enough public information to release the correct asset and amount while preserving private history,
- token custody checks must be hardened before final contracts are deployed.

#### 1.7.2 Soroban contract integration

The planned on-chain deployment is a set of specialized Soroban contracts:

| Contract | Stellar/Soroban dependency | Responsibility |
|---|---|---|
| CT-20 (`contracts/ct20`) | SEP-41 token interface, Soroban storage, BN254/Poseidon host functions | shield, transfer, unshield, nullifier tracking, Merkle root management, shielded supply accounting |
| Viewing key registry (`contracts/viewing_keys`) | Soroban storage and events | viewing-key commitment registration only |
| Compliance (`contracts/compliance`) | Cross-calls `contracts/verifier` | verified sanctions non-membership proof storage/retrieval |
| Shielded swap (`contracts/swap`) | `ct20`/`verifier` cross-calls, SEP-41 token transfers, relayer authorization | real escrow via `ct20::unshield`, relayer-fronted liquidity, fairness-proof-gated payout and re-shield, cancellation/reclaim paths |
| Verifier (`contracts/verifier`) | BN254 pairing/Poseidon host functions | shared Groth16 verifying-key storage and the `verify()` entrypoint every other contract calls |
| Governance (`contracts/governance`) | Cross-calls `contracts/verifier` | timelocked verifying-key rotation, admin transfer — is `verifier`'s own admin |

Soroban storage plan:

- instance storage: admin configuration, verifier key references, current Merkle root, pause state, contract metadata,
- persistent storage: note commitments, spent nullifiers, historical roots, shielded supply per asset, swap state, viewing-key commitments,
- event stream: encrypted notes, note indexes, nullifier events, root updates, viewing-key events, swap lifecycle events.

TTL and rent considerations:

- persistent entries used for nullifiers and historical Merkle roots must remain available for the intended protocol lifetime,
- wallet and indexer code must monitor TTL extension requirements where applicable,
- final contracts must define explicit storage retention policy for roots, note commitments, swap states, and registry entries.

#### 1.7.3 Protocol 25 and native cryptography

ZKELLA relies on Stellar's Soroban cryptographic host functions rather than implementing expensive curve logic in WASM contract code.

Planned usage:

- `bn254_multi_pairing_check` for Groth16 verifier equations,
- BN254 G1/G2 decoding and validation for proof and verifying-key material,
- Poseidon/Poseidon2 hashing for note commitments, nullifiers, Merkle tree nodes, and swap intent commitments,
- native host execution to keep verification within Soroban resource limits.

Contract review must confirm:

- proof byte formats are canonical and rejected on malformed input,
- verifying keys are versioned per circuit and cannot be confused across shield, transfer, unshield, swap, or compliance circuits,
- public inputs are ordered identically in Circom, SDK witness generation, and Soroban verification,
- resource cost remains acceptable under realistic Merkle depth, event, and storage workloads.

#### 1.7.4 Soroban RPC and indexer integration

Soroban RPC is the online interface for wallets and the ingestion source for the persistent indexer.

```
Wallet / SDK
   | simulateTransaction
   | sendTransaction
   | getTransaction / getLedgerEntries
   v
Soroban RPC
   | contract events
   | ledger cursors
   v
ZKELLA indexer
   | encrypted notes
   | Merkle paths
   | root and nullifier status
   v
Wallet / auditor clients
```

Planned wallet RPC usage:

- simulate transactions before submission to estimate resource fees and catch invalid proofs,
- submit shield, transfer, unshield, viewing-key, and swap transactions,
- read current Merkle root, shielded supply, verifier key metadata, and pause status,
- query transaction status and ledger inclusion.

Planned indexer RPC usage:

- consume CT-20 and registry events from a configured start ledger,
- persist encrypted note bundles and their leaf indexes,
- reconstruct incremental Merkle paths from event order and contract roots,
- expose root, note, nullifier, and health APIs to wallets,
- support `birthday_ledger` sync so wallets do not scan irrelevant history.

Indexer verification requirements:

- indexer-served Merkle paths must be independently checked by the wallet against an on-chain root,
- indexer state must be replayable from Soroban events,
- event schemas must be stable and versioned before final release,
- multiple indexers should be able to serve the same note state without becoming trusted authorities.

The indexer is intentionally a purpose-built recovery and state-sync service rather than a generic analytics indexer. It exists because Stellar RPC retention is too short for a privacy protocol that must preserve note history, encrypted bundles, and Merkle authentication paths after the public event window has closed. The design therefore assumes multiple independent operators, replay from ledger cursors, and wallet-side verification of every served path and note bundle.

#### 1.7.5 Stellar DEX integration for shielded swaps

ZKELLA does not plan to replace the Stellar DEX. The shielded swap primitive uses Stellar's public liquidity while hiding the user's private note history and target shielded output.

Planned swap architecture:

```
Input shielded note
        |
        | commit_swap(intent_commitment, nullifier, expiry)
        v
+-----------------------+
| Shielded swap state   |
+-----------+-----------+
            |
            | relayer executes public Stellar DEX operation
            v
+-----------------------+        execution report
| Stellar DEX           | -------------------------+
| path payment / offer  |                          |
+-----------------------+                          v
                                      +--------------------------+
                                      | reveal_and_claim proof   |
                                      | - intent matches         |
                                      | - execution fair         |
                                      | - output note valid      |
                                      +------------+-------------+
                                                   |
                                                   v
                                        Output shielded note
```

Planned DEX execution options:

- path payment flow for routing through available Stellar liquidity,
- offer-management flow where a relayer or solver executes a quoted trade,
- explicit slippage bounds in the private intent,
- expiry ledger to prevent indefinite note locking,
- cancellation flow if no valid execution is reported before expiry.

Final design questions that must be resolved before production:

- exact representation of DEX execution evidence inside the fairness circuit,
- which Stellar operation types are supported in the first release,
- whether relayer execution is permissioned, permissionless, or governed,
- how reference prices and slippage constraints are encoded and verified,
- how failed or partial public execution is handled without compromising private state.

#### 1.7.6 Ledger ordering, events, and finality

ZKELLA depends on Stellar ledger ordering for deterministic note history.

Planned event ordering rules:

- each accepted note commitment receives a deterministic leaf index,
- the emitted event includes enough data for indexers to reconstruct insertion order,
- nullifier events are emitted when notes are spent,
- root update events allow clients to match indexer state against contract state,
- viewing-key and swap events include versioned payloads so future clients can parse them safely.

Finality and recovery assumptions:

- wallet clients should treat notes as usable only after transaction success is confirmed through Soroban RPC,
- indexers should advance by ledger cursor and be able to resume after downtime,
- encrypted note recovery should be possible from indexer history plus wallet keys,
- if an indexer is unavailable, wallets can still check critical on-chain state but may need another indexer or backup bundle for full note reconstruction.

#### 1.7.7 Deployment topology on Stellar

Planned testnet deployment:

- deploy PoC and reviewed testnet CT-20 contracts to Stellar Testnet,
- publish contract IDs, WASM hashes, verifier key versions, and supported asset IDs,
- operate a public testnet indexer,
- document known limitations and resource-budget findings for each deployment.

Planned mainnet deployment:

- deploy only after contract review, circuit review, trusted setup, and resource profiling are complete,
- publish immutable release metadata for contract WASM, verifying keys, circuit artifacts, and SDK versions,
- configure governance controls for verifier-key rotation and emergency pause,
- monitor event ingestion, RPC lag, indexer health, and transaction failure reasons.

The architecture assumes Stellar wallets, assets, and DEX liquidity remain the primary public layer; ZKELLA adds a confidentiality layer on top of that settlement fabric.

## 2. Core components

The target architecture is composed of eight primary components:

1. CT-20 confidential token contract
2. Verifier registry contract (Groth16 verifying-key storage and the shared `verify()` entrypoint)
3. Governance contract (timelocked verifying-key rotation, relayer/admin controls)
4. Viewing key registry contract
5. Compliance contract (sanctions non-membership proofs, verified against the verifier registry)
6. Shielded swap contract
7. zkella-sdk and off-chain prover
8. Persistent indexer and wallet sync service

The verifier and governance contracts were originally described as one combined "governance/verifier management" component; the repository implementation splits them so the verifying-key registry (`contracts/verifier`) can be reused by every other contract (`ct20`, `swap`, `compliance`) as a plain cross-contract call, while `contracts/governance` owns only the timelocked rotation policy on top of it. Likewise, viewing-key registration and sanctions-proof publication were originally one component; the repository splits them into `contracts/viewing_keys` and `contracts/compliance` so an unrelated compliance-record store doesn't share a contract with the viewing-key registry.

Each component is described below.

### 2.0 Component interaction map

```
                                +----------------------+
                                | Governance / verifier|
                                | management           |
                                +----------+-----------+
                                           |
                      verifier keys, pause | controls, relayer list
                                           v
+-------------+     proofs + txs    +------+-------+     events      +---------------+
| Wallet / SDK | -----------------> | CT-20 ledger | --------------> | Indexer       |
|             | <----------------- |              | <-------------- | Merkle paths  |
+------+------+  roots, balances   +------+-------+  note history   +-------+-------+
       |                                  |                          |
       | viewing-key commitments          | SEP-41 custody           | decrypted
       v                                  v                          | permitted data
+------+------+                   +-------+------+                   v
| Viewing key |                   | Stellar      |          +--------+-------+
| registry    |                   | public layer |          | Auditor /     |
+-------------+                   +--------------+          | integrator    |
       ^
       |
       | disclosure proofs
+------+------+
| Compliance  |
| workflows   |
+-------------+

Shielded swaps extend the CT-20 ledger through a swap controller that locks input
nullifiers, references public DEX execution, and mints verified shielded outputs.
```

### 2.1 CT-20 confidential token contract

The CT-20 contract (`contracts/ct20`) is the core shielded ledger on Soroban. Shield, transfer (2-in/2-out and 4-in/4-out), and unshield are all implemented with real on-chain Groth16 verification, cross-calling `contracts/verifier`.

It handles:

- `shield()`/`transfer()`/`transfer4()`/`unshield()` state transitions, each gated on a real Groth16 proof check,
- Merkle insertion of note commitments,
- duplicate commitment protection,
- nullifier tracking to prevent double-spend,
- shielded supply accounting per asset,
- admin pause/unpause and a two-step admin transfer.

Key contract interfaces (real, current):

- `initialize(admin, verifier)`
- `shield(from, asset, amount, rho, rcm, commitment, encrypted_note, shield_proof, shield_pub)`
- `transfer(nullifiers, out_commitments, encrypted_notes, proof, pub_inputs)` / `transfer4(...)` (4-in/4-out variant)
- `unshield(nullifier, to, proof, pub_inputs)`
- `merkle_root()`, `merkle_path(leaf_index)`, `leaf_count()`
- `is_spent(nullifier)`, `shielded_supply(asset)`
- `pause()`, `unpause()`, `transfer_admin(new_admin)`, `accept_admin()`

The contract stores a persistent Merkle root and incremental tree state for note commitments.

### 2.2 Verifier registry contract

`contracts/verifier` is a shared Groth16/BN254 verifying-key registry used by `ct20`, `swap`, and `compliance` alike — one contract, one verifying key per circuit, rather than each contract embedding its own copy.

Key methods:

- `initialize(admin)`
- `register_verifying_key(circuit, vk)` — first-time registration only, fails if already set
- `update_verifying_key(circuit, new_vk)` — rotation, admin-gated (called by `governance` under its own timelock, not directly by end users)
- `get_verifying_key(circuit)`
- `verify(circuit, public_inputs, proof) -> bool`

`circuit` is a `CircuitType` enum (`Shield`, `Transfer`, `Unshield`, `NonMembership`, `Transfer4x4`, `SwapFairness`) so one contract can hold a distinct verifying key per proof shape.

### 2.3 Governance contract

`contracts/governance` owns the timelocked verifying-key rotation policy on top of the verifier registry, and its own admin lifecycle. It is set as `contracts/verifier`'s own admin address, so its cross-contract calls into `verifier` satisfy that contract's `admin.require_auth()` without a separate signature.

Key methods:

- `initialize(admin, verifier)`
- `register_vk(circuit, vk)` — initial registration, no timelock (nothing is being replaced yet)
- `queue_vk_update(circuit, new_vk)` / `execute_vk_update(circuit)` / `cancel_vk_update(circuit)` — 7-day-timelocked rotation of an already-registered key
- `transfer_admin(new_admin)` / `accept_admin()` — two-step admin handover

### 2.4 Viewing key registry contract

`contracts/viewing_keys` registers viewing-key commitments only — sanctions/compliance proofs live in a separate contract (§2.5) so an unrelated compliance-record store doesn't share state or lifecycle with the viewing-key registry.

Key methods:

- `register(owner, vk_commitment, birthday)`
- `get_viewing_key_commitment(owner)`

Compliance model details:

- Viewing key registration is opt-in. A note holder chooses when to register a disclosure commitment.
- The contract does not grant spending authority to auditors. It only records commitments needed for authorized transparency.
- Authorized disclosure is enforced by the viewing key holder and the off-chain indexer, not by on-chain spending logic.

### 2.5 Compliance contract

`contracts/compliance` publishes sanctions-list non-membership proofs, verified against `contracts/verifier`'s `NonMembership` circuit before anything is stored (an earlier design stored the proof bytes as an unverified blob — that gap is closed).

Key methods:

- `initialize(verifier)`
- `publish_compliance_proof(owner, proof, pub_inputs: CompliancePublicInputs { sanctions_root, tk_commitment })`
- `get_compliance_proof(owner)`

### 2.6 Shielded swap contract

The shielded swap contract (`contracts/swap`) implements a commit-reveal swap over shielded notes. It reuses `ct20`'s own already-audited shield/unshield paths for the value-moving steps rather than inventing separate custody logic, and has been through a senior-auditor pass plus a full live-Testnet run of its lifecycle (see `docs/POC_IMPLEMENTATION.md`).

It:

- escrows real value via a real `ct20::unshield` cross-call at commit time (which doubles as the note-ownership proof — no separate ownership circuit),
- requires a real, relayer-fronted SEP-41 transfer of the output asset before a claim can be revealed,
- verifies a fairness proof binding the revealed amount back to the original (still-private) intent commitment, pays the relayer, and re-shields the output as a new note via a real, separate `ct20::shield` call,
- really refunds both sides — via `cancel_swap` (never executed) or `reclaim_expired_swap` (executed but never claimed, after a grace window) — rather than only flipping a status flag.

Key methods:

- `initialize(admin, verifier, ct20)`
- `commit_swap(nullifier_in, intent_commitment, asset_in, asset_out, amount_in, anchor, refund_to, ownership_proof, expiry_ledger) -> swap_id`
- `execute_swap(swap_id, amount_out, relayer)`
- `reveal_and_claim(swap_id, out_rho, out_rcm, out_commitment, out_value_commit, encrypted_note, fairness_proof, fairness_pub, shield_proof) -> leaf_index`
- `cancel_swap(swap_id)` / `reclaim_expired_swap(swap_id)`
- `set_relayer(relayer, approved)` — admin-gated, scoped to this contract (not governance)

### 2.7 Off-chain prover and SDK

The SDK is the interface between user wallets and the Soroban contracts.

Primary functions, and their current status:

- note creation, commitment generation, and BN254 ECDH-based encryption — real (`sdk/src/notes`, `sdk/src/crypto`),
- key derivation (spending/nullifier/viewing/transmission keys, diversified addresses) — real (`sdk/src/keys`),
- Groth16 proof generation for shield, transfer, transfer4, unshield, and swap fairness — real, via `snarkjs` against the compiled circuits (`sdk/src/prover`),
- transaction assembly and submission to Soroban RPC for `shield()`/`transfer()`/`unshield()` — real (`sdk/src/wallet/wallet.ts`),
- wallet sync via the indexer — real (`sdk/src/indexer`),
- higher-level swap (`ZKELLASwap`), auditor (`ZKELLAAuditor`), and compliance (`ZKELLACompliance`) wrapper classes — **still stubs**: their methods exist and type-check but return placeholder values rather than calling the real, already-working contracts and provers underneath them.

Primary SDK modules:

- `sdk/src/keys`, `sdk/src/notes`, `sdk/src/crypto`, `sdk/src/prover`
- `sdk/src/wallet` (`wallet.ts` real; `swap.ts`/`auditor.ts` stubs)
- `sdk/src/compliance` (stub)
- `sdk/src/indexer`

### 2.8 Persistent indexer and wallet sync

A dedicated indexer is required because Stellar RPC event retention is short, and because wallets need reliable state recovery for confidential notes.

The indexer:

- consumes Soroban events from a Stellar RPC node,
- stores encrypted note bundles, commitments, nullifiers, Merkle nodes, and roots,
- serves note history and Merkle authentication paths,
- supports wallets in reconstructing shielded state from any point in time.

Indexer trust boundaries:

- The indexer is an availability and recovery layer, not a security authority.
- Wallet clients must independently verify decrypted notes and Merkle paths against on-chain CT-20 roots.
- If the indexer is unavailable, clients can still use on-chain data for critical state checks, but note reconstruction will be degraded.
- Multiple indexers can coexist to reduce single-point-of-failure risk.

This is now a real, running reference implementation (`indexer/`, Node/TypeScript, `node:sqlite` for storage), validated against live Stellar Testnet event data — not a design sketch. Real API endpoints:

- `GET /notes`
- `GET /merkle/path/:leafIndex`
- `GET /merkle/root` (proxied live to `ct20` itself, not duplicated)
- `GET /commitment/:hex`
- `POST /nullifiers/batch`
- `GET /health`

Not yet covered: horizontal scaling, multiple independent operators, and the operational runbook described in `docs/SCF_READINESS.md` — see `indexer/README.md` for the current status in detail.

### 2.9 Soft PoC implementation versus target architecture

The repository intentionally separates implemented material from the full target architecture. Most of the gap this table used to describe has closed:

| Area | Current repository state | Target architecture requirement |
|---|---|---|
| CT-20 shield/transfer/unshield | Real on-chain Groth16 verification for all three flows, validated locally and with real transactions on live Stellar Testnet | Hardened SEP-41 custody checks, full resource profiling at scale, complete test coverage, external security review |
| Verifier / governance | Real, split contracts (§2.2–2.3): shared verifying-key registry plus timelocked rotation | External audit of the timelock and admin-transfer logic |
| Viewing keys / compliance | Real, split contracts (§2.4–2.5): viewing-key commitment registry plus verified sanctions non-membership proofs | Indexer-mediated viewing-key access workflow, richer disclosure tooling |
| Shielded swaps | Real value movement throughout the lifecycle, audited (three fixed issues) and run end-to-end on live Testnet with real circuit proofs (see `docs/POC_IMPLEMENTATION.md`) | Any actual Stellar DEX execution integration — the current relayer model requires the relayer to front liquidity directly, with no on-chain DEX call; wiring a real DEX trade into the flow is still roadmap work |
| SDK | Real key/note/crypto/prover modules and a real base wallet (`shield`/`transfer`/`unshield`); `ZKELLASwap`/`ZKELLAAuditor`/`ZKELLACompliance` wrapper classes are still stubs | Wire the swap/auditor/compliance wrapper classes to the real contracts and provers they're meant to call; stable public API; generated bindings |
| Indexer | Real, running reference implementation, validated on live Testnet | Horizontal scaling, multiple independent operators, operational runbook |

Every existing contract and code module should still be treated as reviewable material, not finished production infrastructure, until it satisfies the target requirement column — in particular, none of it has been through an *external* security review yet.

## 3. Core data model

The ZKELLA protocol is built around four primary data objects.

### 3.1 Shielded note

A shielded note contains:

- `value` — amount in base units,
- `asset_id` — SEP-41 contract address,
- `rho` — note randomness for nullifier derivation,
- `rcm` — commitment randomness.

### 3.2 Note commitment

A note commitment is the on-chain representation of a shielded note:

```
cm = Poseidon2(Poseidon2(value_field, asset_field), Poseidon2(rho, rcm))
```

This commitment is inserted into the CT-20 Merkle tree.

### 3.3 Nullifier

A nullifier prevents note reuse:

```
nf = Poseidon2(nk, rho)
```

`nk` is derived from the user’s spending key and is unique per note.

### 3.4 Merkle tree

The CT-20 contract uses an incremental binary Merkle tree with depth 32.

- leaf node = note commitment,
- empty leaf = `Poseidon2(0, 0)`,
- internal node = `Poseidon2(left, right)`.

The current root is stored in contract instance storage and used as a public anchor for proofs.

### 3.5 Encrypted note bundle

Shielded notes are transferred off-chain as encrypted bundles containing:

- `ephemeral_pk` — ephemeral BN254 public key,
- `ciphertext` — ChaCha20-Poly1305 encryption of the note plaintext.

The plaintext includes `value`, `asset_id`, `rho`, and `rcm`.

## 4. Protocol flows

The architecture supports five primary flows.

### 4.1 Shield flow

```
User Wallet -> SDK -> Soroban RPC -> CT-20 -> Stellar public layer
```

Steps:

1. The wallet builds a shield note with `value`, `asset_id`, `rho`, `rcm`, and computes the note commitment.
2. The SDK encrypts the note bundle for the recipient or viewing key.
3. The SDK generates a Groth16 proof attesting to note correctness, possession of the secret key, and asset conservation.
4. The wallet submits a `shield()` transaction to CT-20 that also includes the SEP-41 asset transfer into the contract.
5. The CT-20 contract verifies the on-chain proof using Protocol 25 BN254 host functions, checks the SEP-41 transfer, inserts the note commitment into the Merkle tree, updates shielded supply, and emits shield event data.

### 4.2 Transfer flow

```
User Wallet -> SDK -> Indexer -> Soroban RPC -> CT-20
```

Steps:

1. The sender wallet requests the input note’s Merkle authentication path from the indexer.
2. The SDK constructs one or more output notes, computes their commitments, and encrypts output bundles.
3. The SDK generates a Groth16 transfer proof over input nullifiers, output commitments, and balance conservation.
4. The wallet submits a `transfer()` transaction to CT-20.
5. CT-20 verifies the proof, marks input nullifiers as spent, inserts output commitments, and emits transfer event data.

### 4.3 Unshield flow

```
User Wallet -> SDK -> Indexer -> Soroban RPC -> CT-20 -> Stellar public layer
```

Steps:

1. The wallet obtains the note’s Merkle path from the indexer.
2. The SDK generates an unshield proof that links the note commitment, nullifier, and public recipient address.
3. The wallet submits an `unshield()` transaction.
4. CT-20 verifies the proof, marks the nullifier as spent, transfers the underlying SEP-41 asset to the recipient, and emits unshield events.

### 4.4 Shielded swap flow

```
User Wallet -> SDK -> Soroban RPC -> CT-20 -> Stellar DEX -> CT-20
```

Steps:

1. The wallet submits a private swap intent to CT-20 using `commit_swap()`, which locks an input note nullifier and records a swap commitment.
2. A relayer observes the intent, executes the corresponding public DEX trade on Stellar, and returns execution details.
3. The wallet or relayer submits `execute_swap()` with the public trade result.
4. The user submits `reveal_and_claim()` with a proof that the public execution matched the committed private swap terms and a shielded output note.
5. CT-20 verifies the proof, mints the output note commitment, and emits swap event data.
6. If the swap expires without execution, the wallet can call `cancel_swap()` to recover the input note.

Relayer risk and verification:

- The relayer is semi-trusted for submitting public DEX execution details but cannot unilaterally finalize the shielded output without a valid user proof.
- The `reveal_and_claim()` proof must verify the reported DEX result against the original private swap intent.
- If the relayer reports incorrect or stale execution data, the claim will fail and the user can cancel after expiry.

### 4.5 Compliance disclosure flow

```
Viewing Key Holder -> Soroban RPC -> Viewing Key Registry -> Indexer -> Auditor
```

Steps:

1. The user registers a viewing key commitment on the viewing key registry contract (`contracts/viewing_keys`).
2. The indexer stores encrypted notes and associates them with the viewing key commitment when permitted.
3. An auditor uses the registered viewing key to request decryption of permitted note history from the indexer.
4. The compliance contract (`contracts/compliance`) can publish verified on-chain sanctions non-membership proofs, while the actual disclosure remains an off-chain consent process.
5. Authorized disclosure is therefore opt-in and based on the holder's viewing key, not on contract-level spending rights.

## 5. System topology

```
           +---------------+      +----------------+
           |  User Wallet  |      |  Wallet / App  |
           |  (SDK client) |      |  / Integrator  |
           +-------+-------+      +-------+--------+
                   |                      |
                   | SDK / proof / tx     | UI / integration
                   |                      |
            +------+------+       +-------+---------+
            |  zkella-sdk  |       |  Stellar RPC    |
            |  (WASM proof |       |  / network     |
            |   + tx build)|       +-------+---------+
            +------+------+               |
                   |                      |
         submit txs |                      | event stream
                   ▼                      ▼
      +--------------------------+  +-----------------------+
      |  Soroban / ZKELLA on-    |  |  zkella-indexer       |
      |  chain contracts         |  |  (persistent note     |
      |  - CT-20, viewing keys,  |  |   storage + API)      |
      |    swap, governance      |  +-----------------------+
      +--------------------------+
                   |
                   | DEX settlement / SEP-41 transfers
                   ▼
      +--------------------------+
      |  Stellar public layer     |
      |  - SEP-41 tokens         |
      |  - DEX / settlement      |
      +--------------------------+
```

## 6. Implementation lifecycle and remaining roadmap

The target architecture should be delivered through a staged lifecycle. The existing repository sits in the first stage.

```
+------------------+     +------------------+     +------------------+
| Soft PoC         | --> | Reviewed testnet  | --> | Production-ready |
| - initial CT-20  |     | - full proofs     |     | - final contracts|
| - SDK scaffolds  |     | - transfer flow   |     | - hardened SDK   |
| - test vectors   |     | - unshield flow   |     | - monitored ops  |
+------------------+     +------------------+     +------------------+
        |                         |                         |
        v                         v                         v
  validate design           improve contracts          deploy after
  assumptions               and integrations           completed review
```

Remaining roadmap requirements:

- review and improve every existing soft PoC contract before treating it as final protocol logic,
- replace placeholder proof paths with real Groth16 verification through BN254 host functions,
- complete private transfer, unshield, viewing-key registry, shielded swap, and indexer implementations,
- profile Soroban resource usage for Poseidon/Merkle operations and optimize storage, budget, and event layout,
- harden SDK cryptography, key agreement, transaction assembly, and error handling,
- expand unit, property, integration, and testnet regression coverage,
- finalize operational controls for verifier-key rotation, pause/unpause, relayer authorization, and deployment monitoring.

## 7. Trust and security model

### 7.1 Proof and verifier lifecycle

- ZK soundness is provided by Groth16 over BN254.
- Soroban contracts verify proofs using Stellar Protocol 25 pairing host functions.
- Each proof circuit is associated with an on-chain verifier key stored in governance-managed contract state.
- Verifier key rotation is controlled by governance with a timelock and audit trail.
- If a verifier key is compromised, the contract can pause new shielding/transfer operations and publish a replacement key before resuming.

### 7.2 Contract state and minimal on-chain exposure

- On-chain state is limited to note commitments, nullifiers, Merkle roots, verifier parameters, proof-status markers, and authorized relayer/viewing-key commitments.
- The CT-20 contract does not store decrypted note contents or recipient privacy secrets.
- Security depends on the correctness of the contract logic and the soundness of the underlying circuits.

### 7.3 Indexer trust model

- The indexer is an availability and recovery layer, not a security authority.
- Wallets and auditor clients must verify decrypted notes and Merkle paths against the on-chain CT-20 Merkle root.
- If the indexer is unavailable or returns stale data, the client can still validate proofs and state using Soroban RPC and on-chain root information.
- Multiple independent indexers are recommended for resilience.

### 7.4 Compliance and disclosure assumptions

- Viewing key registration is opt-in and does not grant spending authority.
- The contract records commitments and compliance proofs, but does not itself release private note plaintexts.
- Disclosure requires off-chain consent and the use of the viewing key together with indexer-held encrypted note bundles.
- Compliance proofs such as sanctions non-membership are intended to be published on-chain without revealing private note values.

### 7.5 Threat assumptions

The architecture assumes:

- the Groth16 circuit setup and verifier keys are generated securely,
- wallet private keys and viewing keys are kept confidential by users,
- relayers are semi-trusted for shielded swap execution and can be audited through proof checks,
- the Stellar public layer remains secure for SEP-41 asset settlement and DEX execution.

Trusted setup assumptions:

- Production deployments assume an audited or MPC-based setup ceremony for the Groth16 circuit parameters.
- If the trusted setup is not secure, proof soundness cannot be guaranteed, so the verifier key lifecycle must be strictly controlled.

## 8. Appendices

### 8.1 Document relationships

- `docs/TECHNICAL_SPEC.md` contains full protocol details and contract interfaces.
- `docs/CIRCUIT_SPEC.md` contains circuit-level design and proof structure.
- `docs/INTEGRATION_GUIDE.md` describes SDK and integrator workflows.
- `docs/POC_IMPLEMENTATION.md` describes the dedicated PoC/current implementation status separately from the full architecture.
