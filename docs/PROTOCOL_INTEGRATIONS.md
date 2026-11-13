# ZKELLA — Integrating with other Stellar protocols

**Status: research/design, nothing in this document is implemented.** This is a study of how ZKELLA could plug into the most relevant existing Soroban protocols (Soroswap, Blend, and others), written before any contract code for it exists. Treat every interface detail below as "needs to be re-verified against the live contract source immediately before writing code against it" — several of the specifics (exact `Request` enum values, exact router return types) could not be pinned down from public documentation alone during this pass; see each section's "Needs verification" note.

## 0. The core idea, and why it's not a modification to ZKELLA's existing contracts

The request that triggered this research referenced Morpho's own layering: Morpho Blue is a bare, immutable lending primitive, and MetaMorpho is a separate, permissionless vault layer built *on top of* it that adds risk curation without ever touching Blue's code. Morpho's own framing is that this keeps the base primitive simple and auditable while letting an unbounded number of independently-built, independently-risked products share its liquidity.

ZKELLA's `token`/`verifier`/`swap`/`governance`/`compliance` contracts are in the same position Morpho Blue is in: they're the audited, stable, privacy-critical core (see `docs/audit/THREAT_MODEL.md`), and every new DeFi integration is a request to extend their attack surface for a purpose that has nothing to do with what they were built and reviewed for. The right answer, mirroring Morpho Blue → MetaMorpho, is: **don't touch the core contracts at all.** Build one small, separate "adapter" contract per external protocol (`ZkellaSoroswapAdapter`, `ZkellaBlendAdapter`, …), each of which:

