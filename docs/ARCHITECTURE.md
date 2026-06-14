# ZKELLA Architecture

This document describes the full architecture of the ZKELLA protocol. It is intended as the single reference for the complete design of the system, including how on-chain contracts, off-chain proving, indexer infrastructure, client SDKs, and compliance capabilities fit together.

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

### 1.2 Product positioning

ZKELLA is not a private smart contract VM. It is an application-layer confidential finance stack that:

- preserves Stellar's public settlement guarantees,
- wraps existing SEP-41 assets into confidential notes,
- embeds compliance-aware disclosure, and
- keeps private state recoverable through a dedicated indexer.

### 1.3 Stellar integration

ZKELLA is designed as a native Stellar application layer, not a fork or separate network. The integration is explicit and contract-aware:

- `SEP-41 asset contracts` are the public asset source. Shield operations transfer real Stellar asset units from a SEP-41 issuer or user into CT-20 custody, and unshield operations release the same asset back onto the Stellar public ledger.
- `Soroban smart contracts` are the execution layer. CT-20, viewing key registry, shielded swap, and governance contracts all run on Soroban and enforce the confidential-token protocol.
- `Stellar DEX` is the public settlement layer for shielded swaps. ZKELLA submits swap intents and later verifies the corresponding public DEX execution before minting shielded output notes.
- `Soroban RPC` is the client and indexer integration point. Wallets use RPC to submit transactions and query contract state; the indexer consumes RPC event streams to rebuild note history and Merkle paths.
- `Protocol 25 / BN254 host functions` enable on-chain verification of Groth16 proofs and efficient Poseidon/Poseidon2 hashing in Soroban contracts.
- `Protocol 26` improvements support safe wide integer arithmetic, explicit TTL storage rules, and expanded BN254 group operations for future advanced ZK and cryptographic workflows.

A concrete Stellar integration example:

1. A user approves a SEP-41 asset transfer to the CT-20 contract.
2. The wallet builds a `shield()` transaction that includes the SEP-41 transfer, the note commitment, encrypted note bundle, and Groth16 proof.
3. The CT-20 contract verifies the proof, confirms the SEP-41 transfer succeeded, inserts the note commitment into its Merkle tree, and updates shielded supply.
4. Later, `unshield()` proofs release the same underlying SEP-41 asset back to a public Stellar address.

Operational assumption:

- The shield flow assumes the wallet submits the SEP-41 asset transfer together with the `shield()` proof or in an atomically linked contract execution.
- CT-20 must verify that the public token transfer occurred successfully before accepting the note commitment, ensuring the shielded balance is backed by real SEP-41 assets.

The architecture assumes Stellar wallets, assets, and DEX liquidity remain the primary public layer; ZKELLA adds a confidentiality layer on top of that settlement fabric.

## 2. Core components

The target architecture is composed of six primary components:

1. CT-20 confidential token contract
2. Viewing key registry contract
3. Shielded swap contract
4. Governance and verifier management contract
5. zkella-sdk and off-chain prover
6. Persistent indexer and wallet sync service

Each component is described below.

### 2.1 CT-20 confidential token contract

The CT-20 contract is the core shielded ledger on Soroban.

It handles:

- `shield()` deposits from public SEP-41 assets into the shielded pool,
- Merkle insertion of note commitments,
- duplicate commitment protection,
- nullifier tracking to prevent double-spend,
- shielded supply accounting,
- proof verification for shield, transfer, and unshield operations.

Key contract interfaces:

- `initialize(admin, verifying_key)`
- `shield(...)`
- `transfer(...)`
- `unshield(...)`
- `merkle_root()`
- `is_spent(nullifier)`
- `shielded_supply(asset)`

The contract stores a persistent Merkle root and incremental tree state for note commitments.

### 2.2 Viewing key registry contract

The viewing key registry enables controlled disclosure and auditor workflows while preserving the separation between spending authority and audit authority.

It supports:

- on-chain registration of viewing key commitments tied to an owner address,
- binding a viewing key to a specific disclosure identity or counterparty,
- publication of compliance proofs such as sanctions non-membership or authorized provenance,
- retrieval of active compliance proofs for auditors and regulated third parties.

Compliance model details:

- Viewing key registration is opt-in. A note holder chooses when to register a disclosure commitment.
- The contract does not grant spending authority to auditors. It only records commitments and proofs needed for authorized transparency.
- Authorized disclosure is enforced by the viewing key holder and the off-chain indexer, not by on-chain spending logic.
- The contract can store proofs that a given disclosure identity is not on a sanctions list, but audit data access remains a separate off-chain consent process.

Key methods:

- `register(owner, vk_commitment, birthday)`
- `publish_compliance_proof(owner, sanctions_root, proof, pub_inputs)`
- `get_compliance_proof(owner)`

### 2.3 Shielded swap contract

The shielded swap contract connects shielded notes to public Stellar DEX settlement.

It is designed to:

- accept private swap intents from shielded note owners,
- lock input note nullifiers while a relayer executes the public trade,
- verify that the executed trade respects the user’s fairness constraints,
- mint a shielded output note for the swap result,
- allow refunds or cancellations after expiry.

Key methods:

- `commit_swap(...)`
- `execute_swap(...)`
- `reveal_and_claim(...)`
- `cancel_swap(...)`

### 2.4 Governance and verifier management

Governance is responsible for safe verifier key updates and contract lifecycle controls.

Key responsibilities:

