# ZKELLA PoC Implementation Status

This document is the dedicated PoC/current-implementation status note for ZKELLA. It explains the code that exists today, the protocol components already present in the repository, and the areas scheduled for completion during the delivery roadmap.

The existing contracts, SDK modules, tests, and deployment evidence represent only a soft PoC implementation. They are intentionally incomplete and must be reviewed, benchmarked, hardened, and improved before they can become final ZKELLA protocol contracts or production deployment artifacts.

## Full specification

The full ZKELLA protocol specification is documented separately in:

- `docs/TECHNICAL_SPEC.md` — full protocol design, cryptographic primitives, circuit inputs, contract interfaces, and architecture
- `docs/CIRCUIT_SPEC.md` — circuit-level design and proof structure
- `docs/INTEGRATION_GUIDE.md` — SDK and integration details

This document does not replace the full spec. It only describes current PoC implementation status so reviewers and contributors can distinguish existing code from the remaining delivery scope.

For a single, chronological ledger of every real on-chain transaction referenced throughout this document — across every deployment epoch, including superseded ones — see `docs/POC_TESTNET_VALIDATION.md`.

## Current PoC implementation foundation

**This section is the original, earliest status summary in this document — kept for history, but superseded by everything below it.** At the time it was written, only shield's contract logic, commitment/Merkle mechanics, and SDK note construction existed. Since then: real on-chain Groth16 verification landed for shield/transfer/unshield, the shielded swap primitive's full commit-execute-reveal lifecycle (including commit-time ownership proof and real value movement) went in and was audited, the persistent indexer became a real running service, and all of it was exercised on live Stellar Testnet — see "Update: senior audit, contract-stack redeployment, and a real live-Testnet swap lifecycle" and "Persistent indexer" further below for the current state, and "Implementation boundaries" at the end of this document for the accurate, current summary.

Original scope, for reference:

- `contracts/token` shield contract logic
- native Poseidon2 and Merkle tree support in Rust
- note commitment computation and duplicate-commitment protection
- incremental Merkle tree insertion and root tracking
- shielded supply accounting
- encrypted note bundle handling
- TypeScript SDK support for note construction and note encryption
- unit tests covering current computation and contract behavior

## Testnet deployment evidence

Date: June 13, 2026  
Network: Stellar Testnet (`Test SDF Network ; September 2015`)  
Deployer account: `GB2HC2NLXR7LHKXGS2IZL4F5LZVQVKRBKCWONQQW4WIYUXDILHORWQPZ`

### Deployed addresses

- Optimized ShieldedToken PoC contract: `CCYH6YZLJBFP6QLEQIWN7NHZCVM462L6ADEENWML6OTD3VOWR4UOEMBP`
  - Lab link: `https://lab.stellar.org/r/testnet/contract/CCYH6YZLJBFP6QLEQIWN7NHZCVM462L6ADEENWML6OTD3VOWR4UOEMBP`
- Native XLM Stellar Asset Contract used for PoC shield testing: `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC`
- Earlier non-optimized ShieldedToken deployment, superseded by the optimized instance: `CC5AXRY3PO7PBQKXTWLEL2ECVHLLVDMREZXUQIGSJZCDOIMBS5CKGAUQ`

### Successful testnet transactions

| Action | Transaction hash | Link |
| --- | --- | --- |
| Upload optimized ShieldedToken WASM | `7f40aa87c515bc1364e22882dc82868a3043a4cdf05e8d7233ef54bb1beafbb6` | `https://stellar.expert/explorer/testnet/tx/7f40aa87c515bc1364e22882dc82868a3043a4cdf05e8d7233ef54bb1beafbb6` |
| Deploy optimized ShieldedToken contract | `ec8e90bc04b44a3cbcfbf8e61e266ffb7843cf66712d67cef5bfa2384792d50b` | `https://stellar.expert/explorer/testnet/tx/ec8e90bc04b44a3cbcfbf8e61e266ffb7843cf66712d67cef5bfa2384792d50b` |
| Initialize ShieldedToken with deployer admin and placeholder verifying key | `0d84883577da8aa562ed7bc9748751a48c923973b1c2960bdab1a482046c2382` | `https://stellar.expert/explorer/testnet/tx/0d84883577da8aa562ed7bc9748751a48c923973b1c2960bdab1a482046c2382` |
| Pause ShieldedToken as admin | `9bfea719225beb0d597719ff10a90f497ec34243dacfaec6515ec26f1b5bce6a` | `https://stellar.expert/explorer/testnet/tx/9bfea719225beb0d597719ff10a90f497ec34243dacfaec6515ec26f1b5bce6a` |
| Unpause ShieldedToken as admin | `1e14e66fe2d790b93fcbc6fa029f30b2e8c3982f8db7db0a56b075664bff281d` | `https://stellar.expert/explorer/testnet/tx/1e14e66fe2d790b93fcbc6fa029f30b2e8c3982f8db7db0a56b075664bff281d` |

### Verified live state

- `leaf_count()` on the optimized ShieldedToken contract returned `0`.
- `shielded_supply(native XLM asset contract)` returned `0`.
- The contract is initialized and unpaused after the successful unpause transaction.

### Shield transaction finding

A valid PoC note was generated from the repository SDK using `sdk/dist/notes/builder.js` and `sdk/dist/notes/encrypt.js`:

- amount: `1000000` stroops
- asset: `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC`
- commitment: `88680fcb3c35673634c252517288f4229cbd1d51c721f170fcab363df643eb0a`
- encrypted note length: `176` bytes

Submitting `shield()` against the optimized ShieldedToken contract failed during Stellar testnet simulation with `HostError: Error(Budget, ExceededLimit)`, even with `--instruction-leeway 100000000` and `--resource-fee 1000000`. No shield transaction hash exists because the transaction was rejected at simulation before submission.

This is an important PoC engineering finding: deployment, initialization, admin control, and simple state reads are live on testnet, but the current on-chain Poseidon/Merkle shield path needs budget profiling and optimization before public testnet shield transactions can complete reliably.

### Update: root cause found and fixed, validated locally (not yet re-run on Testnet)

Two root causes were identified and fixed:

1. **Poseidon was pure-Rust field arithmetic**, not the native host function — Soroban's `poseidon_permutation` host function (protocol 25+) didn't exist yet when this code was first written, so `contracts/token/src/poseidon.rs` hand-rolled BN254 field arithmetic in `#![no_std]` Rust. A `shield()` call runs ~35 of these hashes (3 for the commitment + 32 for the Merkle insert) entirely in WASM-interpreted bignum math. `poseidon.rs` now also has a `Poseidon2Hasher` that calls the official `soroban-poseidon` crate's native permutation, sharing one sponge instance across all hashes in a call (the crate's own docs flag that constructing a fresh sponge per hash — the naive approach — rebuilds its parameter tables every time, which is itself significant avoidable overhead).
2. **`merkle.rs`'s empty-subtree-root lookup was O(depth²)** on a sparse/fresh tree: it recomputed the whole empty-hash chain from scratch on every tree level instead of tracking it incrementally across the one loop that already walks levels in order. Fixed to O(depth) — 32 extra hashes total instead of up to 496 on the very first insert (the worst case, and exactly the scenario this documented failure hit).

`contracts/token/shield()` now also performs **real on-chain Groth16/BN254 proof verification** via a cross-contract call to a new `contracts/verifier` registry contract (see "Implemented components" below), rather than the `TODO(M2)` stub this section originally described.

