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
             |  - ShieldedToken contract           |
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
| - ShieldedToken ledger                  |   | - encrypted note store  |
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
- the shielded swap contract (`contracts/swap`) genuinely moves value — real escrow via a `ShieldedToken::unshield` cross-call, real relayer-fronted liquidity, real payout and re-shield — and has been through a senior-auditor pass (three fixed issues) and a full live-Testnet run of its commit → execute → reveal-and-claim lifecycle with real circuit proofs at every stage,
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
| Smart contracts | Rust, Soroban SDK, Soroban instance/persistent storage | ShieldedToken ledger, nullifier registry, Merkle roots, viewing-key registry, swap controller, verifier governance |
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
  ShieldedToken contract                  Shielded swap contract

Data plane:

  Wallet/SDK
     | 1. build notes + proofs
     v
  Soroban RPC
     | 2. submit shield/transfer/unshield/swap tx
     v
  ShieldedToken / swap contracts
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
| SEP-41 token contracts | Source and sink for public assets entering or leaving ShieldedToken custody |
| Soroban contracts | Execution layer for ShieldedToken, viewing-key registry, shielded swap controller, and verifier governance |
| Soroban host functions | Native BN254 and Poseidon/Poseidon2 operations for proof verification and commitment/Merkle hashing |
| Soroban RPC | Transaction submission, simulation, state reads, event streaming, and wallet/indexer synchronization |
| Stellar DEX | Public liquidity and execution venue for shielded swap settlement |
| Stellar ledger history | Finality anchor, event ordering source, trusted setup beacon source, and recovery checkpoint source |

Several of these touchpoints are already real and exercised on live Stellar Testnet — see §1.4 above for exactly which — but none of it has been through an external security review or a production trusted-setup ceremony, so every integration point below should still be reviewed before being treated as final, production-ready infrastructure.

#### 1.7.1 SEP-41 asset custody model

ShieldedToken is designed to wrap existing Stellar assets without creating a separate asset universe. A shielded note is backed by real SEP-41 token units held or controlled by the ShieldedToken contract.

Planned custody flow:

1. User selects a SEP-41 asset contract, such as XLM's Stellar Asset Contract or a stablecoin asset contract.
2. Wallet/SDK builds a shield note with `asset_id = SEP-41 contract address`.
3. Public SEP-41 units move into ShieldedToken custody during `shield()`.
4. ShieldedToken records only the note commitment, encrypted note payload, asset identifier, and shielded supply accounting.
5. During `unshield()`, ShieldedToken verifies the spend proof, marks the nullifier as spent, and releases SEP-41 units to a public Stellar address.