- rotate Groth16 verifying keys with a timelock,
- manage relayer authorizations,
- pause/unpause CT-20 operations,
- transfer admin authority safely.

Key methods:

- `update_verifying_key(admin, circuit_id, new_vk)`
- `set_relayer(admin, relayer, approved)`
- `transfer_admin(current_admin, new_admin)`

### 2.5 Off-chain prover and SDK

The SDK is the interface between user wallets and the Soroban contracts.

Primary functions:

- note creation and commitment generation,
- viewing key derivation and transmission key management,
- encrypted note bundle construction,
- Groth16 proof generation in WASM,
- transaction assembly and submission to Soroban,
- wallet sync via the indexer.

Primary SDK modules:

- `sdk/src/keys`
- `sdk/src/notes`
- `sdk/src/prover`
- `sdk/src/wallet`
- `sdk/src/indexer`

### 2.6 Persistent indexer and wallet sync

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

Typical indexer API endpoints:

- `GET /notes`
- `GET /merkle/path/{leaf_index}`
- `GET /merkle/root`
- `POST /nullifiers/batch`
- `GET /health`

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

1. The user registers a viewing key commitment on the viewing key registry contract.
2. The indexer stores encrypted notes and associates them with the viewing key commitment when permitted.
3. An auditor uses the registered viewing key to request decryption of permitted note history from the indexer.
4. The contract can publish on-chain compliance proofs, such as sanctions non-membership, while the actual disclosure remains an off-chain consent process.
5. Authorized disclosure is therefore opt-in and based on the holder’s viewing key, not on contract-level spending rights.


```
User Wallet -> SDK -> Indexer -> CT-20
```

Steps:

1. Fetch Merkle paths from indexer.
2. Construct output notes and encrypted bundles.
3. Generate Groth16 transfer proof.
4. Submit `transfer()` transaction.
5. CT-20 verifies proof, spends nullifiers, and inserts outputs.

### 4.3 Unshield flow

```
User Wallet -> SDK -> Indexer -> CT-20
```

Steps:

1. Fetch Merkle path for the note.
2. Generate Groth16 unshield proof.
3. Submit `unshield()` transaction with recipient.
4. CT-20 verifies proof, spends nullifier, and transfers SEP-41 tokens.

### 4.4 Shielded swap flow

```
User Wallet -> SDK -> CT-20 -> Stellar DEX -> CT-20
```

Steps:

1. Commit private swap intent to CT-20.
2. Relayer executes DEX trade publicly.
3. Relayer reports execution to CT-20.
4. User proves fairness and claims shielded output note.

### 4.5 Compliance disclosure flow

```
Viewing Key Holder -> Indexer -> Auditor
```

Steps:

1. Register a viewing key commitment on-chain.
2. Auditor decrypts allowed notes from indexer data.
3. Contract publishes compliance proof if required.
4. Off-chain disclosure is performed under authorized consent.

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

## 6. Trust and security model

### 6.1 Proof and verifier lifecycle

- ZK soundness is provided by Groth16 over BN254.
- Soroban contracts verify proofs using Stellar Protocol 25 pairing host functions.
- Each proof circuit is associated with an on-chain verifier key stored in governance-managed contract state.
- Verifier key rotation is controlled by governance with a timelock and audit trail.
- If a verifier key is compromised, the contract can pause new shielding/transfer operations and publish a replacement key before resuming.

### 6.2 Contract state and minimal on-chain exposure

- On-chain state is limited to note commitments, nullifiers, Merkle roots, verifier parameters, proof-status markers, and authorized relayer/viewing-key commitments.
- The CT-20 contract does not store decrypted note contents or recipient privacy secrets.
- Security depends on the correctness of the contract logic and the soundness of the underlying circuits.

### 6.3 Indexer trust model

- The indexer is an availability and recovery layer, not a security authority.
- Wallets and auditor clients must verify decrypted notes and Merkle paths against the on-chain CT-20 Merkle root.
- If the indexer is unavailable or returns stale data, the client can still validate proofs and state using Soroban RPC and on-chain root information.
- Multiple independent indexers are recommended for resilience.

### 6.4 Compliance and disclosure assumptions

- Viewing key registration is opt-in and does not grant spending authority.
- The contract records commitments and compliance proofs, but does not itself release private note plaintexts.
- Disclosure requires off-chain consent and the use of the viewing key together with indexer-held encrypted note bundles.
- Compliance proofs such as sanctions non-membership are intended to be published on-chain without revealing private note values.

### 6.5 Threat assumptions

The architecture assumes:

- the Groth16 circuit setup and verifier keys are generated securely,
- wallet private keys and viewing keys are kept confidential by users,
- relayers are semi-trusted for shielded swap execution and can be audited through proof checks,
- the Stellar public layer remains secure for SEP-41 asset settlement and DEX execution.

Trusted setup assumptions:

- Production deployments assume an audited or MPC-based setup ceremony for the Groth16 circuit parameters.
- If the trusted setup is not secure, proof soundness cannot be guaranteed, so the verifier key lifecycle must be strictly controlled.

## 7. Appendices

### 7.1 Document relationships

- `docs/TECHNICAL_SPEC.md` contains full protocol details and contract interfaces.
- `docs/CIRCUIT_SPEC.md` contains circuit-level design and proof structure.
- `docs/INTEGRATION_GUIDE.md` describes SDK and integrator workflows.
- `docs/POC_IMPLEMENTATION.md` describes the dedicated PoC/current implementation status separately from the full architecture.