**Locally measured** (Soroban's real host environment via `soroban-sdk`'s test harness, with `InvocationResourceLimits::mainnet()` explicitly enforced — the SDK's own snapshot of the current Testnet/Mainnet instruction limit, 400M, as of 2026-07-10 — rather than the SDK's more conservative built-in local-test default of 100M, which is what this document's original failure was actually hitting):

- Full `shield()` call (commitment computation + Merkle insert + real Groth16 verification): **~104M instructions**, about 26% of the 400M budget.
- The verifier's cross-contract Groth16 check alone: **~30M instructions** of that total.
- Regression test: `contracts/token/src/lib.rs`'s `shield_fits_within_mainnet_instruction_budget`.

**What this does and does not show.** The local host environment runs the same `soroban-env-host` crate and the same instruction-cost model Testnet does, so this is a meaningful signal, not a guess. It is not, however, a live Testnet transaction — real network conditions (actual WASM execution rather than native Rust, real ledger I/O, transaction envelope overhead) can differ from the local harness. What remained open at this point — re-running this on live Testnet, repeating it to confirm repeatability, and publishing the transaction hash and resource profile — is exactly what the next update below does.

The verifier contract was also validated against a **genuine proof from the actual compiled `shield.circom` circuit** (real `circom` 2.2.3 compilation, a local dev Powers-of-Tau ceremony via `snarkjs`, a real witness from the SDK's own cross-validated test vector in `circuits/shield/shield_test_vectors.json`, and a proof independently confirmed valid by `snarkjs groth16 verify` before conversion to the contract's wire format) — see `contracts/verifier/src/lib.rs`'s `verify_accepts_real_shield_circuit_proof` test. This is separate from the budget-measurement test above, which uses a synthetic (but genuinely curve-arithmetic-correct) proof for contract-plumbing tests that need proofs matching dynamically-generated test addresses no fixed circuit artifact could match; the circuit-fidelity claim is proven once, at the verifier level, against the real circuit.

### Update: live Testnet run completed — real shield() transactions, 3× repeated

Date: August 3, 2026
Network: Stellar Testnet (`Test SDF Network ; September 2015`)
Deployer account: `GD76DVHMUR5GTTOKAD54LRBUQKHSENJYLFODIGF45YOU7XXN36FXTSAW` (freshly created and friendbot-funded for this run, scoped to ZKELLA only)

All six contracts (`verifier`, `governance`, `compliance`, `viewing_keys`, `ShieldedToken`, `swap`) were deployed and wired together on live Testnet:

| Contract | Address |
| --- | --- |
| verifier | `CCV4L5FI6CPWDSNX5MHYSXVP7NOYOFPXOJPAGHAUOY2CIXFOSFIIEA43` |
| governance | `CD5K35CAPMHOZ7UFGDLUG6TF2PJHXAEKAMEECSZOPT4YINY3X7KKFKHP` |
| compliance | `CBX7D5PGD6E6U2FHNBXFANWA3L6DDX3IX5B5AZVX6LOD4BML3D2OH3W5` |
| viewing_keys | `CCWLVJKU6ZHHAUUY567HS6QJVBEZQVX4MBCQSTNW2IAMRLE7ZNHXY2WD` |
| ShieldedToken | `CBAI76M764AFB5JQ3VFAUTIX6MBICDYMVWALV5IXCM6KSEGU5LHL2BZ7` |
| swap | `CDVGPM7LBZZAOEYFPZGANCYMRQPAKMCUJZTMODCPHVLU53IHNDAZVOQ6` |

`verifier`'s admin is `governance`'s own contract address (self-authorizing pattern: cross-contract calls from `governance` satisfy `verifier`'s `admin.require_auth()` without a signature); `ShieldedToken`, `governance`, and `compliance` all point at the same `verifier` instance. The real `shield.circom` verifying key was registered on-chain via `governance.register_vk`.

**Deploying this surfaced two real bugs**, both found only because CLI-driven live deployment exercises paths that in-process Rust tests (which call generated typed clients directly, bypassing WASM introspection and real XDR-derived addresses) do not:

1. **`verifier`'s own `CircuitType` spec metadata was dropped by the WASM linker.** `contracts/verifier` re-exported `CircuitType`/`Error` from `zkella-verifier-interface` via `pub use`. `stellar contract invoke`/`info interface` failed with `Missing Entry CircuitType` for every function on the contract, because the type's `contractspecv0` definition — physically embedded only in the interface crate's compiled rlib — didn't survive linking into `verifier`'s own WASM, even though the type name was correctly referenced in each function's spec entry. `governance`/`ShieldedToken`/`compliance`, which only `use` (not `pub use`) the same type internally, were unaffected; the retention turned out to depend on the type being defined in the same crate that exports functions using it, not on where it's canonically owned. Fixed by declaring `CircuitType`/`Error` locally in `contracts/verifier/src/lib.rs` instead of re-exporting them (they're `#[repr(u32)]` with identical variants, so still wire-compatible with `verifier-interface`'s copy used everywhere else); a `From` conversion bridges the two nominal types at the handful of test call sites that talk to both. See the doc comment above `CircuitType` in `contracts/verifier/src/lib.rs` for the full account.
2. **`address_to_field_bytes` (in `contracts/token/src/lib.rs`) read the wrong byte offset out of an `Address`'s XDR encoding**, dropping the last 4 bytes of the real 32-byte contract hash and prepending 4 bytes of a discriminant instead. It assumed `addr.to_xdr(env)` returns a bare `ScAddress` (4-byte discriminant + 32-byte hash), but it actually returns the full `ScVal` wrapper — an *extra* 4-byte tag ahead of that. Every existing test passed anyway because both sides of every test (the Rust contract logic and the test's own circuit-witness construction) called the same buggy function and stayed internally consistent; the bug only surfaces when checked against an independently-computed value — which is exactly what happens the moment a *real* Stellar contract address (the native XLM SAC, `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC`) is used, since its proof has to be built by an independent StrKey decode outside the buggy function's own reach. Caught by cross-checking `address_to_field_bytes`'s on-chain output against a from-scratch Python StrKey decode of that real address (`address_to_field_bytes_and_commitment_match_real_testnet_asset` test) before ever submitting a transaction. Fixed by reading the *last* 32 bytes of the XDR (robust to however many discriminant layers precede the hash) instead of a fixed forward offset of 4. This is a materially significant fix — uncaught, it would have made every real `shield()`/`transfer()`/`unshield()` call against a real asset/recipient address fail its Groth16 check on live Testnet, silently, no matter how correct the circuits and proofs were.

With both fixes in place, three real `shield()` transactions were submitted and succeeded, each with a genuine circuit-generated Groth16 proof (`circom` 2.2.3 + `snarkjs` groth16 prove against the real `shield.circom`/`shield.zkey`, independently verified with `snarkjs groth16 verify` before submission) and a real SEP-41 token transfer of native XLM from the deployer into the shielded pool:

| # | Amount (stroops) | Tx hash | Leaf index |
| --- | --- | --- | --- |
| 1 | `10000000` (1 XLM) | `7969b08549258d1f4f2431d8c9655ff9a4c351614276f51b195e7f69fc20e2cb` | 0 |
| 2 | `20000000` (2 XLM) | `94d864f48ad104d8d9ed01ef10750def3261e09fa7719cd21bc7fe20f81a1dc5` | 1 |
| 3 | `30000000` (3 XLM) | `1f4f719d53b802aca2cf481ea759ae0319f442c9cde869f4dcd447e28284158c` | 2 |

(`https://stellar.expert/explorer/testnet/tx/<hash>` for each.) Post-run state: `leaf_count() = 3`, `shielded_supply(native) = 60000000` (6 XLM total, matching the three deposits), `merkle_root()` non-empty and changed after each call. Each transaction's on-chain event log confirms the real token `transfer` event from the deployer to the `ShieldedToken` contract for the exact shielded amount.

This closes the Tranche 1 budget-viability milestone's remaining requirement for real. It is a live-network confirmation, not just a local-harness measurement: real WASM execution, real ledger I/O, real transaction envelope overhead, real native BN254 host-function pairing checks, real Poseidon2 native hashing — and it surfaced (and let us fix) two bugs that no amount of local `cargo test` would have caught, because both were specifically about the boundary between this code and the outside world (WASM introspection, real XDR-encoded addresses).

### Update: senior audit, contract-stack redeployment, and a real live-Testnet swap lifecycle

Date: August 3, 2026
Network: Stellar Testnet (`Test SDF Network ; September 2015`)
Deployer account: `GD76DVHMUR5GTTOKAD54LRBUQKHSENJYLFODIGF45YOU7XXN36FXTSAW`

A senior-auditor pass over `contracts/swap` found and fixed three real issues, two before this deployment and one caught only by exercising the real network:

1. **Fund-orphaning via `intent_commitment` collision.** `swap_id` is derived solely from `sha256(intent_commitment)`. Without an explicit uniqueness check, a second `commit_swap` call reusing the same `intent_commitment` (e.g. from a non-unique `intent_nonce`) would both pull a *second* real note's value into escrow and silently overwrite the first swap's `SwapState`, permanently orphaning the first escrow with no record left to reclaim it by. Fixed by checking `swap_id` uniqueness *before* the `ShieldedToken::unshield` cross-call — a duplicate is rejected before a second note's nullifier is ever spent. Regression test: `commit_swap_rejects_duplicate_intent_commitment`.
2. **Checks-effects-interactions violation in `execute_swap`.** State was updated *after* the relayer's token transfer. Since `asset_out` is an arbitrary, non-allowlisted `Address` (not a vetted token), a malicious token contract's `transfer()` could attempt a reentrant `execute_swap` call while the swap was still `Committed`, potentially pulling a second `amount_out` from the relayer before the first call's own state write landed. Fixed by writing `state.status = Executed` (and the rest of the state update) *before* the external `token::Client::transfer` call, matching the CEI pattern already used everywhere else in the contract.
3. **Missing nested cross-contract authorization in `reveal_and_claim` — found only by a real Testnet transaction, not by `cargo test`.** `reveal_and_claim` calls `ShieldedToken::shield(from = swap's own address, ...)`. Soroban auto-authorizes a contract for calls it makes *directly* (so `ShieldedToken::shield`'s own `from.require_auth()` was fine), but `ShieldedToken::shield` itself then calls `token::Client::transfer(from = swap, to = ShieldedToken, amount)` on the underlying SEP-41 asset — a call `ShieldedToken` makes, not `swap`, putting swap's required authorization a *second* hop deep in the call stack. Soroban only auto-authorizes one hop; the second needs an explicit `env.authorize_as_current_contract(...)` entry describing exactly the sub-invocation `ShieldedToken::shield` is about to make. Without it, the call fails on real (non-mocked) auth with `Error(Auth, InvalidAction)`. The existing unit tests never caught this because they use `mock_all_auths_allowing_non_root_auth()`, which — per `soroban-sdk`'s own doc comment on `authorize_as_current_contract` — explicitly does not fail if a required `authorize_as_current_contract` call is missing or wrong. This was caught only because the live demonstration below hit the real error on-chain; the fix, and the two prior audit fixes, are covered by `cargo test --workspace` (`contracts/swap/src/lib.rs`), but this specific auth gap remains **without** an automated regression test — retrofitting one requires replacing the test suite's blanket auth mock with an explicit `mock_auths`/`set_auths` tree for this one call, which wasn't done here; the real live-Testnet transactions below (both the pre-fix failure and the post-fix success, same intent/amounts, different swap contract instance) are the actual evidence this is fixed, and should be treated as the regression check until a proper unit test is added.

Because `verifier`/`governance` predated the `SwapFairness` circuit type (added earlier this session) — confirmed via `stellar contract info interface --id <address> --network testnet` against the previously-deployed instances, which showed `CircuitType` topping out at `Transfer4x4=4` — a full fresh redeploy of the wired contract set was needed rather than a partial one (`verifier`'s admin is `governance`'s address, and `ShieldedToken`/`compliance`/`swap` each fix their `Verifier` address once at `initialize()` with no rotation path, so a new verifier means new everything downstream of it):

| Contract | New address |
| --- | --- |
| verifier | `CCRLI4EAT62QVMTJR62NNJUZCERCGSYGNM534Z5R6RYSFRKELUZIG2MG` |
| governance | `CDCSHTT3R75M3BEOEDPETB3RDB4BFXI5Q2KDI2KFT3O6M73WBVUBSZWD` |
| compliance | `CA2EU46YYEBJW5C3JCRD3IAGTUD7UBPFBPYTT3I7UTESBK7FYXFCVG7Q` |
| ShieldedToken | `CDE7U6HTLMDFAEQOT5BIZ3W7VJKAQN2MFQKYVV5E3W5YIPUBSRBHAXCE` |
| swap (pre-fix, superseded — see below) | `CA4NYL2ZA67NSYOVPZMDA3YC62ARWYD52JA5NHYXRBP4TGSX3UNHBRPH` |
| swap (post-fix, live) | `CDPPRPAVKUJGNYE3AVFIBSTV7LCEOUPMM7USL7XARS2L2QRLUIMC53K3` |

Real Shield, Unshield, and SwapFairness verifying keys (from `circuits/{shield,unshield,swap}/build/wire_format_output.txt`) were registered on the new `governance`/`verifier`.

**The first end-to-end attempt, against the pre-fix `swap` contract, is itself the audit evidence for finding #3 above**: a real note was shielded (tx `99aabc85fd3b3abc7a437dd2330b6bc9b12a646e9b696a53633248891eacc117`, leaf 0), a real `unshield.circom` ownership proof committed it into escrow via `commit_swap` (tx `7c1f7fe60120902a8062b756dfe674e7148c4552cef6a0e97eb0e018a3b790f8`), a relayer fronted the output asset via `execute_swap` (tx `b701041942470e91b91ae2c4bb276cd97a9e05e8b43410505dc0757538a0482e`) — and `reveal_and_claim` then failed on-chain with `HostError: Error(Auth, InvalidAction)`, exactly as fix #3 above describes. (That swap instance's escrowed funds — 5,000,000 stroops of `asset_in`, 4,955,000 stroops of `asset_out`, both effectively the deployer's own testnet funds since it is both shielder and relayer in this demo — are recoverable via `reclaim_expired_swap` once its claim window passes; they are not lost, just temporarily locked in the now-superseded contract instance.)

After the fix (a one-line-of-reasoning, ~25-line `authorize_as_current_contract` addition — see `contracts/swap/src/lib.rs`'s `reveal_and_claim`) and redeploy, **the full commit-reveal shielded-swap lifecycle ran successfully end-to-end on live Testnet, with real Groth16 proofs at every stage and real value moving at every step — the concrete demonstration of what makes ZKELLA's swap primitive more than a confidential-token clone**:

| Step | What happened | Tx hash |
| --- | --- | --- |
| 1. Shield | Real `shield.circom` proof; 5,000,000 stroops native XLM shielded into `ShieldedToken` (leaf 1) | `09337d4f15c659dcc45d004caa7c423b0984b501168b97482b2abc0bd4f91944` |
| 2. `commit_swap` | Real `unshield.circom` ownership proof for that note; cross-call into `ShieldedToken::unshield` verified it on-chain and atomically escrowed the 5,000,000 stroops into `swap`'s own balance | `0bfb955de61f19564d793ce62dd21d12c5fc5b95b0a58bd81aa63128bbe2a971` |
| 3. `execute_swap` | Relayer (the deployer, self-approved via `set_relayer`) really fronted 4,955,000 stroops into escrow | `0fe6c726448e874005306bb9e8c637f958f5773205c0136f245c001336504574` |
| 4. `reveal_and_claim` | Real `swap_fairness.circom` proof verified on-chain (binding the revealed `amount_out`/`min_amount_out` back to the `intent_commitment` from step 2, without either having been revealed at commit time); relayer paid the escrowed 5,000,000 stroops; a **second, separate** real `shield.circom` proof verified the new output note, re-shielded into `ShieldedToken` as leaf 2 | `cc3d8a0bacfe1e70092fffba68fc81aca28d05c5aee185cea2cff337f0e60c4e` |

Every proof above (ownership, fairness, output-shield) is a genuine `circom`/`snarkjs`-generated Groth16 proof against the real compiled circuits — none of this used synthetic test-relation proofs, unlike some of the earlier local-only swap testing. `sdk/src/prover/swapFairness.ts` is a new, permanent addition to the SDK (mirroring the existing `shield.ts`/`unshield.ts` prover modules) written to generate the fairness proof for this demonstration and now available for wallet integration.

This closes the delivery-roadmap item "a dedicated audit of the swap primitive... and a live-Testnet run of its full lifecycle" from `README.md`'s "Planned implementation scope" — the swap primitive has now been audited, fixed, and run end-to-end on live Testnet with real proofs and real value movement throughout.

## Reviewer-readiness milestones

The next phase of the repository work is explicitly organized around the main reviewer feedback:

1. Budget-viability milestone — **complete: real shield() transactions on live Testnet, repeated 3×**
   - real shield transactions on Stellar Testnet reaching on-chain Groth16 verification and completing within Soroban budget — see "Update: live Testnet run completed" above,
   - transaction hashes, contract addresses, and resulting on-chain state published above.

2. Indexer resilience milestone — **the recovery mechanism itself is complete and live-Testnet-validated** (see "Persistent indexer" below); production hardening (horizontal scaling, multiple operators) remains open
   - document why a custom indexer is required for encrypted-note recovery and Merkle-path serving — done, see `docs/ARCHITECTURE.md` §1.7.4,
   - define a deployment model with replay from ledger cursors, health checks, and support for multiple independent operators — the real reference implementation replays from cursors and is health-checkable today; multi-operator support is still open.

3. Operational readiness milestone — **first version done**, see `docs/RUNBOOK.md`
   - publish an operational runbook covering deployment, monitoring, incident handling, and rollback paths — done: covers every deployed contract and the indexer, with concrete monitoring checks, key-rotation procedures, and four incident-response playbooks,
   - define incident classes for proof-failure, indexer outage, key exposure, and misconfiguration — done, all four covered in `docs/RUNBOOK.md` §4.
   - **Caveat:** this is a first draft, written against the current soft-PoC deployment, and has not been exercised in a real incident or a drill — see the runbook's own "Known limitations" section.

4. Compliance and competitive positioning milestone — **substantially addressed in the existing docs** (README's "Status highlights", this document's "Shielded swap" and "Originality" evidence)
   - position ZKELLA as a compliance-aware confidential finance stack rather than a generic confidential-token implementation,
   - document how viewing keys enable selective disclosure and regulatory-aligned workflows.

5. Ecosystem engagement milestone
   - keep public milestones, testnet evidence, and protocol updates visible in the repository and in public channels to show sustained engagement with the Stellar ecosystem.

## Implemented components

### Contracts

- `contracts/token/src/lib.rs`
  - `initialize()` stores the address of a `contracts/verifier` registry contract (the verifying key itself lives there, not in `ShieldedToken`, so it can be rotated without redeploying the token contract) and instance storage TTL handling
  - `shield()` flow: authorization, amount validation, encrypted note sizing, public-input consistency checks, commitment recomputation, duplicate commitment detection, **real on-chain Groth16 proof verification** via cross-contract call, Merkle insertion, shielded supply update, event emission, and token transfer
  - `transfer()`: 2-in-2-out note transfer (fixed arity, matching `circuits/transfer_2in2out/transfer.circom`; the 4-in-4-out circuit is not wired to a `CircuitType` yet) — arity/input validation, anchor check, nullifier-unspent and duplicate-output-commitment checks, real proof verification, nullifier/commitment state updates
  - `unshield()`: nullifier/anchor/recipient-hash validation (the circuit doesn't constrain `recipient_hash` itself — see the function's doc comment for the exact binding the contract enforces), real proof verification, nullifier spend, public token release

- `contracts/verifier/src/lib.rs` (new)
  - Groth16/BN254 verifying-key registry, one key per `CircuitType` (Shield/Transfer/Unshield/NonMembership), shared across contracts
  - `verify()`: a generic Groth16 pairing check against Soroban's native `bn254_g1_add`/`bn254_g1_mul`/`bn254_multi_pairing_check` host functions (protocol 25+) — see the module doc comment for the wire format
  - validated against both a hand-constructed (arkworks) Groth16 relation and a genuine proof from the compiled `shield.circom` circuit

- `contracts/verifier-interface/src/lib.rs` (new)
  - `CircuitType`, `Error`, and a `#[contractclient]`-only `VerifierClient` — no `#[contract]`/`#[contractimpl]` of its own. `ShieldedToken`, `governance`, and `compliance` depend on this to *call* the verifier, not on `zkella-verifier` directly. Found empirically: depending on another contract crate directly for its client pulls that crate's actual `#[contractimpl]` — including its WASM export directives — into the *caller's* compilation graph, and because Soroban contract exports are unconditional (never dead-code-eliminated), two contracts that both happen to export a function with the same name (e.g. both defining their own `initialize`) can produce a `duplicate symbol` WASM linker error. This hit `compliance` depending on `zkella-verifier` directly; `ShieldedToken` and `governance` happened not to trigger it under the codegen-unit partitioning in place at the time, which isn't something to rely on. See the crate's own doc comment for the full explanation.

- `contracts/compliance/src/lib.rs` (new)
  - sanctions/compliance non-membership proof registry, split out of `contracts/viewing_keys` (which used to store this under an unrelated key with no verification)
  - `publish_compliance_proof()` now actually verifies the proof against the verifier's `CircuitType::NonMembership` key before storing it

- `contracts/viewing_keys/src/lib.rs`
  - scoped to viewing-key commitment registration only, now that compliance records live in `contracts/compliance`

- `contracts/governance/src/lib.rs`
  - `execute_vk_update()` now actually cross-calls the verifier's `update_verifying_key()` — it used to return the new VK bytes without writing them anywhere, leaving governance and the verifier disconnected

- `contracts/token/src/poseidon.rs`
  - `Poseidon2Hasher`: native host-backed Poseidon (official `soroban-poseidon` crate), sharing one sponge instance across all hashes in a call
  - the original pure-Rust field-arithmetic implementation is kept (`poseidon2_bytes`) as the equivalence baseline its native counterpart is tested against, and by a test-only Merkle-path verification helper

- `contracts/token/src/merkle.rs`
  - incremental binary Merkle tree insertion with persistent storage, now O(depth) rather than O(depth²) for empty-subtree lookups on a sparse tree
  - empty subtree root handling and current root computation
  - Merkle path generation and verification helpers
  - `is_known_root()`: accepts the current root or any of the last `ROOT_HISTORY_SIZE` (32) roots, not only an exact match against the current one — see "Update: Merkle root-history window" below

- `contracts/token/src/types.rs`
  - storage key definitions
  - public input and event definitions
  - explicit error codes, including new ones for invalid input arity and unshield recipient-hash mismatch

### SDK

- `sdk/src/notes/builder.ts`
  - note generation, commitment math, and asset binding

- `sdk/src/crypto/bn254.ts`
  - real BN254 G1 scalar multiplication (`scalarMultBase`/`scalarMultPoint`) via `ffjavascript`'s `buildBn128()` — the same curve implementation `snarkjs` itself uses for every proof this SDK generates, not a new independently-reviewed curve library
  - covered by `tests/unit/bn254.test.ts`, including an explicit ECDH correctness check (`a*(b*G) === b*(a*G)`)

- `sdk/src/notes/encrypt.ts`
  - encrypted note bundle encoding and decryption via real BN254 ECDH (`sdk/src/crypto/bn254.ts`), replacing the previous BLAKE2b-based approximation

- `sdk/src/keys/keys.ts`
  - `transmissionKey` is a real BN254 G1 point (`vk * G`, compressed to 32 bytes), not the viewing key's raw bytes — the previous stub literally set `transmissionKey = viewingKey`, which leaked the viewing key to anyone who saw a shielded address; `vk * G` is one-way. `ZKELLAKeys.generate()`/`fromSeed()` are `async` (real curve arithmetic isn't synchronous).
  - `deriveAddress`'s diversified-address derivation is now real too: `pk_d = vk * hashToCurveG1(diversifier)`, using a genuine try-and-increment hash-to-curve construction over BN254's base field (`sdk/src/crypto/bn254.ts`'s `hashToCurveG1`, covered by `tests/unit/bn254.test.ts`). `sdk/src/notes/encrypt.ts`'s `encryptNote` takes an optional `basePoint` parameter so a sender can encrypt against a specific diversified address's base point (`g_d`) instead of the fixed generator — `tryDecryptNote` needs no corresponding change, since `vk * ephemeralPk` lands on the same shared point regardless of which base point the sender used. `tests/unit/keys.test.ts`'s `a note encrypted to a diversified address decrypts correctly with the viewing key alone` test proves this round-trips for real.

- `sdk/src/wallet/wallet.ts`
  - Every method now does something real. `shield()`/`transfer()`/`unshield()` all build real Groth16 proofs (via `generateShieldProof`/`generateTransferProof`/`generateUnshieldProof`) and submit real Soroban transactions — built with `@stellar/stellar-sdk`'s `TransactionBuilder`, correctly-typed `ScVal` struct/vec encoding (`structScVal`/`vecScVal` helpers, since Soroban structs are sorted-by-symbol maps and `nativeToScVal`'s own struct type-hint shape doesn't cover `Vec<BytesN<32>>` fields), signed, submitted via `sendTransaction`, and polled to completion via `getTransaction` — replacing the previous `submitShieldTx`/`buildShieldXdr` JSON-stub pair entirely. The contract's actual return value (not a racy subsequent state read) is what's decoded for the real leaf index/indices.
  - `transfer()`'s coin selection is simple (largest-two-notes) and honestly documented: ZKELLA's transfer circuits take exactly 2 or 4 real, already-shielded inputs with no "dummy input" support, so a wallet with fewer than 2 spendable notes of an asset can't call it — `unshield()` (full-value only) is the only spend path for a lone note.
  - `WalletConfig` gained `stellarSecret` (the Stellar account that signs/pays for transactions — distinct from the shielded-pool spending key) and per-circuit `wasmPath`/`zkeyPath` config for each of `shieldCircuit`/`transferCircuit`/`transfer4Circuit`/`unshieldCircuit`.

### Tests and vectors

- existing unit tests in `tests/unit` validate:
  - commitment computation
  - Merkle tree insertion and root calculation
  - encryption helpers and note serialization

- `tests/e2e/shield.test.ts` is intended to demonstrate the shield flow end-to-end for the current implementation foundation

- the Rust contracts workspace (`cargo test --workspace`) has 70 passing tests across `ShieldedToken` (35), `verifier` (17), `compliance` (3), `governance` (3), `swap` (10), and `viewing_keys` (2), including:
  - native-vs-pure-Rust Poseidon equivalence (including a non-canonical-input regression case)
  - real Groth16 proofs from the compiled `shield.circom`, `unshield.circom`, `transfer_2in2out/transfer.circom`, and `transfer_4in4out/transfer.circom` circuits, each independently confirmed valid by `snarkjs groth16 verify` and then verified on-chain via the actual host pairing check
  - the `shield_fits_within_mainnet_instruction_budget` regression test described above
  - happy-path and negative-path coverage for `shield()`, `transfer()`, `transfer4()` (4-in-4-out), and `unshield()`
  - a regression test proving governance's VK-rotation timelock actually reaches the verifier contract (it previously didn't — see `contracts/governance/src/lib.rs`)
  - regression tests for a security audit finding fixed in `transfer()`/`transfer4()`: the same nullifier (i.e. the same real note) submitted twice in one call was accepted, letting the underlying note's value be double-counted (`sum_in` in the circuit treats each input slot independently) and minting fabricated value in the outputs. Fixed at both the contract boundary (same-call pairwise distinctness check on nullifiers and output commitments, before proof verification) and, for defense-in-depth, in `circuits/transfer_2in2out/transfer.circom` and `circuits/transfer_4in4out/transfer.circom` (explicit non-equality constraints).
  - a regression test for a second finding: `unshield()` never decremented `ShieldedSupply`, so `shielded_supply()` only ever grew and permanently overstated real token backing after any unshield. Fixed with an explicit non-negative check (not just `checked_sub`, which only catches `i128` type-level overflow — a negative result is a "valid" `i128` that still violates the supply invariant).
  - a **critical** finding from a follow-up audit: fixing the quadratic-degree compile bug above (an unfactored expression circom's `<==` can't handle) made `circuits/common/merkle.circom` compile for the first time — and doing so exposed that it never constrained its path-direction bit (`index[i]`) to be boolean. Unconstrained, a prover can choose any field value for it, turning the intended "pick left/right sibling" logic into a full linear interpolation that lets a prover force the computed root to equal *any* value from *any* starting leaf — a complete Merkle-membership forgery, letting an attacker fabricate a note that was never actually shielded and spend it as if real. This was dormant (the circuit couldn't compile, so it was never deployable) until the compile fix made it live; fixed with the standard `index[i]*(index[i]-1) === 0` boolean constraint. The real-witness tests above (built with genuinely boolean `index` bits) are what confirm the fix doesn't break legitimate proving.
  - a related hardening fix in `circuits/swap/swap_fairness.circom`: the same class of issue, a bit-packing scheme (`amount_in * 2^32 + max_slippage_bps`) with no range constraint on `max_slippage_bps`, which would let a prover choose a value ≥2^32 to make the packing non-injective. Fixed proactively since it's a clear future landmine.
  - a **security-review finding**, fixed and validated with a real circuit proof: `swap_fairness.circom`'s `min_amount_out` public input was never bound to `intent_commitment` — only `amount_in`/`max_slippage_bps` were — so a prover could supply an arbitrarily low `min_amount_out` (e.g. 0) at reveal time regardless of the slippage tolerance actually committed to, defeating the swap's entire front-running/price-protection guarantee. This was dormant (no proof check existed) until `contracts/swap::reveal_and_claim` was wired to real on-chain verification (see below), at which point it became a live gap. Fixed by deriving `min_amount_out` in-circuit as `floor(amount_in * (10000 - max_slippage_bps) / 10000)` (the standard circom quotient/remainder pattern, plus a `max_slippage_bps <= 10000` bound so the subtraction can't wrap the field) and constraining the public input to equal that derivation — `zkella-verifier`'s new `verify_accepts_real_swap_fairness_circuit_proof` / `verify_rejects_real_swap_fairness_circuit_proof_with_forged_min_amount_out` tests cover a real compiled-circuit proof for both a correctly-derived value and the exact forged-`min_amount_out` attack the finding described (the latter is additionally confirmed to fail at `circom`'s own witness-generation time, before a proof can even be produced).
  - `swap`'s `reveal_and_claim` now does real on-chain Groth16 verification of the fairness proof (a new `CircuitType::SwapFairness` registered in `contracts/verifier`), covered by `reveal_and_claim_accepts_real_fairness_proof` / `reveal_and_claim_rejects_tampered_fairness_proof`. This is a genuinely correct, tested proof check — but it is *not* the whole swap primitive; see "shielded swap primitive" below for exactly what's still missing before any value can move.
- the TypeScript SDK (`sdk/src/prover/shield.ts`) now generates real Groth16 proofs via `snarkjs.groth16.fullProve` against the compiled `shield.circom` artifacts, replacing the previous zero-filled stub — and its wire-format encoder (`encodeProof`/`encodeVerifyingKey`, factored out to `sdk/src/prover/encoding.ts` since it's circuit-agnostic and now shared with `unshield.ts`) is cross-validated in `tests/unit/prover.test.ts` against the *exact* proof and verifying key bytes actually accepted by the real Stellar Testnet transaction documented above (tx `7969b08549258d1f4f2431d8c9655ff9a4c351614276f51b195e7f69fc20e2cb`), not just against a self-consistent round-trip. Fixing this also caught that the previous stub's `encodeProof` had two real bugs of its own — little-endian instead of big-endian per-coordinate encoding, and the wrong G2 `Fp2` coordinate order — that would have made any real proof it encoded rejected on-chain; they were never caught before because nothing had exercised it against a genuine circuit proof or a real network. `value_commit`'s blinding factor `rcv` is generated fresh inside `generateShieldProof` per call rather than stored on `Note`, since it only binds that one proof (not the note's long-term commitment/nullifier) and folding it into the persisted note plaintext would exceed `ShieldedToken`'s fixed 176-byte `ENCRYPTED_NOTE_LEN`.
- `sdk/src/prover/unshield.ts` now generates real Groth16 proofs for the unshield circuit the same way. `tests/unit/prover-unshield.test.ts` cross-validates the wire-format encoder against a real compiled-circuit proof (the same one `zkella-verifier`'s `verify_accepts_real_unshield_circuit_proof` test uses); `tests/unit/prover-unshield-e2e.test.ts` drives `generateUnshieldProof` itself end-to-end — builds a real Merkle tree with the SDK's own `poseidon2` (cross-checking it against circomlibjs's/the circuit's hash in the process), computes a real nullifier and recipient-hash binding, and confirms `circom`'s own witness calculator accepts the result (it enforces every `<==`/`===` constraint at witness-generation time, so a non-throwing call is a real soundness signal, not just "the code ran"). The caller is still responsible for sourcing the note's Merkle sibling path itself — via `ShieldedToken.merkle_path(leafIndex)` (a view call) until an indexer exists to serve it — since the prover module deliberately has no RPC dependency of its own.
- `sdk/src/prover/transfer.ts` (2-in-2-out) and `sdk/src/prover/transfer4.ts` (4-in-4-out) complete the SDK's prover coverage — every ShieldedToken entry point (`shield`, `transfer`, `transfer4`, `unshield`) now has a real SDK-side Groth16 prover, not just a stub. Both follow the shield/unshield pattern: real `snarkjs.groth16.fullProve` calls, the shared wire-format encoder, and a same-two-tier test strategy (`prover-transfer(4).test.ts` cross-validates the encoder against a real compiled-circuit proof; `prover-transfer(4)-e2e.test.ts` drives the prover itself through a real multi-note Merkle tree and lets `circom`'s own constraint-checking witness calculator be the soundness signal). Getting `transfer4`'s real-circuit proof required installing `circom` 2.2.2 (not previously available in this environment) and running a fresh local dev Groth16 setup against `pot16_final.ptau` (40,268 constraints, well under 2^16) — this is the same class of dev-only ceremony already used for shield/transfer_2in2out/unshield, explicitly not a substitute for a real multi-party ceremony, which remains open (see "What remains in the delivery roadmap" below). That real circuit proof also closed a standing gap: `CircuitType::Transfer4x4` previously had only synthetic (non-circuit) proof coverage in `zkella-verifier`'s test suite (`verify_accepts_real_transfer_4in4out_circuit_proof` now exists alongside the shield/transfer/unshield equivalents).

### Shielded swap — real value movement

`contracts/swap` now genuinely moves value, rather than just verifying a fairness proof against otherwise-inert bookkeeping. The design deliberately reuses `ShieldedToken`'s own already-real, already-audited `shield`/`unshield` paths instead of inventing new ones:

- **`commit_swap`** cross-calls `ShieldedToken::unshield(nullifier_in, swap_contract_address, ownership_proof, ...)` — a **real `unshield.circom`-shaped Groth16 proof** (not a new circuit) doubles as the swap's note-ownership proof, and the same call atomically pulls `amount_in` of `asset_in` into `swap`'s own balance (a real SEP-41 transfer) while marking the nullifier spent on `ShieldedToken`'s side. `commit_swap` also takes a `refund_to` address, used only on the unwind paths below.
- **`execute_swap`** now requires the relayer to actually front `amount_out` of `asset_out` into escrow (a real `token::Client::transfer`), not just record a number.
- **`reveal_and_claim`** verifies the fairness proof (as before), then releases real value on both sides: pays the relayer the escrowed `asset_in`, and re-shields the escrowed `asset_out` as a fresh note for the claimant via a **real, separate** `ShieldedToken::shield` call (`shield_proof` is a genuine `shield.circom` proof for the *output* note — distinct from the fairness proof, which proves a different thing). Returns the real leaf index `ShieldedToken` assigns, not a placeholder.
- **`cancel_swap`** (never executed) and the new **`reclaim_expired_swap`** (executed but never claimed, after a `CLAIM_WINDOW_LEDGERS` grace period) both really return escrowed funds — to `refund_to` and/or the relayer — rather than just flipping a status enum. This closes a real fund-lock risk: without it, a swap that's committed-but-cancelled, or executed-but-never-claimed, would strand real escrowed value in the contract permanently.

Cross-calling `ShieldedToken` from `swap` needed a new `contracts/token-interface` crate (mirroring `contracts/verifier-interface`'s established pattern exactly, for the identical reason: depending on `zkella-token` directly risks a WASM `duplicate symbol` linker error, since Soroban contract exports are unconditional). Getting the test suite to deploy a *real* `ShieldedToken` instance to exercise this also surfaced a real, unrelated packaging bug: `ShieldedToken`'s crate only built as `cdylib`, never `rlib` — meaning it (like `verifier` before the `verifier-interface` split) could never actually be used as a Rust dependency by anything, including its own dev-dependents. Fixed by adding `rlib` to its `crate-type`, matching `verifier`'s existing pattern. `contracts/swap/src/tests`'s `full_swap_lifecycle_moves_real_value` test exercises the entire flow — commit, execute, claim — with real (synthetic-relation, not real-circuit) proofs and asserts actual SEP-41 token balances move as expected at every step; `cancel_swap_refunds_escrowed_asset_in` and `reclaim_expired_swap_refunds_both_sides` cover both unwind paths.

**Update, August 3, 2026:** the swap primitive has since been through a senior-auditor pass (two additional fixes: an `intent_commitment`-collision fund-orphaning bug, and a CEI-ordering reentrancy risk in `execute_swap`) and a full real-circuit live-Testnet run of the entire lifecycle — commit, execute, reveal-and-claim — with genuine `circom`/`snarkjs` proofs for ownership, fairness, and the output shield, not the synthetic-relation proofs used in the local tests described above. See "Update: senior audit, contract-stack redeployment, and a real live-Testnet swap lifecycle" below for the fixes, addresses, and transaction hashes. `commit_swap`'s reused unshield proof establishing ownership of a note worth `amount_in`: the on-chain equality check `pub_inputs.pub_value == amount_in` is a plain argument, not something the proof itself independently re-derives, but the unshield proof fixes `pub_value` as a public input the verifier checks, so it can't be spoofed independently of the proof — this held up under the live run and is not considered open any further.

### Update: Merkle root-history window closes a reviewer-flagged reliability gap

`transfer()` and `unshield()` originally required `pub_inputs.anchor` to equal `merkle_root()` **exactly** — the current root and nothing else. This was flagged in an external technical review: since one `ShieldedToken` instance shares a single Merkle tree across every asset it wraps, *any* shield/transfer/unshield call — including on a completely unrelated asset — advances the root and invalidates every proof still in flight against the previous one. Nothing was insecure about this (no invalid proof was ever accepted), but it was a real, self-inflicted liveness problem: a proof could easily go stale between generation and submission under ordinary concurrent usage, forcing a rebuild.

Fixed by adding a root-history ring buffer (`contracts/token/src/merkle.rs`'s `is_known_root()`, `StorageKey::RootHistory`): every insertion appends the new root and evicts the oldest once more than `ROOT_HISTORY_SIZE` (32) are held, and `transfer()`/`unshield()` now accept any root still in that window instead of only the newest one. This is the standard mitigation used by Tornado Cash/Zcash-style shielded pools — it narrows the problem, it does not eliminate it: a proof anchored to a root that falls out of the last 32 insertions still needs to be rebuilt. Two dedicated regression tests (`transfer_accepts_anchor_still_within_root_history_window`, `transfer_rejects_anchor_evicted_from_root_history_window`) cover both edges of the window. `cargo test --workspace` is at 64/64 passing after this change (was 62/62).

Per-asset Merkle trees were considered and not adopted here: they would only remove *cross-asset* collisions, not same-asset ones, at the cost of extra per-asset instance-storage state — see `docs/TECHNICAL_SPEC.md` §12.1 for the full tradeoff writeup.

**Deployed as of the 2026-08-14 redeployment** (`deployments.json`, `token` `CACD4IA6OJQPG3AVGPQPJT3SJKP7YQQM4BIHUD7F7NG74KDJQLGIZQOQ` — see `docs/TESTNET_DEPLOYMENT.md`). One honest precision: the real Testnet transactions run against this deployment (two shields, the full swap lifecycle) all used the *current* root as their anchor at submission time — they exercise the baseline case, which would pass with or without this fix. The specific behavior this fix adds — accepting an anchor that's stale but still within the last 32 roots — remains verified by the two dedicated regression tests (`transfer_accepts_anchor_still_within_root_history_window`, `transfer_rejects_anchor_evicted_from_root_history_window`, run via Soroban's real host environment locally), not by a live transaction deliberately constructed to submit a stale-but-in-window anchor. What *is* now live-confirmed: the deployed contract runs this code path at all, not the old exact-equality version.

### Update: external audit — three critical swap vulnerabilities, one high-severity governance gap, and three lower-severity issues, all fixed

An external technical review of the full repository (contracts, SDK, indexer) found seven real, verified issues — three of them critical, directly threatening escrowed user funds in `contracts/swap`. Every finding was independently re-verified against the actual code before being fixed (not taken on faith), and every fix has a dedicated regression test. `cargo test --workspace` is at 70/70 passing after this change (was 64/64); the full TypeScript suite is at 103/103.

**Critical — `reveal_and_claim` never checked `fairness_pub.intent_commitment` against `state.intent_commitment`.** Once a swap reached `Executed` (public `asset_in`/`asset_out`/`amount_out`), *anyone* — not only the executing relayer — could construct their own unrelated `intent_commitment` (e.g. with `max_slippage_bps = 10000` to force `min_amount_out = 0`), produce a real, internally-valid fairness proof for it, and steal the escrowed `asset_out` by supplying their own `out_commitment`. This was a genuine fund-theft path exploitable by any chain observer, not a soundness nicety. Fixed with a single equality assertion; regression test `reveal_and_claim_rejects_mismatched_intent_commitment`.

**Critical — `commit_swap`'s reused `unshield.circom` ownership proof was replayable with a different `refund_to`.** The proof's public inputs (`anchor`, `nullifier`, `pub_value`, `pub_asset_id`, `recipient_hash`) bound the escrow only to "unshield to the swap contract's own fixed address" — identical for every user and every swap. Nothing tied the proof to *which* `commit_swap` call used it, so a party who observed a submitted-but-not-yet-final transaction (e.g. a failed/retried submission, still visible in public transaction history) could resubmit the exact same proof bytes with their *own* `refund_to`, spend the victim's nullifier first, and later drain the escrow via `cancel_swap` once it expired. Fixed by threading a new `binding_tag: BytesN<32>` parameter through `contracts/token::unshield()` (folded into `recipient_hash` as `Poseidon2(address_field(to), binding_tag)` — the circuit places no constraint on this value, so this required no circuit or trusted-setup change, only a contract-and-SDK-level convention change) and having `commit_swap` compute `binding_tag = Poseidon2(intent_commitment, refund_to)`. Direct (non-swap) unshields pass `binding_tag = [0u8; 32]`, reproducing the original formula exactly — `sdk/src/wallet/wallet.ts`'s real `unshield()` flow updated accordingly. Regression tests: `commit_swap_rejects_proof_replayed_with_different_refund_to`, `unshield_binding_tag_changes_the_accepted_recipient_hash`.

**Critical — `swap.initialize` had no re-initialization guard and no auth.** Every other contract in this workspace (`token`, `governance`, `verifier`, `compliance`) already panics on a second `initialize()` call; `swap` was the sole outlier — callable by anyone, at any time, silently overwriting `Admin`/`Verifier`/`Token` on an already-operating, already-funded contract. Fixed to match the established pattern exactly. Regression test: `initialize_cannot_be_called_twice`.

**High — `governance::register_vk` activated a circuit's first verifying key instantly, with no timelock**, on the reasoning that a brand-new circuit "isn't relied upon yet" so an instant key carries no soundness risk. That reasoning is false the moment any real value exists anywhere in the system: every circuit's proofs are ultimately checked against the same shared `ShieldedToken` Merkle tree and `shielded_supply` bookkeeping, so a malicious VK activated instantly for *any* circuit — even one that never had a key before — can forge output notes and drain value already resting in the pool via an already-legitimate circuit like `Unshield`. This was not hypothetical: on the live Testnet deployment, `Transfer`/`Transfer4x4`'s VKs are still unregistered as of this writing (see "Current live contracts" in `docs/TESTNET_DEPLOYMENT.md`), meaning this exact gap was live and unaddressed for those two circuits. Fixed by removing the separate fast path entirely — `register_vk` no longer exists; first-time registration now goes through the same `queue_vk_update`/`execute_vk_update` 7-day-timelocked path as a replacement, with `execute_vk_update` now choosing `register_verifying_key` or `update_verifying_key` depending on whether the circuit already has a key. Regression test: `execute_vk_update_performs_first_time_registration_through_the_timelock`.

**Medium — `reclaim_expired_swap` could panic permanently on an extreme `expiry_ledger`.** `state.expiry_ledger + CLAIM_WINDOW_LEDGERS` used a plain `+`, which panics on overflow in the release build (`overflow-checks = true` — confirmed, not assumed) for an `expiry_ledger` near `u32::MAX`, permanently locking both the relayer's fronted `asset_out` and the claimant's escrowed `asset_in` with no recovery path. Fixed at the source: `commit_swap` now rejects any `expiry_ledger` that would overflow that later addition, plus `checked_add` in `reclaim_expired_swap` itself as defense-in-depth. Regression test: `commit_swap_rejects_expiry_ledger_that_would_overflow_the_claim_window`.

**Medium — `ZKELLAWallet.shield()`'s `opts.to` was completely ignored.** The parameter existed in the type signature and doc comment, but the function body always encrypted to `this.config.keys.transmissionKey` (the sender's own key) regardless — a deposit "for a recipient" silently remained spendable only by the sender and undecryptable by the intended recipient. Fixed to use `hexToBytes(to)` when provided, matching `transfer()`'s already-correct handling of the same parameter shape in the same file. Three regression tests in `tests/unit/wallet-shield-recipient.test.ts`, including a genuine encrypt/decrypt round trip proving the recipient (and only the recipient) can decrypt the resulting note.

**Low/Medium — the indexer's event sync could silently skip events under high load, and `/notes` had no `limit` cap.** `indexer/src/sync.ts` advanced its cursor via `lastEventLedger + 1`, which — for the very unlikely but real case of a single ledger containing more matching events than the page size (1000) — would skip the remaining events in that ledger permanently (they are never fetched again). Fixed to resume mid-page via the RPC response's own `pagingToken` (a `cursor`-based continuation Soroban RPC's `getEvents` already supports) rather than jumping straight to the next ledger. Separately, `/notes`'s `?limit=` query param flowed unvalidated into a SQL `LIMIT ?` clause — since SQLite treats a *negative* `LIMIT` as "unlimited," this was a real unbounded-query vector, not just a resource-exhaustion risk for large values. Fixed with `parseNotesLimit()`, clamped to `[1, 1000]`, unit-tested in isolation (`tests/unit/indexer-http-limit.test.ts`).

**Update, 2026-08-14: the five contract-level fixes above are now live-deployed and demonstrated on real Testnet.** See `docs/TESTNET_DEPLOYMENT.md`'s "Update: live redeployment closes all seven findings on real Testnet" for the transaction hashes — including the governance timelock actually being queued, waited on for real (a `testnet-fast-timelock` build was used specifically so this could run in one sitting — see that doc for why), and executed; two real shield transactions; and a full swap lifecycle whose `commit_swap` and `reveal_and_claim` each exercise one of the two Critical proof-binding fixes directly. The two SDK/indexer-level fixes (`shield()`'s recipient handling, indexer pagination/limit) aren't "redeployed" in the same sense — they ship whenever a consumer updates to the current SDK/indexer code — and haven't been separately demonstrated against a live two-party shield or a real high-load indexer run; their regression tests remain the evidence for those two specifically.

### Persistent indexer

`indexer/` is a real, running reference implementation of the indexer `docs/ARCHITECTURE.md` describes as necessary (Stellar RPC's own event retention window is too short for wallets to recover note/nullifier history beyond it). It polls `SorobanRpc.Server.getEvents` for `ShieldedToken`'s `("zkella","note")`/`("zkella","nf")` events, persists them in SQLite (Node's built-in `node:sqlite`, no external database dependency), and serves them over HTTP in exactly the shape `sdk/src/indexer/client.ts`'s `IndexerClient` already expected — `merkle_root`/`merkle_path` are proxied live to `ShieldedToken` itself rather than duplicated, since the contract is already the source of truth for current tree state.

Validated against live Stellar Testnet, not just locally: submitted a fourth real `shield()` transaction (tx `a82d7bd240d3bbf9b4caeb9f7c2737023e3dfafc8b55a469e3c44812a3c4c243`, leaf index 3, 5 XLM), then confirmed the indexer's sync loop correctly found and persisted the real `("zkella","note")` event (exact commitment/encrypted-note/leaf-index match), and that every HTTP endpoint — `/health`, `/notes`, `/merkle/root`, `/merkle/path/:i`, `/commitment/:hex`, `/nullifiers/batch` — returned correct data, including the `/merkle/root` and `/merkle/path` endpoints' live proxy calls into the real deployed contract. That live run also caught a real bug in the process: the view-call simulation helper used a hand-typed placeholder Stellar address with an invalid StrKey checksum (`accountId is invalid`); fixed by generating a real keypair once at module load instead.

Not yet covered: horizontal scaling, multiple independent operators, and alerting on sync failures/backfill tooling — this is a correct, working reference implementation of the API surface, not a production deployment. `node:sqlite` is still an experimental Node API as of the version used here. A first-version operational runbook covering this service now exists — see `docs/RUNBOOK.md`.

## What remains in the delivery roadmap

These capabilities are not yet implemented in the current repository and remain part of the delivery roadmap:

- systematic review and improvement of all existing soft PoC contracts and SDK code before finalization
- BN254 `verifying_key` structural validation beyond wire-format length checks (`contracts/verifier` validates shape, not that the bytes encode a VK from a specific audited circuit)
- indexer horizontal scaling, multi-operator support, and alerting/backfill tooling — the indexer itself is real and live-Testnet-validated (see above), this is about running it at production scale
- a *proven* operational runbook and incident-response plan — a first version now exists (`docs/RUNBOOK.md`, deployment, monitoring, rollback, key handling, and escalation paths for contract failures, indexer outages, key exposure, and misconfiguration), but it hasn't been exercised in a real incident or a drill; hardening it based on real use remains open
- a real (non-dev) Groth16 trusted-setup ceremony for each circuit — the one used for every real-circuit test and every live-Testnet transaction in this repository is explicitly a local dev ceremony (single contributor, not a public multi-party computation), not suitable for any deployment handling real value
- an external, independent security review — everything in this document, including the senior-audit pass described above, was performed by the same team building the protocol, not a third party

**Closed, no longer open:** the nested cross-contract authorization fix in `reveal_and_claim` now has a dedicated regression test — `reveal_and_claim_authorize_as_current_contract_satisfies_real_non_mocked_auth` in `contracts/swap/src/lib.rs`. It was validated only by real live-Testnet transactions until an OpenZeppelin engineer, asked directly, confirmed the fix: switch off blanket auth mocking (`mock_all_auths_allowing_non_root_auth()`) for the specific call that exercises `authorize_as_current_contract`, and use `env.set_auths(&[])` with real (non-mocked, strict) Soroban authorization checking instead — plus explicit `MockAuth` entries only for the one real external signer (`relayer`) still needed elsewhere in the same test. Confirmed as a genuine regression test, not a tautology: temporarily removing the `authorize_as_current_contract` call from `reveal_and_claim` makes this specific test fail with the exact same `Error(Auth, InvalidAction)` the original live-Testnet failure hit, before the fix was restored.

## Implementation boundaries

This repository is best understood as:

- a full technical specification and architecture for the ZKELLA protocol
- a soft PoC implementation with a working core: shield/transfer/unshield with real on-chain Groth16 verification and real Soroban RPC submission, a shielded swap primitive that genuinely moves value (reusing `ShieldedToken`'s own shield/unshield paths), a real BN254 ECDH/hash-to-curve key and encryption layer including diversified addresses, and a real (if reference-scale) persistent indexer — all validated both locally and with real transactions on live Stellar Testnet
- a codebase that still schedules a real multi-party trusted-setup ceremony, an *external* security review, and indexer production-scale hardening for delivery-roadmap completion — the internal audit of the swap primitive's ownership↔intent binding is done (see "Update: senior audit..." above)

It is not yet a complete implementation of the full ZKELLA specification, and none of this has been through a security review or a real (non-dev) trusted-setup ceremony. Existing contracts and code should be treated as reviewable PoC material only, not as final or production-ready protocol logic — the cryptographic core working correctly in tests is necessary, not sufficient, for that.

## How to use this document

Use this doc when you want to understand which repository files are currently implemented and which features remain in the delivery roadmap. Use `docs/TECHNICAL_SPEC.md` and `docs/ARCHITECTURE.md` for the full protocol semantics, cryptographic design, and system architecture.