```
Public Stellar balance
        |
        | SEP-41 transfer / contract invocation
        v
+---------------------+        private note commitment
| ShieldedToken custody       | --------------------------------+
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

- shielded supply for each asset must never exceed the ShieldedToken contract's backing SEP-41 balance,
- each note commitment must bind `asset_id` so notes cannot be replayed across assets,
- unshield must reveal enough public information to release the correct asset and amount while preserving private history,
- token custody checks must be hardened before final contracts are deployed.

#### 1.7.2 Soroban contract integration

The planned on-chain deployment is a set of specialized Soroban contracts:

| Contract | Stellar/Soroban dependency | Responsibility |
|---|---|---|
| ShieldedToken (`contracts/token`) | SEP-41 token interface, Soroban storage, BN254/Poseidon host functions | shield, transfer, unshield, nullifier tracking, Merkle root management, shielded supply accounting |
| Viewing key registry (`contracts/viewing_keys`) | Soroban storage and events | viewing-key commitment registration only |
| Compliance (`contracts/compliance`) | Cross-calls `contracts/verifier` | verified sanctions non-membership proof storage/retrieval |
| Shielded swap (`contracts/swap`) | `ShieldedToken`/`verifier` cross-calls, SEP-41 token transfers, relayer authorization | real escrow via `ShieldedToken::unshield`, relayer-fronted liquidity, fairness-proof-gated payout and re-shield, cancellation/reclaim paths |
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

- consume ShieldedToken and registry events from a configured start ledger,
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

**This subsection describes a target design that differs from what's actually implemented and already audited/live on Testnet.** The real `contracts/swap` (see §2.6) doesn't call the Stellar DEX at all, and `commit_swap`'s real signature is `commit_swap(nullifier_in, intent_commitment, asset_in, asset_out, amount_in, anchor, refund_to, ownership_proof, expiry_ledger)`, not the simplified `commit_swap(intent_commitment, nullifier, expiry)` shown below. What's real instead: the relayer directly fronts the output asset as SEP-41 liquidity in `execute_swap`, and the contract's own escrow/payout logic (reusing `ShieldedToken`'s shield/unshield paths) does the rest — see `docs/TECHNICAL_SPEC.md` §9 for the exact real flow and why it differs from the DEX-execution model below. Wiring an actual on-chain Stellar DEX call into the flow remains open, roadmap work.

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

- deploy PoC and reviewed testnet ShieldedToken contracts to Stellar Testnet,
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

1. ShieldedToken contract
2. Verifier registry contract (Groth16 verifying-key storage and the shared `verify()` entrypoint)
3. Governance contract (timelocked verifying-key rotation, relayer/admin controls)
4. Viewing key registry contract
5. Compliance contract (sanctions non-membership proofs, verified against the verifier registry)
6. Shielded swap contract
7. zkella-sdk and off-chain prover
8. Persistent indexer and wallet sync service

The verifier and governance contracts were originally described as one combined "governance/verifier management" component; the repository implementation splits them so the verifying-key registry (`contracts/verifier`) can be reused by every other contract (`ShieldedToken`, `swap`, `compliance`) as a plain cross-contract call, while `contracts/governance` owns only the timelocked rotation policy on top of it. Likewise, viewing-key registration and sanctions-proof publication were originally one component; the repository splits them into `contracts/viewing_keys` and `contracts/compliance` so an unrelated compliance-record store doesn't share a contract with the viewing-key registry.

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
| Wallet / SDK | -----------------> | ShieldedToken ledger | --------------> | Indexer       |
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

Shielded swaps extend the ShieldedToken ledger through a swap controller that locks input
nullifiers, escrows real value via ShieldedToken's own unshield/shield paths (not a Stellar
DEX call — see §1.7.5's caveat), and mints verified shielded outputs.
```

### 2.1 ShieldedToken contract

The ShieldedToken contract (`contracts/token`) is the core shielded ledger on Soroban. Shield, transfer (2-in/2-out and 4-in/4-out), and unshield are all implemented with real on-chain Groth16 verification, cross-calling `contracts/verifier`.

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

`contracts/verifier` is a shared Groth16/BN254 verifying-key registry used by `ShieldedToken`, `swap`, and `compliance` alike — one contract, one verifying key per circuit, rather than each contract embedding its own copy.

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

The shielded swap contract (`contracts/swap`) implements a commit-reveal swap over shielded notes. It reuses `ShieldedToken`'s own already-audited shield/unshield paths for the value-moving steps rather than inventing separate custody logic, and has been through a senior-auditor pass plus a full live-Testnet run of its lifecycle (see `docs/POC_IMPLEMENTATION.md`).

It:

- escrows real value via a real `ShieldedToken::unshield` cross-call at commit time (which doubles as the note-ownership proof — no separate ownership circuit),
- requires a real, relayer-fronted SEP-41 transfer of the output asset before a claim can be revealed,
- verifies a fairness proof binding the revealed amount back to the original (still-private) intent commitment, pays the relayer, and re-shields the output as a new note via a real, separate `ShieldedToken::shield` call,
- really refunds both sides — via `cancel_swap` (never executed) or `reclaim_expired_swap` (executed but never claimed, after a grace window) — rather than only flipping a status flag.

Key methods:

- `initialize(admin, verifier, token)`
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
- Wallet clients must independently verify decrypted notes and Merkle paths against on-chain ShieldedToken roots.
- If the indexer is unavailable, clients can still use on-chain data for critical state checks, but note reconstruction will be degraded.
- Multiple indexers can coexist to reduce single-point-of-failure risk.

This is now a real, running reference implementation (`indexer/`, Node/TypeScript, `node:sqlite` for storage), validated against live Stellar Testnet event data — not a design sketch. Real API endpoints:

- `GET /notes`
- `GET /merkle/path/:leafIndex`
- `GET /merkle/root` (proxied live to `ShieldedToken` itself, not duplicated)
- `GET /commitment/:hex`
- `POST /nullifiers/batch`
- `GET /health`

