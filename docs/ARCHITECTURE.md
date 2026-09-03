# ZKELLA Architecture

This document describes the full architecture of the ZKELLA protocol. It is intended as the single reference for the complete design of the system, including how on-chain contracts, off-chain proving, indexer infrastructure, client SDKs, and compliance capabilities fit together.

The repository currently contains a PoC implementation only. Existing contracts and SDK code validate early design assumptions, but they are not final versions of the protocol contracts and must be reviewed, optimized, hardened, and improved before any production deployment.

## Summary

ZKELLA implements a confidential-finance protocol on Stellar: shielded notes, private transfers, a shielded swap primitive, and compliance-aware disclosure, all backed by on-chain Groth16 verification. The core flows, shield, transfer, unshield, and swap, are real and working, and each has been exercised in an independently-verifiable transaction on live Stellar Testnet, not just locally. §6.1 gives a full, component-by-component account of what is built and proven today versus what remains, with the transaction evidence behind each claim.

The most important open item right now: the 4-in/4-out transfer entrypoint's real, compiled-WASM instruction cost sits at essentially the mainnet instruction limit rather than comfortably under it, and closing that margin, via a concrete, already-identified verifier optimization (see §2.2 and §6.1), is a prerequisite for that path to ship on mainnet, not an optional improvement. See `docs/SCF_READINESS.md` for the full measurement history, including its sensitivity to the exact Rust compiler version used.

None of this has been through an independent, external security review yet, and no production, multi-party trusted-setup ceremony has been run; both remain required before any mainnet deployment. See §1.4 and §6 for the complete maturity boundary and remaining roadmap.

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

#### 1.7.5b Alternative target design: AMM-sourced execution as an explicit fallback path

**Status: design reviewed and Router-vs-Aggregator decided (Router-only for v1) — implementation deliberately deferred, not started.** This is a documented, reviewed design ready to pick up later, not a rejected one. Do not implement from this section alone; the remaining open questions at the end must still be resolved first.

A separate question from §1.7.5's classic-DEX target design: could `execute_swap` cross-call a Soroban-native AMM contract (as opposed to Stellar's classic ledger-level DEX/liquidity-pool operations, which remain unreachable — `docs/TECHNICAL_SPEC.md` §9)? A Soroban AMM is a normal contract with a public `swap()`-style entrypoint, reachable via the same cross-contract-call mechanism `swap` already uses for `ShieldedToken`. This is technically straightforward. The design work is in *how* to do it without silently degrading the primitive's core privacy property, and in surveying what actually exists on Stellar today rather than assuming a single generic "the AMM."

##### What actually exists on Stellar today

There is no single canonical Soroban AMM — several independent, differently-interfaced protocols operate concurrently: **Soroswap** (a Uniswap-v2-style AMM with its own `SoroswapRouter` contract), **Phoenix**, and **Aqua** (Aquarius). Building and maintaining a direct adapter to each independently would be real, ongoing integration debt with no natural end (new AMMs launch, existing ones change interfaces).

Soroswap already solves this exact problem at the ecosystem level: the **Soroswap Aggregator** is a separate, real on-chain contract that routes across Soroswap, Phoenix, and Aqua through a `SoroswapAggregatorAdapterTrait` adapter pattern — a per-protocol adapter contract behind one stable caller-facing interface. `SoroswapRouter`'s own interface (confirmed from Soroswap's technical docs), which the Aggregator's caller-facing shape follows closely:

```rust
fn swap_exact_tokens_for_tokens(
    e: Env,
    amount_in: i128,
    amount_out_min: i128,
    path: Vec<Address>,
    to: Address,
    deadline: u64,
) -> Result<Vec<i128>, CombinedRouterError>
```

**Decided (this session): target `SoroswapRouter` directly for v1, not the Aggregator.** Weighed against integrating the Aggregator immediately for multi-AMM (Soroswap + Phoenix + Aqua) reach — the smaller, simpler dependency wins for a first iteration: one external contract to audit and reason about (`SoroswapRouter`) instead of two layers (the Aggregator plus its per-protocol adapters). Multi-AMM reach via the Aggregator remains a deliberate, explicitly-deferred future widening, not a rejected idea — revisit once the Router-only path has real operational experience behind it (see `docs/DESIGN_EXPLORATION.md` §2.3 for the tracked status).

Both options carry the same category of dependency, sized differently for the Router:

- **Trust dependency.** Any bug, pause, or exploit in `SoroswapRouter` becomes ZKELLA's problem the moment escrowed user funds route through it. Its own audit status must be checked before this is implementable, not assumed.
- **Interface stability dependency.** If the Router's interface changes, `contracts/swap` needs a corresponding update — an external dependency our own release cadence doesn't control.
- **Fee/pricing transparency, not a new risk.** The Router's own protocol fees are already reflected in whatever `amount_out` it returns — this doesn't change the slippage-floor check `contracts/swap` already needs to do (§ below), it's just baked into the number.
- **Exact deployed contract addresses (testnet/mainnet) are not yet confirmed** — this doc deliberately does not state one, to avoid a stale/wrong address propagating the way earlier deployment records in this repo have had to be corrected before. Sourcing and verifying the real address is a precondition for implementation, not an assumption to make now.

