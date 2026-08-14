# ZKELLA — Design Exploration & Improvement Roadmap

This document consolidates every improvement and exploration point identified while reviewing ZKELLA against OpenZeppelin's Confidential Token protocol (`OpenZeppelin/stellar-contracts`, `packages/tokens/src/confidential`) and other prior art (Railgun, Penumbra, Aztec Connect), plus the public-AMM integration question for the shielded swap. It is a planning document, not a record of completed work — each item states the finding, why it matters, and a concrete next step. Status is tracked per item and should be updated as work happens; this file is not meant to go stale the way earlier "planned" sections in other docs did before this pass.

---

## Tier 1 — Cheap, well-scoped, low/no open design questions

### 1.1 Document the `Range64` boundary explicitly
**Status: done** (this session, `docs/CIRCUIT_SPEC.md` §1.3). Every circuit constrains amounts to 64 bits while Soroban contracts pass `i128`. Not a practical issue for any realistic Stellar asset amount, but was previously an undocumented assumption rather than a stated design boundary.

### 1.2 ECDH point/negation-collision resistance
**Status: verified safe, no action needed.** OpenZeppelin's `DESIGN.md` documents an explicit fix for `ECDH(r_e, P)` colliding with `ECDH(r_e, -P)` when only the x-coordinate is bound. Checked `sdk/src/notes/encrypt.ts`/`sdk/src/crypto/bn254.ts`: ZKELLA's shared secret is `blake2b(sharedPoint)` over the full compressed point (`toRprCompressed`), which encodes the y-parity — not vulnerable to this specific collision class. Worth a one-line addition to `docs/TECHNICAL_SPEC.md`'s threat model table stating this was checked and confirmed safe, so a future reviewer doesn't have to re-derive it.

### 1.3 Expand the single compliance hook
**Status: design note written (`docs/ARCHITECTURE.md` §2.5), implementation not started.** `contracts/token` currently has, at most, a minimal, optional single hook — nowhere near OpenZeppelin's 8-point generic `Hooks` trait, which is deliberately not being copied wholesale (judged disproportionate to current needs). The proposed middle ground — three targeted hooks (`on_shield`, `on_transfer`, `on_unshield`), decoded-parameters-only, no-op default — is now written up in §2.5, explicitly flagged there as a proposal to evaluate against `contracts/token`'s real call-site structure, not a committed design. Next step: that feasibility check, before any code changes.

### 1.4 Formalize indexer-operator vs. wallet compliance role separation
**Status: done (doc-only).** Added to `docs/ARCHITECTURE.md` §2.8's indexer trust-boundaries list — names the distinction OpenZeppelin's `INDEXER.md` makes explicit (indexer operator vs. wallet as separate roles with potentially different compliance obligations) and notes it hasn't mattered operationally yet since ZKELLA's indexer is self-hosted per deployment today, but will matter once multi-operator indexing becomes real.

---

## Tier 2 — Real design work required before implementation

### 2.1 Fine-grained relayer delegation
**Status: open, roadmap.** `swap.set_relayer` is a binary allowlist — an approved relayer can front any amount, no cap. OpenZeppelin's `set_spender`/`revoke_spender` (confidential allowance with `live_until_ledger` expiry) is a more precise pattern worth adapting if ZKELLA ever moves beyond a small set of trusted relayers toward a more open relayer market. Not urgent while relayer participation is small and permissioned.