Not yet covered: horizontal scaling, multiple independent operators, and an operational runbook — see `indexer/README.md` for the current status in detail.

### 2.9 Soft PoC implementation versus target architecture

The repository intentionally separates implemented material from the full target architecture. Most of the gap this table used to describe has closed:

| Area | Current repository state | Target architecture requirement |
|---|---|---|
| ShieldedToken shield/transfer/unshield | Real on-chain Groth16 verification for all three flows, validated locally and with real transactions on live Stellar Testnet | Hardened SEP-41 custody checks, full resource profiling at scale, complete test coverage, external security review |
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

This commitment is inserted into the ShieldedToken Merkle tree.

### 3.3 Nullifier

A nullifier prevents note reuse:

```
nf = Poseidon2(nk, rho)
```

`nk` is derived from the user’s spending key and is unique per note.

### 3.4 Merkle tree

The ShieldedToken contract uses an incremental binary Merkle tree with depth 32.

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
User Wallet -> SDK -> Soroban RPC -> ShieldedToken -> Stellar public layer
```

Steps:

1. The wallet builds a shield note with `value`, `asset_id`, `rho`, `rcm`, and computes the note commitment.
2. The SDK encrypts the note bundle for the recipient or viewing key.
3. The SDK generates a Groth16 proof attesting to note correctness, possession of the secret key, and asset conservation.
4. The wallet submits a `shield()` transaction to ShieldedToken that also includes the SEP-41 asset transfer into the contract.
5. The ShieldedToken contract verifies the on-chain proof using Protocol 25 BN254 host functions, checks the SEP-41 transfer, inserts the note commitment into the Merkle tree, updates shielded supply, and emits shield event data.

### 4.2 Transfer flow

```
User Wallet -> SDK -> Indexer -> Soroban RPC -> ShieldedToken
```

Steps:

1. The sender wallet requests the input note’s Merkle authentication path from the indexer.
2. The SDK constructs one or more output notes, computes their commitments, and encrypts output bundles.
3. The SDK generates a Groth16 transfer proof over input nullifiers, output commitments, and balance conservation.
4. The wallet submits a `transfer()` transaction to ShieldedToken.
5. ShieldedToken verifies the proof, marks input nullifiers as spent, inserts output commitments, and emits transfer event data.

### 4.3 Unshield flow

```
User Wallet -> SDK -> Indexer -> Soroban RPC -> ShieldedToken -> Stellar public layer
```

Steps:

1. The wallet obtains the note’s Merkle path from the indexer.
2. The SDK generates an unshield proof that links the note commitment, nullifier, and public recipient address.
3. The wallet submits an `unshield()` transaction.
4. ShieldedToken verifies the proof, marks the nullifier as spent, transfers the underlying SEP-41 asset to the recipient, and emits unshield events.

### 4.4 Shielded swap flow

**This subsection is the target design; see §1.7.5's caveat above — the real, implemented, audited flow (`contracts/swap`) doesn't call the Stellar DEX and has a different call sequence. The real steps are listed after this target version; see `docs/TECHNICAL_SPEC.md` §9.3 and `docs/TESTNET_DEPLOYMENT.md` for the exact signatures and live-Testnet transaction hashes.**

```
User Wallet -> SDK -> Soroban RPC -> ShieldedToken -> Stellar DEX -> ShieldedToken
```

Target-design steps:

1. The wallet submits a private swap intent to ShieldedToken using `commit_swap()`, which locks an input note nullifier and records a swap commitment.
2. A relayer observes the intent, executes the corresponding public DEX trade on Stellar, and returns execution details.
3. The wallet or relayer submits `execute_swap()` with the public trade result.
4. The user submits `reveal_and_claim()` with a proof that the public execution matched the committed private swap terms and a shielded output note.
5. ShieldedToken verifies the proof, mints the output note commitment, and emits swap event data.
6. If the swap expires without execution, the wallet can call `cancel_swap()` to recover the input note.

Real, implemented steps (no DEX call anywhere in the contract):

1. `commit_swap` escrows `amount_in` of `asset_in` immediately via a real `ShieldedToken::unshield` cross-call — the same real Groth16 ownership proof both proves the committer owns the note and atomically pulls its value into escrow.
2. A relayer calls `execute_swap`, really transferring `amount_out` of `asset_out` into escrow (a real SEP-41 transfer, not a DEX trade) — how the relayer sources that liquidity, including whether they route through the DEX themselves, is entirely off-chain and outside the contract's concern.
3. The claimant calls `reveal_and_claim` with a real swap-fairness proof (binding the now-revealed `amount_out`/`min_amount_out` back to the still-private `intent_commitment`) and a second, separate real shield proof for the output note; the contract pays the relayer the escrowed `asset_in` and re-shields `asset_out` as a new note via a real `ShieldedToken::shield` call.
4. `cancel_swap` (never executed) or `reclaim_expired_swap` (executed but never claimed) really refund both sides if the happy path doesn't complete.

Relayer risk and verification (target-design framing above; the real mechanism is simpler — there's no "reported DEX result" to verify, since the relayer never reports execution data on-chain at all):

- The relayer is semi-trusted for fronting the output asset but cannot unilaterally finalize the shielded output without a valid user fairness proof.
- The real `reveal_and_claim()` proof verifies the revealed `amount_out`/`min_amount_out` against the original, still-private `intent_commitment` — not against any DEX execution report, since none exists.
- If the relayer never fronts the output asset, `execute_swap` is simply never called and the user can `cancel_swap` after expiry; if the relayer fronts it but the user never claims, `reclaim_expired_swap` returns both sides' funds after a grace window.

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
      |  - ShieldedToken, viewing keys,  |  |   storage + API)      |
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

The target architecture is delivered through a staged lifecycle. The repository has moved past the first stage — see §1.4 and §2.9 above for exactly what's real — and sits between the first two:

```
+------------------+     +------------------+     +------------------+
| Soft PoC         | --> | Reviewed testnet  | --> | Production-ready |
| (fully passed)   |     | (repo is here)    |     | - final contracts|
+------------------+     +------------------+     +------------------+
                                   |                         |
                                   v                         v
                          real proofs, real value      deploy after
                          movement, live-Testnet        completed review
                          evidence — not yet an          + real ceremony
                          external review