##### The hybrid model: RFQ-first, AMM-sourced execution as an explicit, non-default fallback

The recommended shape is **not** "replace the relayer with the AMM" — it's "use the AMM only when the private RFQ path (`sdk/src/relayer/quote.ts`, §2.7) doesn't produce a competitive result." The RFQ path stays the privacy-preserving default; AMM-sourced execution is a separate, clearly-labeled path a wallet opts into, not a silent substitution. This preserves the primitive's core value (private-by-default) while giving users a way to still get a swap done when no relayer is available — better usability without lowering the default privacy bar.

##### Proposed contract surface (design only — not implemented)

A **separate** entrypoint from `execute_swap`, not a modification of it — keeping the two execution paths structurally distinct makes the privacy trade-off visible in the contract's own interface, not just in documentation:

```rust
fn execute_swap_via_amm(
    env:            Env,
    swap_id:        BytesN<32>,
    router:         Address,      // must be on an approved-router allowlist — see security review below
    min_amount_out: i128,         // caller's own floor; independent of, and in addition to, the user's protocol-level floor checked later at reveal_and_claim
    deadline:       u64,
) -> Result<i128, Error>;

fn set_approved_router(env: Env, router: Address, approved: bool) -> Result<(), Error>; // admin-gated, mirrors set_relayer
```

##### Security review of this proposal (senior-auditor pass, before any code is written)