1. Calls the *already-existing* `unshield()` entrypoint on `token`, with itself (the adapter) as the `to` address — this is already supported today, `to: Address` in `unshield()`'s signature (`contracts/token/src/lib.rs:972`) accepts any address, contract or account, so no change is needed here at all.
2. Holds the resulting real SEP-41 tokens (XLM, USDC, whatever asset was shielded) in its own balance for the duration of one transaction.
3. Calls the external protocol's own public entrypoint (Soroswap's router, Blend's pool) using the standard `token::Client` pattern ZKELLA's own `shield()`/`unshield()` already use internally (`contracts/token/src/lib.rs:433,640,928,1092,1144`) to move those tokens — this is the exact same cross-contract-call mechanism, just pointed at someone else's contract instead of ZKELLA's own storage.
4. Takes whatever comes back (swapped tokens, a Blend position receipt, etc.) and either hands it to the user directly (if they're willing to take it unshielded) or calls `shield()`/`shield_batch()` again to fold it back into a new private note.

Nothing about this requires changing `token`, `verifier`, `swap`, `governance`, or `compliance`. It's new, separately-deployed, separately-upgradable, separately-audited code, exactly the way MetaMorpho vaults are separate contracts from Morpho Blue. If an adapter has a bug, it can be paused/abandoned/redeployed without anything touching the core shielded-pool logic ever being at risk.

## 1. The privacy boundary this creates — the one thing every integration gives up

This needs to be said plainly before anything else, because it's the actual engineering cost of every option below, not a footnote.

The moment an adapter calls `unshield()`, the **unshielded amount, the asset, and the adapter's own call to the external protocol become fully public** on Stellar's ledger, same as any other transaction. Soroswap and Blend are ordinary public Soroban contracts; they have no notion of a shielded balance and never will, because they need public balances to do AMM math or collateral accounting at all. So:

- **What stays private:** which *specific note/owner* initiated the unshield (same guarantee shield_batch already gives — k-anonymity among everyone else whose notes exist in the tree), and the recipient of the final re-shielded output (a brand-new note, unlinkable on its own to the unshield that funded it, if there's a long-enough delay or a batching layer between the two).
- **What becomes public:** the exact amount and asset moved in that single adapter call, and the fact that *some* ZKELLA user did it, at that exact block.
- **The naive, bad version of this feature:** a single-user "unshield → swap → re-shield" done directly, in one transaction, with no delay and no batching, leaks the amount and timing just as much as not using ZKELLA at all — an observer watching the adapter contract's calls learns everything a normal Soroswap trade would tell them. The only thing hidden is *who* (which shielded note) paid for it, and that's a real but much weaker guarantee than what shield/transfer/unshield give on their own.

The fix, which `shield_batch` already pioneers in this codebase, is **batching at the adapter layer**: a relayer collects N independent unshield-intents off-chain, submits them together so the adapter does N unshields and then one or few aggregated external-protocol calls, and re-shields N outputs. This is strictly more engineering than a single-user adapter call, and is the actual bar this needs to clear before calling the privacy story real rather than cosmetic. Recommendation: ship a single-user adapter first (useful, honest, clearly scoped), document the leakage above explicitly instead of implying privacy it doesn't have, and treat batching as a distinct, second milestone — not something to silently promise in v1.

## 2. Soroswap — the most natural first integration

**What it is:** the leading constant-product AMM/DEX on Soroban, with a Router contract that handles multi-hop swaps across pairs and a Factory/Pair model comparable to Uniswap V2. [DEV Community comparison of the Router contract to Uniswap V2](https://dev.to/esteblock/the-router-contract-comparison-between-soroswap-and-uniswapv2-2dmn), [`soroswap-router-sdk` on npm](https://www.npmjs.com/package/soroswap-router-sdk).

**Why it's the right one to start with:** ZKELLA already has a shielded `swap` contract with its own commit-reveal fairness circuit, but it currently has to be its own counterparty — it has no real market depth. Routing the *public* leg of a swap through Soroswap means ZKELLA users get real liquidity and real market prices instead of ZKELLA needing to bootstrap a second order book. It's also the lowest-complexity integration on this list: a swap is a single call, there's no position to track afterward, and the token flow is exactly the shield/unshield pattern ZKELLA already has tested code for.

**Concrete adapter shape:**
```
ZkellaSoroswapAdapter::unshield_and_swap(
    // unshield() args, verbatim — proof, nullifier, pub_inputs, etc.
    unshield_args: UnshieldArgs,
    // swap params
    min_amount_out: i128,
    path: Vec<Address>,       // token path through Soroswap pairs
    deadline: u64,
) -> i128   // amount received, or re-shield receipt
```
Internally: call `token::unshield(..., to: self_address, ...)` → call Soroswap Router's `swap_exact_tokens_for_tokens(amount_in, min_amount_out, path, to: self_address, deadline)` → either transfer the output to the caller directly, or call `token::shield(...)` again with the output amount to produce a fresh note.

**Needs verification before writing code:** the Router's real parameter list and return type. A mirrored copy of generated contract docs ([third-party WASM doc mirror](https://leighmcculloch.github.io/stellar-contract-wasms/4c3db3ebd2d6a2ab23de1f622eaabb39501539b4611b68622ec4e47f76c4ba07.html)) describes `swap_exact_tokens_for_tokens` taking an input amount, a minimum output, a token path, a recipient, and a deadline, returning a vector of per-hop amounts — consistent with the Uniswap V2 shape — but this was not cross-checked against Soroswap's own primary source or `docs.soroswap.finance` directly in this pass; do that before implementing, along with pinning the exact mainnet/testnet Router contract address.

## 3. Blend — the higher-value, higher-complexity integration

**What it is:** a lending/borrowing protocol on Soroban with isolated pools, where every position action (supply, withdraw, borrow, repay, and auction-related actions) goes through one `submit()` entrypoint taking a vector of typed `Request` structs. [Blend docs — integrate pool](https://docs.blend.capital/tech-docs/integrations/integrate-pool), [Blend whitepaper](https://docs.blend.capital/blend-whitepaper).

**Why it's more interesting but harder:** shielded *private collateral* is a genuinely compelling story — nobody else on Stellar is offering it — but it runs into a structural problem a swap doesn't have. A swap is a single atomic action with no state left behind. A Blend position is *ongoing* state (collateral, debt, accrued interest) that must stay associated with *some* on-chain address so Blend's own liquidation/interest logic can find and manage it later. That address is necessarily public and necessarily reusable across the position's lifetime — which is close to the opposite of ZKELLA's per-note, use-once privacy model. Two honest options, not one:
  - **Custodial-adapter model:** the adapter contract itself holds the Blend position (one shared position per adapter, or per pool, not per user), and tracks each user's share of it internally via ZKELLA-side shielded notes (a "shielded LP receipt"). This hides individual users behind the adapter's aggregate position, which is a real privacy win, but concentrates real liquidation/slippage risk management inside a new, unaudited contract — exactly the kind of thing Morpho Blue's "immutable, minimal base" philosophy warns against bolting onto a primitive.
  - **Per-user position model:** each user gets their own on-chain Blend position (simpler, safer, matches how Blend is normally used) — but then that position's address is linkable to every unshield that ever funded or drew from it, which gives up most of the privacy benefit for anything beyond a one-time deposit.

**Needs verification before writing code:** the exact current `Request` enum and `submit()` signature. Public docs only confirm two of the enum's values directly (`Deposit = 0`; two auction-related variants at 8 and 9) and describe a `from`/`spender`/`to` triple that a Code4rena audit item ("Remove 'spender' and 'to' from 'submit'") suggests may have already changed in v2 ([Code4rena Blend v2 audit](https://code4rena.com/audits/2025-02-blend-v2-audit-certora-formal-verification), [`blend-contract-sdk` README on docs.rs](https://docs.rs/crate/blend-contract-sdk/2.25.0/source/README.md)). Pin the exact deployed `submit()` signature and full `Request` enum from `blend-contracts-v2`'s `pool` crate source directly, not from docs, before writing any adapter code.

**A separate, real risk worth flagging regardless of whether ZKELLA integrates:** one weakly-sourced report (a single LinkedIn post, not corroborated elsewhere in this search pass) describes a manipulated-oracle-price exploit draining roughly $10.5M from a Blend pool (reported as the "YieldBlox" pool) in February 2026. This was not independently confirmed in this pass and should not be treated as established fact, but it's directly on-point: any integration that lets ZKELLA-held collateral sit inside a Blend pool inherits that pool's oracle-dependency risk, on top of whatever ZKELLA-specific risk the adapter itself introduces. If this integration goes forward, confirming the current state of that incident (post-mortem, fix status, which pool, whether it's the pool ZKELLA would use) is a precondition, not an afterthought.

## 4. Other Stellar protocols worth a one-line note each

- **Phoenix** — another AMM, pitched by its own team as a broader "DeFi hub" with lending/borrowing planned for later ([Phoenix: Building the First DeFi Hub on Stellar](https://medium.com/stellar-community/phoenix-building-the-first-defi-hub-on-stellar-cae669829ab5)). Same integration shape as Soroswap (swap-only adapter), evaluate once Soroswap's adapter is proven, as a second source of liquidity/routing rather than a first choice — no indication in this search pass that it offers anything Soroswap doesn't for ZKELLA's purposes specifically.
- **Aquarius** — a liquidity-management layer sitting on top of other Stellar protocols, reported to have passed $50M TVL per a Messari citation. Only relevant to ZKELLA if/when ZKELLA itself wants to bootstrap or incentivize its own liquidity (e.g. for the shielded `swap` contract's counterparty pool) — not a user-facing integration the way Soroswap/Blend are.
- **Reflector** — the oracle most consistently named across Stellar DeFi (used by Blend for pricing, and by other protocols for liquidation checks). The one place this is directly relevant to ZKELLA today, independent of any new adapter: `compliance`'s circuits/contracts don't currently consume any price oracle, but if ZKELLA ever adds a USD-denominated compliance threshold (e.g. "sanctions-screen anything over $X equivalent") rather than a raw-token-amount threshold, Reflector is the oracle to use, and it would plug into the *existing* `compliance` contract, not a new adapter.
- **FxDAO** — appeared only once, in a single weak source (a LinkedIn post listing protocols a third-party wallet API covers), with no detail on its current status. Not enough signal to say anything useful about it in this pass; revisit only if a future search turns up primary-source documentation.

## 5. What this implies for ZKELLA's own architecture, concretely

1. **New top-level directory, not new code in `contracts/token`:** something like `contracts/adapters/soroswap/` and `contracts/adapters/blend/`, each its own crate, its own `Cargo.toml`, its own test suite, built and deployed independently of the four existing contracts. This keeps `docs/audit/THREAT_MODEL.md`'s existing STRIDE analysis of the core contracts valid without re-opening it for every new integration — each adapter gets its *own* threat model entry instead.
2. **No new circuit work required for Soroswap.** The existing `unshield`/`shield` circuits and their already-audited binding (`recipient_hash`, from `contracts/token/src/lib.rs:989-996`) already support "unshield to an arbitrary contract address" and "shield from an arbitrary caller" — a swap adapter needs zero new ZK circuitry, only new plain Rust glue code and `token::Client`-style cross-contract calls.
3. **Blend, if pursued, is a research spike before it's an engineering task.** The open question isn't "can we call `submit()`" (easy, once the real signature is confirmed) — it's which of the two position models in Section 3 ZKELLA is willing to ship, because that's a privacy-model decision, not an implementation detail, and it should be made deliberately rather than falling out of whichever was easier to code first.
4. **Recommended order:** Soroswap adapter (single-user, privacy caveat documented honestly per Section 1) first, as the cheapest real integration that proves the adapter pattern end-to-end on live Testnet → batching layer for the Soroswap adapter second, to make the privacy claim actually hold → Blend research spike (pin the real `Request`/`submit` interface, pick a position model, resolve the oracle-risk question) as a separate, later decision — not bundled into the same milestone as Soroswap.

## Sources consulted

- [DEV Community: Router Contract, Soroswap vs Uniswap V2](https://dev.to/esteblock/the-router-contract-comparison-between-soroswap-and-uniswapv2-2dmn)
- [soroswap-router-sdk (npm)](https://www.npmjs.com/package/soroswap-router-sdk)
- [Third-party mirror of a Soroswap router contract's generated WASM docs](https://leighmcculloch.github.io/stellar-contract-wasms/4c3db3ebd2d6a2ab23de1f622eaabb39501539b4611b68622ec4e47f76c4ba07.html)
- [Blend docs — integrate pool](https://docs.blend.capital/tech-docs/integrations/integrate-pool)
- [Blend docs — Fund Management](https://docs.blend.capital/tech-docs/core-contracts/lending-pool/fund-management)
- [blend-contract-sdk 2.25.0 README (docs.rs)](https://docs.rs/crate/blend-contract-sdk/2.25.0/source/README.md)
- [Code4rena: Blend v2 audit (Certora formal verification)](https://code4rena.com/audits/2025-02-blend-v2-audit-certora-formal-verification)
- [Blend whitepaper](https://docs.blend.capital/blend-whitepaper)
- [Morpho — "The Morpho Stack: Not Monolithic, Not Modular, But Aggregated"](https://morpho.org/blog/the-morpho-stack-not-monolithic-not-modular-but-aggregated/)
- [Morpho docs — MetaMorpho](https://docs.morpho.org/concepts/metamorpho/)
- [Morpho — "Introducing MetaMorpho: Permissionless Lending Vaults on Morpho Blue"](https://morpho.org/blog/introducing-metamorpho-permissionless-lending-vaults-on-morpho-blue)
- [Morpho docs — shared liquidity](https://docs.morpho.org/concepts/shared-liquidity/)
- [Morpho Blue FAQ](https://docs.morpho.org/faq/morpho-blue-faq)
- [Phoenix: Building the First DeFi Hub on Stellar](https://medium.com/stellar-community/phoenix-building-the-first-defi-hub-on-stellar-cae669829ab5)
- [Decrypt: "Phoenix Rising: Building the One-Stop Shop for DeFi on Stellar"](https://decrypt.co/243194/phoenix-rising-building-the-one-stop-shop-for-defi-on-stellar)