```

What moved the repository past "Soft PoC": real on-chain Groth16 verification for shield/transfer/unshield (not placeholders), a real, audited, live-Testnet-run shielded swap lifecycle, a real running indexer, and a real SDK cryptographic/proving core — all with live Stellar Testnet transaction evidence (`docs/POC_IMPLEMENTATION.md`, `docs/TESTNET_DEPLOYMENT.md`).

What's still needed to reach "Production-ready":

- an *external*, independent security review of every contract and circuit — everything to date, including the audit pass described in §1.4, was done by the team building the protocol, not a third party,
- a real (non-dev), multi-party trusted-setup ceremony per circuit — every proof and verifying key in this repository so far comes from a local, single-contributor dev ceremony,
- wiring the SDK's higher-level `ZKELLASwap`/`ZKELLAAuditor`/`ZKELLACompliance` wrapper classes to the real contracts and provers underneath them (still stubs — see §2.7),
- indexer production hardening: horizontal scaling, multiple independent operators, and an operational runbook (still open — see §2.8),
- resource profiling at production scale (current measurements are from a single local host environment plus a handful of live-Testnet transactions, not sustained load),
- finalized operational controls for verifier-key rotation, pause/unpause, relayer authorization, and deployment monitoring beyond what's already implemented.

## 7. Trust and security model

### 7.1 Proof and verifier lifecycle

- ZK soundness is provided by Groth16 over BN254.
- Soroban contracts verify proofs using Stellar Protocol 25 pairing host functions.
- Each proof circuit is associated with an on-chain verifier key stored in governance-managed contract state.
- Verifier key rotation is controlled by governance with a timelock and audit trail.
- If a verifier key is compromised, the contract can pause new shielding/transfer operations and publish a replacement key before resuming.

### 7.2 Contract state and minimal on-chain exposure

- On-chain state is limited to note commitments, nullifiers, Merkle roots, verifier parameters, proof-status markers, and authorized relayer/viewing-key commitments.
- The ShieldedToken contract does not store decrypted note contents or recipient privacy secrets.
- Security depends on the correctness of the contract logic and the soundness of the underlying circuits.

### 7.3 Indexer trust model

- The indexer is an availability and recovery layer, not a security authority.
- Wallets and auditor clients must verify decrypted notes and Merkle paths against the on-chain ShieldedToken Merkle root.
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
- `docs/TESTNET_DEPLOYMENT.md` is the current live-Testnet address and transaction record.