### 2.2 Off-chain RFQ/quote negotiation protocol for swap relayers
**Status: implemented (`sdk/src/relayer/quote.ts`), 6/6 unit tests passing (`tests/unit/relayer-quote.test.ts`), typecheck clean.** `SwapQuoteClient.requestQuote()` (wallet side) and the `RelayerQuoteHandler` type (relayer side) specify the wire format and, critically, `quoteRespectsSlippage()` enforces the *exact same floor* `circuits/swap/swap_fairness.circom` checks on-chain (`floor(amountIn * (10000 - maxSlippageBps) / 10000)`, integer division, verified bit-for-bit equivalent in tests) — a wallet that only acts on quotes passing this check cannot be talked into a swap that would later fail its own fairness proof. Deliberately scoped as a client library + handler type, not a full relayer server (unlike `indexer/`'s reference service) — pricing/inventory logic is inherently operator-specific.

**Real gap this does *not* close, found during design review (senior-auditor pass on this module itself):** `commit_swap` takes no relayer parameter — any approved relayer can call `execute_swap` for a given `swap_id` first, regardless of which relayer issued the quote a wallet acted on. This RFQ protocol coordinates price discovery; it does not reserve execution rights. Closing that would require a `contracts/swap` change (e.g. an optional preferred-relayer field with a short grace window before falling back to any approved relayer) — a real contract-level follow-up, not an SDK-level one, tracked here rather than silently left implicit.

### 2.3 AMM-sourced execution as an explicit fallback path (RFQ-first hybrid)
**Status: design reviewed, Router-vs-Aggregator decided, implementation deliberately deferred — not scheduled yet.** Confirmed by researching the real Soroban AMM landscape (not assumed): no single canonical AMM exists — Soroswap, Phoenix, and Aqua are independent protocols; the Soroswap Aggregator unifies them via a `SoroswapAggregatorAdapterTrait` adapter pattern, but **the decision (this session) is Router-only for v1** (`SoroswapRouter`'s `swap_exact_tokens_for_tokens(amount_in, amount_out_min, path, to, deadline)` directly) — a smaller, single-contract dependency to audit and reason about, with multi-AMM reach via the Aggregator explicitly deferred to a later widening once this path has real operational experience, not rejected.

Chosen shape, decided with the user: **RFQ-first, AMM-sourced execution as an explicit, separate, non-default fallback** (`execute_swap_via_amm`, distinct from `execute_swap`) — not a replacement of the relayer path. The privacy/MEV cost documented previously (§ below) is unchanged by this framing; what changed is that it's now scoped as an opt-in path used only when RFQ (§2.2) finds no competitive relayer, not a default behavior change.

**Security findings from this review, all incorporated into `docs/ARCHITECTURE.md` §1.7.5b:**
- `SoroswapRouter`'s address **must** be admin-allowlisted (`set_approved_router`) — an unrestricted caller-supplied address is a direct theft vector.
- `path` restricted to a direct pair for v1 (no multi-hop) to minimize attack surface.
- **Changed from an earlier looser sketch**: not fully permissionless — `refund_to.require_auth()` required, so a third party can still relay the call (pay the fee) but cannot pick unfavorable execution parameters (`min_amount_out`) without the original committer's authorization.
- `SwapState`/`reclaim_expired_swap` need a real `ExecutionKind::RelayerFronted | AmmSourced` distinction, not an overload of the existing `Option<Address>` relayer field — a genuine data-model change requiring the existing `contracts/swap` regression suite to be re-verified against it, not a cosmetic addition.

**Open questions still to resolve, once this is picked up** (full list in `docs/ARCHITECTURE.md` §1.7.5b): confirm `SoroswapRouter`'s real audit status and deployed address per network; design the exact `ExecutionKind` state machine change; design the wallet-side UX for surfacing the privacy trade-off at the moment a user opts into this path, not just in documentation.

**Still true, unchanged by this review:** this does not solve MEV or restore `amount_out` privacy — using the Router instead of a raw pool improves pricing, not the underlying privacy/MEV trade-off, which remains exactly as costly as when a raw AMM was the only option considered.

**Implementation timing: explicitly deferred, not this pass.** The user confirmed the direction (Router-only) but asked to keep this as documented design for now — no `contracts/swap` code changes for this item until picked back up deliberately.

### 2.4 Re-evaluate root-history window depth with real collision data
**Status: open, needs Testnet measurement.** The `ROOT_HISTORY_SIZE = 32` window (`contracts/token/src/merkle.rs`, added this session) is a reasonable default, not a measured-optimal one. Once there's meaningful concurrent Testnet activity, measure actual `InvalidAnchor` rejection frequency and revisit whether 32 is comfortably oversized, right-sized, or still too tight — and re-examine whether per-asset trees (previously considered and not adopted, `docs/TECHNICAL_SPEC.md` §12.1) become worth their storage cost once wrapping more than one or two assets concurrently in practice, rather than deciding this in the abstract.

---

## Tier 3 — Major architectural evaluations, no implementation commitment implied

### 3.1 Evaluate a universal/updatable-SRS proof system as a Groth16 alternative
**Status: confirmed with OpenZeppelin directly (their engineer, Boyan) — evaluation still to be scoped.** Their UltraHonk SRS is universal and production-ready, derived from the real Aztec Ignition multi-party ceremony (176 participants — see `https://github.com/AztecProtocol/ignition-verification`), not a dev-only setup. This directly removes the per-circuit trusted-setup problem that is ZKELLA's most-repeated open caveat across every doc (`SECURITY.md`, `docs/CIRCUIT_SPEC.md` §9, `README.md`) — confirmed real, not hypothetical, so the migration question (circom→Noir, Groth16→Honk/PLONK-family, new verifier contract, new SDK proving pipeline) is now worth a real feasibility scoping pass, not just a "worth asking about" placeholder.

### 3.2 Evaluate real Pedersen G1 homomorphic commitments vs. the current Poseidon-sum-constraint approach
**Status: closed — confirmed by OpenZeppelin directly, no gap found.** Their engineer confirmed our own analysis independently: "I don't think you're missing anything... it's because zkella is note-based, while ours is account-based. We need two different balances to prevent in-flight proof invalidation when the owner tries to spend. We need the merge to be proofless for that reason." Their on-chain homomorphic `merge()` is specifically an account-model mechanism (folding one account's two balances together) — it doesn't transpose to a note-based design with no single "balance" to fold. No further evaluation needed here; this item is resolved, not just deprioritized.

### 3.3 Proof-replay binding pattern — cross-validated against OpenZeppelin's own fix
**Status: closed — our `binding_tag` fix (§ closed Critical finding, `docs/POC_IMPLEMENTATION.md`'s "Update: external audit") independently matches OpenZeppelin's own pattern.** They hit the same class of issue in their `Register` operation and fixed it the same way we did: folding an authorizing address (`acct_f`) into the public inputs to prevent replay across different contracts (`OpenZeppelin/stellar-contracts` PR #775). One correction to our own prior assumption: their `sigma`/`r_e` mechanism is *not* replay protection — sigma is a fresh per-operation salt and r_e is derived from it for an unrelated reason (a wallet convention letting the sender recompute r_e from the public event alone, for storage-independent selective disclosure later). We had assumed sigma/r_e served the binding role their `Register`-operation fix actually serves; noted here so this doesn't get misremembered in a future design discussion.

### 3.4 Testing methodology for nested `authorize_as_current_contract` — done
**Status: closed — implemented and verified.** OpenZeppelin's engineer confirmed this is a known, real pain point industry-wide (not just our bug) — they haven't needed `authorize_as_current_contract` in their own library, but agreed blanket auth mocks (`mock_all_auths`/`mock_all_auths_allowing_non_root_auth`) are the wrong tool for testing it, recommending hand-constructed auth entries instead. Implemented as `contracts/swap/src/lib.rs`'s `reveal_and_claim_authorize_as_current_contract_satisfies_real_non_mocked_auth`: `env.set_auths(&[])` switches off blanket mocking for the specific call that exercises `authorize_as_current_contract`, running Soroban's real, strict authorization checker, with explicit `MockAuth` entries only for the one real external signer (`relayer`) still needed earlier in the same test. Verified as a genuine regression test, not a tautology, by temporarily removing the `authorize_as_current_contract` call and confirming the test fails with the same `Error(Auth, InvalidAction)` the original live-Testnet incident hit. Closes the open item this had in `docs/POC_IMPLEMENTATION.md`'s roadmap section.

---

## Explicitly not planned (considered and set aside, with reasoning)

- **Copying OpenZeppelin's account-based, sender/recipient-visible privacy model.** Would eliminate ZKELLA's core value proposition (unlinkable transaction graph). Not a improvement, a different product.
- **A full generic `Hooks` trait with many extension points**, matching OpenZeppelin's exactly. Judged disproportionate to current compliance needs — see 1.3 for the scaled-down alternative under consideration instead.
- **Wiring the classic Stellar DEX/liquidity pools into `execute_swap`.** Structurally unreachable from a Soroban contract (transaction-level operations, not contract-callable host functions) — this is a platform constraint, not a design choice to revisit.

---

## How to use this document

Update the **Status** line on an item when work starts, and move it (or split it) into the relevant real spec doc (`docs/ARCHITECTURE.md`, `docs/TECHNICAL_SPEC.md`, `docs/CIRCUIT_SPEC.md`) once a decision or implementation lands, rather than letting this file and the "real" docs drift apart. Items in "Explicitly not planned" should stay here as a record of the reasoning, not be deleted — a future reviewer asking "did you consider X" deserves a documented answer, not silence.