1. **Router must be allowlisted, not caller-supplied freely.** Without this, any caller could pass the address of a contract they control that accepts the escrowed `asset_in` and returns nothing — a direct theft path. `set_approved_router`, admin-gated, mirrors the existing `set_relayer` pattern exactly. This is not optional hardening, it's the difference between a real feature and a fund-drain vector.
2. **Restrict `path` to a direct pair for v1 — no multi-hop.** A caller-supplied multi-hop `path` widens the attack surface (routing through a low-liquidity or manipulated intermediate pool) for no benefit the Aggregator itself doesn't already provide internally. Recommend constraining to `path == [asset_in, asset_out]` (validated on-chain, not just assumed) until the direct-pair path has real operational experience.
3. **This should not be fully permissionless — require `refund_to.require_auth()`.** An earlier sketch of this idea (this session's prior conversation turn) assumed permissionless execution "anyone can trigger it." On review, that's worse than necessary: it lets a third party trigger execution against parameters they choose (a `min_amount_out` picked to make sandwiching worthwhile), with no economic stake in the outcome. Requiring the original committer's `refund_to` address to authorize this specific call keeps the "no dependency on a specific relayer being available" benefit (anyone can still *pay the transaction fee*, i.e. relay the call) while removing the "anyone can pick unfavorable execution parameters" risk. This is a meaningful improvement over the earlier framing, not a restatement of it.
4. **`SwapState`/`reclaim_expired_swap` need a real design decision, not an assumption.** The current state machine's unclaimed-execution unwind path (`reclaim_expired_swap`) assumes `state.relayer` is always `Some` once `status == Executed`, and refunds the fronted `asset_out` to that relayer. An AMM-sourced execution has no relayer to refund — the `asset_out` came from the AMM, paid for (indirectly) with the user's own escrowed `asset_in`. If never claimed, that `asset_out` should return to `refund_to` (the original committer), not a relayer. This means `SwapState` needs an explicit `ExecutionKind::RelayerFronted | AmmSourced` (not an overloaded `Option<Address>`), and `reclaim_expired_swap` needs a real branch for each — this is a genuine data-model change to `contracts/swap`, not a cosmetic addition, and must be designed carefully enough to keep both paths' existing regression tests passing.

##### What this does not solve — unchanged from the earlier analysis

Everything §1.7.5b previously documented about `amount_out` losing its "hidden until reveal" property, and about MEV exposure from a public, snipeable trade, still applies exactly the same way when routing through `SoroswapRouter` — using the Router instead of a raw pool changes *pricing quality*, not the privacy/MEV trade-off itself. This remains explicitly a "less private, more liquid, opt-in fallback" path, not a resolution of the underlying tension.

##### Open questions still to resolve — implementation deliberately deferred

Router-vs-Aggregator is decided (Router-only for v1); these remain open before any code is written:

- Confirm `SoroswapRouter`'s actual audit status and real deployed contract address per network.
- Decide the exact `ExecutionKind` state-machine change and re-verify every existing `contracts/swap` regression test against it.
- Decide the SDK-side UX for surfacing "RFQ found nothing competitive, offer the AMM-fallback path" to a wallet/user in a way that makes the privacy trade-off legible at the moment of choice, not buried in documentation only.
- Multi-AMM reach via the Aggregator (Phoenix, Aqua) remains a deliberately deferred future widening once the Router-only path has real operational experience — not scoped for the same implementation pass as the items above.

Sources for the landscape research above: [SoroswapRouter technical reference](https://docs.soroswap.finance/01-protocol-overview/03-technical-reference/03-smart-contracts/04-soroswaprouter), [Soroswap Aggregator overview](https://docs.soroswap.finance/01-concepts/aggregator), [soroswap/aggregator repository](https://github.com/soroswap/aggregator/) (confirms Phoenix and Aqua adapters alongside Soroswap's own).

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
- `unshield(nullifier, to, binding_tag, proof, pub_inputs)` — `binding_tag` folds into `recipient_hash` alongside `to`; a direct unshield passes `[0u8; 32]`. See `docs/TECHNICAL_SPEC.md`'s interface listing for why this parameter exists — added to close a real proof-replay finding in `contracts/swap` (`docs/POC_IMPLEMENTATION.md`'s "Update: external audit")
- `merkle_root()`, `merkle_path(leaf_index)`, `leaf_count()`
- `is_spent(nullifier)`, `shielded_supply(asset)`
- `pause()`, `unpause()`, `transfer_admin(new_admin)`, `accept_admin()`

The contract stores a persistent Merkle root and incremental tree state for note commitments.

**Why transfer's input/output count is fixed, not variable.** `transfer()` takes exactly 2 input notes and produces exactly 2 outputs (`circuits/transfer_2in2out/transfer.circom` hard-codes `N_IN = N_OUT = 2`); `transfer4()` is a separate, similarly fixed 4-in/4-out circuit. This is a deliberate shielded-pool design pattern (the same one Zcash Sapling uses), not an arbitrary limitation of the current PoC, for two reasons:

- **Uniform transaction shape.** Every `transfer()` call looks identical on-chain regardless of how many notes the sender actually holds or is consolidating. A variable-arity circuit would make transaction structure itself leak information — an observer could infer how fragmented a user's holdings are just from the shape of their calls, which a note-based privacy design is specifically trying to avoid.
- **Circuit cost.** A variable number of inputs requires padding/dummy notes and conditional in-circuit logic for "was this slot used," which adds constraints and complexity beyond what a fixed shape needs.

The two fixed shapes are the answer to "what if I have more than 2 notes to spend or more than 2 recipients": `transfer4()` exists specifically for **dust consolidation and multi-recipient payments** (see `docs/CIRCUIT_SPEC.md` §5), not as an arbitrary second option. A sender with more inputs/outputs than either shape supports uses multiple transfers, at the cost of revealing more transaction-graph structure across those calls — an accepted tradeoff of this design, not an oversight.

On the live Testnet deployment described in `docs/TESTNET_DEPLOYMENT.md`, all five verifying keys — Shield, Unshield, SwapFairness, Transfer, and Transfer4x4 — are now registered. `Transfer` (2-in/2-out) has been run through a real, live on-chain `transfer()` call (see `docs/TESTNET_DEPLOYMENT.md`'s "Update: Transfer VK registration and a real, live transfer() transaction"); `Transfer4x4`'s key is live-registered but a live 4-in/4-out transaction has not yet been run.

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
- `queue_vk_update(circuit, new_vk)` / `execute_vk_update(circuit)` / `cancel_vk_update(circuit)` — 7-day-timelocked, for *both* first-time registration and replacement of an already-registered key. There used to be a separate `register_vk` offering instant, untimelocked first-time activation; removed after an external review found it let a malicious VK for *any* circuit — even a brand-new one — immediately forge proofs against value already resting in `ShieldedToken`'s shared pool via an already-legitimate circuit. `execute_vk_update` now picks `register_verifying_key` or `update_verifying_key` on the verifier depending on whether the circuit already has a key (`docs/POC_IMPLEMENTATION.md`'s "Update: external audit").
- `transfer_admin(new_admin)` / `accept_admin()` — two-step admin handover

**Guardian role (planned, not yet implemented).** `cancel_vk_update` currently requires the same single admin key as `queue_vk_update`. If that key is compromised, the party that queues a malicious verifying-key rotation is the only party able to cancel it, so the seven-day delay gives outside observers advance warning but gives the team no actual on-chain recourse. The planned fix adds a distinct `guardian` address, separate from `admin`, with authority to call `cancel_vk_update` on its own, a small, contained change to this contract's storage and auth checks, not a new contract or architecture. Moving `admin` itself to a classic Stellar multisig account is a deployment-time decision available today with no code change at all, since `require_auth()` on a classic multisig account already enforces that account's own signer/threshold policy; it is deferred to the mainnet deployment rather than built here, since it changes nothing about this contract's code. A fuller custom smart-account with on-chain proposal and voting logic (the pattern OpenZeppelin's own Stellar contracts package implements) was considered and deliberately deferred as disproportionate for a small team at Testnet stage, worth reconsidering only if operational needs grow after mainnet.

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

**Design note — extension hook points (open, not yet implemented; see `docs/DESIGN_EXPLORATION.md` §1.3).** `contracts/token`'s current compliance integration is, at most, a single optional hook address, no-op if unset — deliberately not a generic extension-point trait, judged disproportionate to today's needs (compare against OpenZeppelin's confidential-token `Hooks` trait, which exposes eight lifecycle points). If ZKELLA's compliance needs grow beyond what a single hook can express, the proposed middle ground is three targeted hooks rather than one or eight:

- `on_shield(from, asset, amount)` — called after auth/validation, before the Groth16 proof check, so a compliance policy can block a deposit from an unapproved source before it ever reaches proof verification,
- `on_transfer(nullifiers, out_commitments, asset_id)` — called after arity/anchor checks, before proof verification, mirroring `on_shield`'s placement,
- `on_unshield(nullifier, to, asset, amount)` — called after the recipient-hash binding check, before proof verification.

Each hook would receive only decoded, already-validated call parameters — never the proof itself, matching OpenZeppelin's own rationale for that boundary (a hook that could see the proof could be tempted to make decisions based on unverified data). A no-op default (no hook configured) would preserve today's behavior exactly. This is a proposal to evaluate, not a committed design — it has not been reviewed against `contracts/token`'s actual call-site structure for feasibility, and doing so is the real next step before writing any code.

### 2.6 Shielded swap contract

The shielded swap contract (`contracts/swap`) implements a commit-reveal swap over shielded notes. It reuses `ShieldedToken`'s own already-audited shield/unshield paths for the value-moving steps rather than inventing separate custody logic, and has been through both a senior-auditor pass and a separate external technical review — the latter finding and fixing three real critical vulnerabilities (a missing `intent_commitment` binding at claim time, a proof-replay path via the reused ownership proof, and a missing re-initialization guard) plus a fund-lock overflow bug, all closed with dedicated regression tests — alongside a full live-Testnet run of its lifecycle (see `docs/POC_IMPLEMENTATION.md`'s "Update: external audit").

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
- off-chain relayer price discovery for shielded swaps (RFQ) — real (`sdk/src/relayer/quote.ts`): `SwapQuoteClient.requestQuote()` and the `RelayerQuoteHandler` type specify the wire protocol closing `docs/TECHNICAL_SPEC.md` §12.4's Known Limitation 1; `quoteRespectsSlippage()` enforces the same floor `circuits/swap/swap_fairness.circom` checks on-chain, so a wallet acting only on quotes this module validates cannot be misled into a swap that would fail its own fairness proof later — see the module's own doc comment for the full security model, including what it deliberately does not guarantee (no execution reservation for the quoting relayer),
- higher-level swap (`ZKELLASwap`), auditor (`ZKELLAAuditor`), and compliance (`ZKELLACompliance`) wrapper classes — **still stubs**: their methods exist and type-check but return placeholder values rather than calling the real, already-working contracts and provers underneath them.

Primary SDK modules:

- `sdk/src/keys`, `sdk/src/notes`, `sdk/src/crypto`, `sdk/src/prover`
- `sdk/src/wallet` (`wallet.ts` real; `swap.ts`/`auditor.ts` stubs)
- `sdk/src/relayer` (`quote.ts` real — RFQ client/handler shape, not a full relayer service)
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
- **Indexer operator and wallet are distinct roles, not yet formally separated in ZKELLA's own docs.** OpenZeppelin's confidential-token indexer spec makes this distinction explicit — an indexer operator archiving event history is a different role, with potentially different compliance obligations, from a wallet operator who only reads that history for its own users. ZKELLA's indexer today is self-hosted per deployment (`indexer/README.md`), so the distinction hasn't mattered operationally yet, but it's worth stating explicitly once third-party or multi-operator indexing (already on the roadmap — see `docs/DESIGN_EXPLORATION.md` §1.4) becomes real: an indexer operator who is not the wallet provider is a new trust/compliance surface that hasn't been analyzed here.

This is now a real, running reference implementation (`indexer/`, Node/TypeScript, `node:sqlite` for storage), validated against live Stellar Testnet event data — not a design sketch. Real API endpoints:

- `GET /notes`
- `GET /merkle/path/:leafIndex`
- `GET /merkle/root` (proxied live to `ShieldedToken` itself, not duplicated)
- `GET /commitment/:hex`
- `POST /nullifiers/batch`
- `GET /health`

Not yet covered: horizontal scaling and multiple independent operators — see `indexer/README.md` for the current status in detail, and `docs/TECHNICAL_SPEC.md` §13.3 for the planned production deployment target (dual-provider RPC failover, AWS ECS Fargate, RDS PostgreSQL Multi-AZ, and a second operator in a different region or cloud provider). An operational runbook now exists (`docs/RUNBOOK.md`, first version, not yet exercised in a real incident) and covers the indexer alongside every other component.

### 2.9 PoC implementation versus target architecture

The repository intentionally separates implemented material from the full target architecture. Most of the gap this table used to describe has closed:

| Area | Current repository state | Target architecture requirement |
|---|---|---|
| ShieldedToken shield/transfer/unshield | Real on-chain Groth16 verification for all three flows, validated locally and with real transactions on live Stellar Testnet | Hardened SEP-41 custody checks, full resource profiling at scale, complete test coverage, external security review |
| Verifier / governance | Real, split contracts (§2.2–2.3): shared verifying-key registry plus timelocked rotation | External audit of the timelock and admin-transfer logic |
| Viewing keys / compliance | Real, split contracts (§2.4–2.5): viewing-key commitment registry plus verified sanctions non-membership proofs | Indexer-mediated viewing-key access workflow, richer disclosure tooling |
| Shielded swaps | Real value movement throughout the lifecycle, audited (three fixed issues) and run end-to-end on live Testnet with real circuit proofs (see `docs/POC_IMPLEMENTATION.md`) | Any actual Stellar DEX execution integration — the current relayer model requires the relayer to front liquidity directly, with no on-chain DEX call; wiring a real DEX trade into the flow is still roadmap work |
| SDK | Real key/note/crypto/prover modules and a real base wallet (`shield`/`transfer`/`unshield`); `ZKELLASwap`/`ZKELLAAuditor`/`ZKELLACompliance` wrapper classes are still stubs | Wire the swap/auditor/compliance wrapper classes to the real contracts and provers they're meant to call; stable public API; generated bindings |
| Indexer | Real, running reference implementation, validated on live Testnet; a first-version operational runbook exists (`docs/RUNBOOK.md`) | Horizontal scaling, multiple independent operators, a runbook proven through real incident use rather than only written |

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

The current root is stored in contract instance storage and used as a public anchor for proofs. A proof's anchor doesn't have to match the current root exactly: `merkle::is_known_root` also accepts any of the last `ROOT_HISTORY_SIZE` (32) roots the tree actually had, tracked as a ring buffer alongside the current root. This matters because one `ShieldedToken` instance shares this single tree across every asset it wraps — without a history window, a proof anchored to root R would be invalidated by *any* other shield/transfer/unshield call landing first, even on a completely different asset. The window doesn't eliminate that risk, it only widens it from "one intervening insertion" to "32 intervening insertions" — see `docs/TECHNICAL_SPEC.md` §12.1 for the full threat-model treatment of this tradeoff.

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

**Fee and submission model (planned, not yet implemented).** `TransferPublicInputs.fee` is already constrained in-circuit (`Σ in_value === Σ out_value + fee`) but the contract has no code today that pays it to anyone — it's proven, not collected. The planned design changes step 4 above: rather than the sender's own wallet submitting directly, an approved relayer (reusing `contracts/swap`'s existing `set_relayer`/`ApprovedRelayer` mechanism, not a second relayer system) submits the transaction, pays the real Stellar network fee from its own account, and is paid the proof-declared `fee` in return. This exists specifically because addresses are hidden in this model; submitting from the sender's own public account would link an identifiable address to the exact moment of an otherwise-private transfer. See `docs/TECHNICAL_SPEC.md` §3.5 for the full design rationale and comparison to OpenZeppelin's account-based model, which doesn't face this problem since it keeps addresses visible by design.

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
| PoC              | --> | Reviewed testnet  | --> | Production-ready |
| (fully passed)   |     | (repo is here)    |     | - final contracts|
+------------------+     +------------------+     +------------------+
                                   |                         |
                                   v                         v
                          real proofs, real value      deploy after
                          movement, live-Testnet        completed review
                          evidence — not yet an          + real ceremony
                          external review
```

What moved the repository past "PoC": real on-chain Groth16 verification for shield/transfer/unshield (not placeholders), a real, audited, live-Testnet-run shielded swap lifecycle, a real running indexer, and a real SDK cryptographic/proving core — all with live Stellar Testnet transaction evidence (`docs/POC_IMPLEMENTATION.md`, `docs/TESTNET_DEPLOYMENT.md`).

What's still needed to reach "Production-ready", at a high level:

- an *external*, independent security review of every contract and circuit — everything to date, including the audit pass described in §1.4, was done by the team building the protocol, not a third party,
- a real (non-dev), multi-party trusted-setup ceremony per circuit — every proof and verifying key in this repository so far comes from a local, single-contributor dev ceremony,
- wiring the SDK's higher-level `ZKELLASwap`/`ZKELLAAuditor`/`ZKELLACompliance` wrapper classes to the real contracts and provers underneath them (still stubs — see §2.7),
- indexer production hardening: horizontal scaling and multiple independent operators (still open — see §2.8); a first-version operational runbook now exists (`docs/RUNBOOK.md`) but is still unproven by real incident use,
- resource profiling at production scale (current measurements are from a single local host environment plus a handful of live-Testnet transactions, not sustained load),
- finalized operational controls for verifier-key rotation, pause/unpause, relayer authorization, and deployment monitoring beyond what's already implemented.

### 6.1 Component-by-component: what is built and what remains

The bullets above are the summary. This section is the detail: for each component, what is real and already exercised on live Stellar Testnet (with the transaction as evidence, not just a claim), against what specific, named work is still required. Every transaction hash below is a genuine, independently-verifiable Testnet transaction — see `docs/TESTNET_DEPLOYMENT.md` and `docs/POC_IMPLEMENTATION.md` for the complete, chronological record this section draws from.

**ShieldedToken — shield.** Built and proven live: `shield()` accepts any SEP-41 asset (the code is asset-generic, not XLM-specific — see `address_to_field_bytes` in `contracts/token/src/lib.rs`), validates the amount and encrypted-note length, computes a Poseidon2 commitment, inserts it into the Merkle tree, and only then pulls the real funds into custody via a standard SEP-41 token transfer, deliberately ordered last (checks-effects-interactions) so a malicious or reentrant token cannot exploit partially-committed state. Two real transactions confirm this end-to-end against the Native XLM Stellar Asset Contract: https://stellar.expert/explorer/testnet/tx/0722df0e01bd81ee256fb317c44a97a4e713c19fe019e27460216887fb7cacee and https://stellar.expert/explorer/testnet/tx/bbeecaeaba30517bd3a2cbc4c2f7512fd9f57b3e3b0e28b9f3bcb2998a55e945. Remaining: `MIN_SHIELD_AMOUNT` is a hardcoded Rust constant, not a governance-settable parameter, so changing it means a redeploy; no non-native SEP-41 asset has actually been shielded live yet, only native XLM, which has no issuer and therefore no clawback right, so the question of what happens to a shielded note if a real issuer claws back the contract's own custodied balance after deposit is unresolved in code; and shield handles exactly one deposit per call, with no batched multi-deposit entrypoint.

**ShieldedToken — Merkle tree.** Built and proven live: incremental insertion, root computation, and shield/note-commitment event emission all work, exercised at https://stellar.expert/explorer/testnet/tx/23d681296467f36021b2adca87d8f648acec821d1db34d37950a23816b28a711 and https://stellar.expert/explorer/testnet/tx/041460cf1932384a8ada14aa36801f314bcfbb1e1a27ce7582ce72c216f32f60. Remaining: the contract declares a `MerkleTreeFull` error for when the tree reaches capacity, but nothing in the insert path actually checks the leaf count against that limit today, so the guard exists in name only; and per-insert cost has only ever been measured against a tree with a handful of leaves, not the thousands where the fixed 32-level path starts to matter.

**ShieldedToken — transfer.** Built and proven live for the 2-in/2-out path: a genuine proof, generated through the SDK's own `generateTransferProof` using Merkle paths read from the deployed contract, was submitted and verified on-chain at https://stellar.expert/explorer/testnet/tx/90fe4d1996815f77c7c87b06a29141e01fab293dc44d01b56364be7c7e4fcf14, correctly spending two input notes and inserting two new output notes. The 4-in/4-out variant's verifying key is also live-registered (queued at https://stellar.expert/explorer/testnet/tx/3c8fb0c8bcff1a8e6fbe99be14f4b6af9cb0473280858a6be3d8999621ac5fa8, executed at https://stellar.expert/explorer/testnet/tx/320c4f2058df75acbd837b78dcaa5ede7bca1422aee708c53a9deea6bbc2ea59). Remaining: `fee` is fully constrained in-circuit (`Σ in_value = Σ out_value + fee`) but nothing in the contract actually pays it to anyone today — see §4.2's note on the planned relayer-fee design; the 4-in/4-out path has never been exercised in a live transaction. Its real compiled-WASM cost was originally measured at roughly 97% of the mainnet instruction budget; re-measuring against current stable Rust toolchains (1.92.0 through 1.94.1) instead shows it landing marginally *over* the 400M budget on every one of them (see `docs/SCF_READINESS.md` for the full comparison), too tight to rely on before the verifier's batched multi-scalar-multiplication optimization (below) lands, which is now a hard prerequisite for this path shipping on mainnet, not just a margin improvement.

**ShieldedToken — unshield.** Built and proven live as a sub-step of the swap's commit flow: https://stellar.expert/explorer/testnet/tx/21c4380b39685c9674edabb2f2830d931e8ead0d557adcff1a4aecdf66bc8038. Remaining: `unshield` takes exactly one nullifier and produces no output commitment, so a user cannot unshield part of a note's value while keeping the rest shielded as change, today requiring transfer-then-unshield as two separate operations; and unshield has never been run as its own standalone, directly-invoked transaction, only ever nested inside swap.

**Verifier registry.** Built and proven live: the on-chain Groth16/BN254 pairing check has cleared real shield proofs (https://stellar.expert/explorer/testnet/tx/0722df0e01bd81ee256fb317c44a97a4e713c19fe019e27460216887fb7cacee) and a real transfer proof (https://stellar.expert/explorer/testnet/tx/90fe4d1996815f77c7c87b06a29141e01fab293dc44d01b56364be7c7e4fcf14). Remaining: public-input aggregation loops individual point-multiplication and point-addition host calls once per input instead of calling Soroban's native batched multi-scalar-multiplication function (available since Protocol 26), real cost left on the table, most visible in transfer4's 19 public inputs; `update_verifying_key` overwrites the old key the instant a rotation executes with no retention window, so a proof built against the pre-rotation key simply fails if submitted after; and there is no batch-verification path for checking several proofs with one combined pairing check.

**Governance.** Built and proven live: the timelocked verifying-key rotation path has run for real more than once — queued at https://stellar.expert/explorer/testnet/tx/cc4809befb3742c283f612cc061e9006722968e3ec8005ae8375fe1074af3201 and activated at https://stellar.expert/explorer/testnet/tx/0131928d88132a34d1e79f8ad262389e5e83095fe61b281d64517a24ff990d42 for Shield, then repeated for Transfer at https://stellar.expert/explorer/testnet/tx/bd1e03efcb9f496790ae782acd36b1101869cf7367209be3357d8881917f696f and https://stellar.expert/explorer/testnet/tx/4bd193f369c94b66145bba90697532f95fe9da41eb06cd87030a848220526bc8 — and the two-step admin-transfer pattern (propose, then accept from the new key) works as designed. Remaining: see §2.3's guardian-role note above — `cancel_vk_update` currently requires the same single admin key as `queue_vk_update`, so a compromised key both queues a malicious rotation and is the only key able to cancel it; no pause mechanism exists on verifier, compliance, or swap today (only `token` can be halted); and `MIN_SHIELD_AMOUNT` and similar constants remain hardcoded rather than governance-settable.

**Viewing keys and compliance.** Built and proven live: `publish_compliance_proof`'s on-chain Groth16 non-membership check is confirmed correct by source review and local tests, and the compliance contract's initialization has run live at https://stellar.expert/explorer/testnet/tx/62521977b17c60b46be3d02640d5470b9fb93cf9e1235d787b678761ae898ac8; the viewing-key registry works for its current, minimal scope of registering and reading back a commitment. Remaining: `publish_compliance_proof` itself has never been submitted as a live transaction, only initialization has; viewing keys have no revocation mechanism once granted; the decrypt-on-request workflow that lets a designated party actually use a granted key to read note history doesn't exist yet; and nobody has specified who maintains the sanctions list a compliance proof checks against or how often it updates.

**Swap.** Built and proven live end-to-end: commit at https://stellar.expert/explorer/testnet/tx/21c4380b39685c9674edabb2f2830d931e8ead0d557adcff1a4aecdf66bc8038, relayer-fronted execution at https://stellar.expert/explorer/testnet/tx/5bfef119f8503f66782f0a22a4942fa43fc83c497ae71b7031ffc0025fa9fb75, and reveal-and-claim at https://stellar.expert/explorer/testnet/tx/88aebe0e9cb0239d74a746facf2af18cdbe2921d1e7d9dbdaa12c6862a91648d, with a prior internal review already finding and fixing three real issues before this state, one of which only a live transaction surfaced. Remaining: `execute_swap` accepts a relayer-supplied `amount_out` with no minimum-bound check, a real griefing path where a relayer can front a technically-valid but economically-poor amount; the stalled-swap recovery path has only ever been tested locally, never live; and no run has tested two swaps competing for the same relayer liquidity concurrently.

**Indexer.** Built and proven live: the reference implementation polls Soroban RPC, persists note and nullifier events past RPC's own short retention window in SQLite, and has synced a real shield event end-to-end (https://stellar.expert/explorer/testnet/tx/0722df0e01bd81ee256fb317c44a97a4e713c19fe019e27460216887fb7cacee), serving it back correctly through every HTTP endpoint; a previously-found silent-data-loss pagination bug and an unbounded-query-limit vector are both already fixed. Remaining: the PostgreSQL-backed, containerized version this architecture targets does not exist yet, today it is a single SQLite file with a single operator; sync behavior under sustained high load or a large backfill has never been tested; query endpoints have no request authentication or rate limiting beyond the one fix above; and multi-operator support (a second independent operator instance) is entirely unbuilt.

**SDK.** Built and proven live: the base wallet's shield and transfer paths work and have each produced a proof used in a real, on-chain-verified transaction (the two URLs above for shield and transfer). Remaining: the wallet's submission path throws on a timeout with no retry logic for a transient RPC failure or an anchor aging out of the root-history window; witness generation runs single-threaded on the main JavaScript thread with no Web Worker offloading, freezing the page during proof generation; and the `ZKELLASwap`/`ZKELLAAuditor`/`ZKELLACompliance` wrapper classes are still stubs returning placeholder values, with nothing published to npm under any tag.

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

## 8. Future work: confidential DeFi integration on Stellar

This section is design thinking, not a commitment. Nothing here is funded, scheduled, or scoped into any current deliverable; it exists to show that ZKELLA's architecture already has a credible path into confidential lending and confidential vaults, the same direction the most advanced comparable project in this space is taking on Ethereum, and to name the real engineering problems that path would actually raise before any of it is built.

### 8.1 Precedent: Zama's confidential vault on Morpho, via Steakhouse Financial

On Ethereum, Zama, Morpho, and Steakhouse Financial launched a confidential USDC yield vault: user deposits are wrapped in Zama's FHE-based confidential token (cUSDC), and the vault itself deposits the real, aggregated USDC into Steakhouse's existing Prime v2 lending strategy on Morpho Blue, collateralized by cbBTC, WBTC, and wstETH. The design's key property is where the privacy boundary sits: individual wallet balances, deposit sizes, and transaction timing are encrypted on-chain, but the vault's own aggregate statistics stay visible, so the market can still verify the vault is solvent without seeing who holds what. Morpho and the underlying lending market operate exactly as they would for any other depositor; the confidentiality is entirely in front of them, at the wrapper layer, not inside the lending protocol itself.

That structure is what makes it generalizable: it does not require the underlying DeFi protocol to know or care about privacy at all. The wrapper contract is just one more counterparty from the lending protocol's point of view, its own real, visible position, aggregating everyone confidential behind it.

### 8.2 Generalizing the pattern for ZKELLA

ZKELLA's note model is a materially different starting point from Zama's FHE-encrypted-balance model, there is no homomorphic computation here, privacy comes from Groth16 proofs over a Merkle tree of commitments and a nullifier set, but the same wrapper-layer structure applies directly, and it reuses primitives ZKELLA already has rather than requiring a new privacy mechanism:

- A **Confidential Vault contract**, structurally similar to `contracts/swap` (a separate contract, not a modification of `contracts/token`), that holds one real, transparent position in an underlying Stellar DeFi protocol, a lending market or a liquidity vault, on behalf of every confidential depositor behind it.
- **Vault-share notes**: a shielded deposit into the vault produces a note whose value represents a *share count* in the vault, not a raw asset amount, using the same commitment/nullifier/Merkle-tree machinery `contracts/token` already implements for ordinary shielded notes. No new note primitive is needed, only a new circuit that proves the deposit and share-count arithmetic correctly.
- A **public exchange rate** as the privacy-preserving yield mechanism: the vault publishes a single, protocol-wide number, current asset value per share, safe to reveal because it says nothing about any individual depositor, the same way `shielded_supply` is already public per asset today. A user's real claim is `share_count × exchange_rate`, computed inside the withdrawal circuit itself, so yield accrues without ever revealing an individual balance or a distribution event tied to one user.
- **Governance-controlled protocol allowlisting**, reusing the same pattern `contracts/swap`'s `set_relayer`/`ApprovedRelayer` mechanism already establishes: the vault should only ever deposit into a specific, governance-approved target contract, never an arbitrary address a caller supplies, for the same reason a relayer has to be approved before it can front liquidity.

### 8.3 Concrete mechanics, at a sketch level

Deposit: a user proves ownership of a spendable shielded note (the existing unshield-style ownership proof), the vault contract deposits the real, transparent asset into the underlying protocol, and a new vault-share note is minted for the user at the current exchange rate, all in one proof-gated call, structurally close to how `contracts/swap`'s `commit_swap` already escrows real value on the strength of a ZK proof.

Withdrawal: the reverse, a user proves ownership of a vault-share note, the circuit computes the real amount owed at the current exchange rate, the vault redeems that amount from the underlying protocol, and the result becomes either a new ordinary shielded note (staying confidential) or an unshielded, public balance, the user's choice, the same way `unshield` already offers today.

Solvency transparency: the vault's own aggregate state, total shares outstanding, total underlying position, mirrors `shielded_supply`'s existing role, a real, checkable number that lets anyone verify the vault isn't insolvent without learning anything about individual depositors.

### 8.4 Real open problems this would raise, not glossed over

- **Liquidity and redemption risk.** Unlike `contracts/token`'s own 1:1 custody, a lending-market position can have insufficient liquidity to redeem on demand if utilization is high elsewhere in that market. The vault has to fail a withdrawal proof-gated call gracefully under that condition, the same way the underlying protocol itself would, without that failure mode becoming a way to fingerprint which shielded note just tried to withdraw.
- **Timing and flow metadata.** The vault's own deposit and withdrawal calls into the underlying protocol are, necessarily, real and visible on-chain, an outside observer can see that the ZKELLA vault moved a given amount at a given block, just not which shielded depositor caused it. This is the same inherent limitation every pooled-privacy design shares, not something specific to this proposal, and should be stated as a known limitation rather than implied away.
- **New circuit, not a small extension.** Share-count arithmetic against a live, changing exchange rate is a genuinely new circuit, not a parameter change to shield, transfer, or unshield, real ZK engineering work comparable in scope to the swap-fairness circuit, not a configuration option.
- **Reward-token-style yield is out of scope for a first version.** A share-price-appreciation model, like Morpho's, keeps the privacy-preserving accounting tractable because there is one public number to track. A protocol that instead distributes separate reward tokens per epoch would need its own per-user, proof-gated claiming mechanism, real additional scope, not assumed away here.
- **Protocol selection is a real, ongoing governance responsibility.** Every protocol the vault is allowed to target has to be independently reviewed before being allowlisted; the allowlist mechanism controls *who* the vault can deposit into, it says nothing about whether that specific counterparty is actually safe.

### 8.5 Status

This is a credible architectural direction, grounded in a live precedent and reusing ZKELLA's existing primitives rather than inventing new ones, but it is unbuilt, uncosted, and unscheduled. It is documented here so the option is visible and understood, not because it is planned for any specific tranche or grant.

## 9. Appendices

### 9.1 Document relationships

- `docs/TECHNICAL_SPEC.md` contains full protocol details and contract interfaces.
- `docs/CIRCUIT_SPEC.md` contains circuit-level design and proof structure.
- `docs/INTEGRATION_GUIDE.md` describes SDK and integrator workflows.
- `docs/POC_IMPLEMENTATION.md` describes the dedicated PoC/current implementation status separately from the full architecture.
- `docs/TESTNET_DEPLOYMENT.md` is the current live-Testnet address and transaction record.

### 9.2 Documentation tooling note

AI-assisted tooling may be used to help format this document, structure its text, and design its diagrams.
