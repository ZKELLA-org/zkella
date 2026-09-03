# ZKELLA — PoC Testnet Validation Report

This document is a single, chronological, standalone ledger of every real on-chain transaction ZKELLA has submitted against Stellar Testnet, from the first deployment attempt through the current live deployment. It exists so a reviewer can independently verify the PoC's on-chain claims — budget viability, real Groth16 verification, real value movement in the shielded swap primitive, and every fix along the way — from transaction hashes alone, without having to reconstruct the history from several narrative documents.

It does not replace the two documents it draws from:

- `docs/TESTNET_DEPLOYMENT.md` is the point-in-time record of the **current** live deployment only (addresses, setup transactions, and the most recent redeployment's evidence) — it is overwritten whenever the contracts are redeployed.
- `docs/POC_IMPLEMENTATION.md` is the narrative status document — what's implemented, what bugs were found and fixed and why, what remains open — with transaction hashes woven into that story as supporting evidence.

This report is the complement to both: it keeps the *entire* transaction history, including superseded deployments, in one place, organized strictly by when each transaction actually happened, so nothing from earlier runs gets lost when a later document is overwritten.

**Every proof-bearing transaction below was independently verified before submission** — the underlying Groth16 proof confirmed valid with `snarkjs groth16 verify` against the real compiled circuit, and every cryptographic value that on-chain state depends on (Merkle anchors, nullifiers, commitments, `binding_tag`/`recipient_hash`) checked against the contract's own live view calls before being used in a proof, not assumed from local bookkeeping. Where a submission failed on-chain, that failure and its transaction hash (where one exists) are included rather than omitted — this is a validation record, not a highlight reel.

All transactions are on `Test SDF Network ; September 2015`; the pattern `https://stellar.expert/explorer/testnet/tx/<hash>` resolves any hash below.

---

## How to read this

- **Epoch** = one deployed contract stack, in the order it was actually deployed. Contracts are redeployed (new addresses) whenever a contract-level change requires it — a new `CircuitType`, a breaking interface change, a fix to something with no in-place migration path.
- Only **Epoch 4** (the last section below) is the current live deployment — see `docs/TESTNET_DEPLOYMENT.md` and `deployments.json` for its addresses as the canonical, always-current source. Epochs 0–3 are historical and superseded; they're kept here because they're the actual evidence for the bugs found and fixes made along the way.
- "Real proof" means a genuine `circom`/`snarkjs`-generated Groth16 proof against the actual compiled circuit for that call, independently verified with `snarkjs groth16 verify` before submission — not a synthetic/hand-constructed proof (those exist only in the local `cargo test` suite, never on-chain).

---

## Epoch 0 — first deployment attempt: budget blocker found (June 13, 2026)

Deployer: `GB2HC2NLXR7LHKXGS2IZL4F5LZVQVKRBKCWONQQW4WIYUXDILHORWQPZ`

| Contract | Address |
| --- | --- |
| ShieldedToken (optimized PoC build) | `CCYH6YZLJBFP6QLEQIWN7NHZCVM462L6ADEENWML6OTD3VOWR4UOEMBP` |
| ShieldedToken (earlier, non-optimized, superseded same day) | `CC5AXRY3PO7PBQKXTWLEL2ECVHLLVDMREZXUQIGSJZCDOIMBS5CKGAUQ` |

Native XLM Stellar Asset Contract used throughout (unchanged in every epoch below): `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC`

| Action | Tx hash |
| --- | --- |
| Upload optimized ShieldedToken WASM | `7f40aa87c515bc1364e22882dc82868a3043a4cdf05e8d7233ef54bb1beafbb6` |
| Deploy optimized ShieldedToken contract | `ec8e90bc04b44a3cbcfbf8e61e266ffb7843cf66712d67cef5bfa2384792d50b` |
| Initialize (deployer admin, placeholder verifying key) | `0d84883577da8aa562ed7bc9748751a48c923973b1c2960bdab1a482046c2382` |
| Pause as admin | `9bfea719225beb0d597719ff10a90f497ec34243dacfaec6515ec26f1b5bce6a` |
| Unpause as admin | `1e14e66fe2d790b93fcbc6fa029f30b2e8c3982f8db7db0a56b075664bff281d` |

A valid PoC note was generated from the repository SDK (amount `1000000` stroops, commitment `88680fcb3c35673634c252517288f4229cbd1d51c721f170fcab363df643eb0a`), but **`shield()` failed at simulation** with `HostError: Error(Budget, ExceededLimit)`, even with `--instruction-leeway 100000000 --resource-fee 1000000` — rejected before submission, so no transaction hash exists for it, and this note was never actually shielded. Verified live state at the time: `leaf_count() = 0`, `shielded_supply(native) = 0`.

**Root cause (found later, see Epoch 1):** two independent issues — pure-Rust Poseidon field arithmetic instead of the native host function, and an O(depth²) empty-subtree lookup in the Merkle insert path — both fixed before the next real attempt.

---

## Epoch 1 — budget fix validated live: real `shield()` transactions, 3× (August 3, 2026)

Deployer: `GD76DVHMUR5GTTOKAD54LRBUQKHSENJYLFODIGF45YOU7XXN36FXTSAW` (freshly created and friendbot-funded, scoped to ZKELLA only — this is the deployer account used for every epoch from here on)

All six contracts deployed and wired together for the first time:

| Contract | Address |
| --- | --- |
| verifier | `CCV4L5FI6CPWDSNX5MHYSXVP7NOYOFPXOJPAGHAUOY2CIXFOSFIIEA43` |
| governance | `CD5K35CAPMHOZ7UFGDLUG6TF2PJHXAEKAMEECSZOPT4YINY3X7KKFKHP` |
| compliance | `CBX7D5PGD6E6U2FHNBXFANWA3L6DDX3IX5B5AZVX6LOD4BML3D2OH3W5` |
| viewing_keys | `CCWLVJKU6ZHHAUUY567HS6QJVBEZQVX4MBCQSTNW2IAMRLE7ZNHXY2WD` |
| ShieldedToken | `CBAI76M764AFB5JQ3VFAUTIX6MBICDYMVWALV5IXCM6KSEGU5LHL2BZ7` |
| swap | `CDVGPM7LBZZAOEYFPZGANCYMRQPAKMCUJZTMODCPHVLU53IHNDAZVOQ6` |

The real `shield.circom` verifying key was registered on-chain via `governance.register_vk` — the fast, untimelocked first-registration path that existed at the time (removed later; see Epoch 4).

**Two real bugs surfaced by this deployment**, neither reproducible by `cargo test` (both are specifically about the boundary between the contract and the outside world):

1. `verifier`'s `CircuitType` spec metadata was dropped by the WASM linker (`pub use` re-export doesn't survive linking the way `use` does) — `stellar contract invoke`/`info interface` failed with `Missing Entry CircuitType`.
2. `address_to_field_bytes` read the wrong byte offset out of an `Address`'s XDR encoding, dropping the last 4 bytes of the real 32-byte contract hash. Every unit test passed anyway because both sides of each test called the same buggy function and stayed internally consistent; it only surfaced against an independently-computed value from a real address.

With both fixed, three real `shield()` transactions succeeded — each a genuine `circom` 2.2.3 + `snarkjs` Groth16 proof against `shield.circom`/`shield.zkey`, independently verified with `snarkjs groth16 verify`, moving real native-XLM SEP-41 value:

| # | Amount (stroops) | Leaf index | Tx hash |
| --- | --- | --- | --- |
| 1 | `10000000` (1 XLM) | 0 | `7969b08549258d1f4f2431d8c9655ff9a4c351614276f51b195e7f69fc20e2cb` |
| 2 | `20000000` (2 XLM) | 1 | `94d864f48ad104d8d9ed01ef10750def3261e09fa7719cd21bc7fe20f81a1dc5` |
| 3 | `30000000` (3 XLM) | 2 | `1f4f719d53b802aca2cf481ea759ae0319f442c9cde869f4dcd447e28284158c` |

Post-run state: `leaf_count() = 3`, `shielded_supply(native) = 60000000` (6 XLM). Each transaction's event log confirms the real token `transfer` event from the deployer to `ShieldedToken` for the exact shielded amount.

### Indexer live validation (same deployment, 4th shield transaction)

A fourth real `shield()` call was submitted specifically to validate the indexer service end-to-end against this same `ShieldedToken` instance:

| Amount (stroops) | Leaf index | Tx hash |
| --- | --- | --- |
| `50000000` (5 XLM) | 3 | `a82d7bd240d3bbf9b4caeb9f7c2737023e3dfafc8b55a469e3c44812a3c4c243` |

The indexer's sync loop correctly found and persisted the real `("zkella","note")` event (exact commitment/encrypted-note/leaf-index match), and every HTTP endpoint (`/health`, `/notes`, `/merkle/root`, `/merkle/path/:i`, `/commitment/:hex`, `/nullifiers/batch`) returned correct data, including `/merkle/root`/`/merkle/path`'s live proxy calls into this contract. This run also caught and fixed a real bug: the view-call simulation helper used a hand-typed placeholder address with an invalid StrKey checksum.

---

## Epoch 2 — senior-audit swap redeploy: the full commit-reveal lifecycle, real value, first time (August 3, 2026)

A fresh redeploy was required because `SwapFairness` hadn't existed as a `CircuitType` on the Epoch 1 `verifier`/`governance` instances (confirmed via `stellar contract info interface`, which showed `CircuitType` topping out at `Transfer4x4=4`):

| Contract | Address |
| --- | --- |
| verifier | `CCRLI4EAT62QVMTJR62NNJUZCERCGSYGNM534Z5R6RYSFRKELUZIG2MG` |
| governance | `CDCSHTT3R75M3BEOEDPETB3RDB4BFXI5Q2KDI2KFT3O6M73WBVUBSZWD` |
| compliance | `CA2EU46YYEBJW5C3JCRD3IAGTUD7UBPFBPYTT3I7UTESBK7FYXFCVG7Q` |
| ShieldedToken | `CDE7U6HTLMDFAEQOT5BIZ3W7VJKAQN2MFQKYVV5E3W5YIPUBSRBHAXCE` |
| swap (pre-fix, superseded below) | `CA4NYL2ZA67NSYOVPZMDA3YC62ARWYD52JA5NHYXRBP4TGSX3UNHBRPH` |
| swap (post-fix) | `CDPPRPAVKUJGNYE3AVFIBSTV7LCEOUPMM7USL7XARS2L2QRLUIMC53K3` |

A senior-auditor pass over `contracts/swap` had already fixed two issues before this deployment (an `intent_commitment`-collision fund-orphaning bug; a CEI-ordering reentrancy risk in `execute_swap`). A third was found only by this live run.

### Pre-fix attempt: the evidence for finding #3 (missing nested cross-contract auth)

Against `swap` at `CA4NYL2ZA67NSYOVPZMDA3YC62ARWYD52JA5NHYXRBP4TGSX3UNHBRPH`:

| Step | Result | Tx hash |
| --- | --- | --- |
| Shield (real proof, leaf 0) | succeeded | `99aabc85fd3b3abc7a437dd2330b6bc9b12a646e9b696a53633248891eacc117` |
| `commit_swap` (real `unshield.circom` ownership proof) | succeeded | `7c1f7fe60120902a8062b756dfe674e7148c4552cef6a0e97eb0e018a3b790f8` |
| `execute_swap` (relayer fronts liquidity) | succeeded | `b701041942470e91b91ae2c4bb276cd97a9e05e8b43410505dc0757538a0482e` |
| `reveal_and_claim` | **failed on-chain**: `HostError: Error(Auth, InvalidAction)` | *(rejected — no hash)* |

Root cause: `reveal_and_claim` calls `ShieldedToken::shield`, which itself calls `token::Client::transfer` on the underlying SEP-41 asset — a call two hops deep in the stack from `swap`'s own authorization, and Soroban only auto-authorizes one hop. Fixed with an explicit `env.authorize_as_current_contract(...)` entry. That superseded contract's escrowed funds (5,000,000 stroops `asset_in`, 4,955,000 stroops `asset_out`) were not lost — recoverable via its own `reclaim_expired_swap`.

### Post-fix: full lifecycle succeeds end-to-end, real value moving at every step

Against `swap` at `CDPPRPAVKUJGNYE3AVFIBSTV7LCEOUPMM7USL7XARS2L2QRLUIMC53K3`:

| Step | What happened | Tx hash |
| --- | --- | --- |
| 1. Shield | Real `shield.circom` proof; 5,000,000 stroops native XLM shielded (leaf 1) | `09337d4f15c659dcc45d004caa7c423b0984b501168b97482b2abc0bd4f91944` |
| 2. `commit_swap` | Real `unshield.circom` ownership proof; escrowed the 5,000,000 stroops into `swap` | `0bfb955de61f19564d793ce62dd21d12c5fc5b95b0a58bd81aa63128bbe2a971` |
| 3. `execute_swap` | Relayer (deployer, self-approved via `set_relayer`) fronted 4,955,000 stroops | `0fe6c726448e874005306bb9e8c637f958f5773205c0136f245c001336504574` |
| 4. `reveal_and_claim` | Real `swap_fairness.circom` proof verified; relayer paid 5,000,000 stroops; a second, separate real `shield.circom` proof re-shielded the output note (leaf 2) | `cc3d8a0bacfe1e70092fffba68fc81aca28d05c5aee185cea2cff337f0e60c4e` |

Every proof in this lifecycle (ownership, fairness, output-shield) is a genuine `circom`/`snarkjs` proof — this is the first time the complete commit-reveal cycle ran on-chain with real value at every step, closing the delivery-roadmap item for "a dedicated audit of the swap primitive and a live-Testnet run of its full lifecycle."

---

## Epoch 3 — Merkle root-history window: deployed, not yet an independent live epoch

The root-history-window reliability fix (`ROOT_HISTORY_SIZE = 32`) was added and regression-tested after Epoch 2, but did not get its own dedicated live-Testnet deployment before being folded into Epoch 4 below — so there is no separate transaction set for it here. Its specific behavior (accepting an anchor that's stale but still within the last 32 roots) remains verified by two Rust regression tests (`transfer_accepts_anchor_still_within_root_history_window`, `transfer_rejects_anchor_evicted_from_root_history_window`), not by a live transaction deliberately constructed to submit a stale-but-in-window anchor — see "What is not yet demonstrated live" below.

---

## Epoch 4 — external technical review redeployment: all seven findings closed live (August 14, 2026)

**This is the current live deployment** — cross-check addresses against `deployments.json` and `docs/TESTNET_DEPLOYMENT.md` before relying on anything below, since those two are the canonical current-state source and this section is a historical snapshot as of the date above.

| Contract | Address |
| --- | --- |
| verifier | `CAD7I5VEXC6QXO6A4K3PP5GLCLY6EJZ6LXLAPDR4WILBRJFINXDGQOER` |
| governance | `CCO72PR2RHEUWXWKB5D5UTHMSJOWNLNA3FELUSAFVXXGTCDGVEUQL4MS` |
| compliance | `CAA6GVANAT7GBWBA3CRXIL7WX4O62NEGBC6XHMPTFMEZPBHMM5PKRNOS` |
| token (ShieldedToken) | `CACD4IA6OJQPG3AVGPQPJT3SJKP7YQQM4BIHUD7F7NG74KDJQLGIZQOQ` |
| swap | `CBGG3UND7P6GMHCUSSYVGIOB6FUO5KK7OZVBA7LI7K4K7CJEV5T3ZRXN` |

**One deliberate, explicitly-flagged exception:** this `governance` binary was built with the `testnet-fast-timelock` Cargo feature (`contracts/governance/Cargo.toml`), shortening `VK_TIMELOCK_LEDGERS` from the real 7-day production value to ~5 minutes (60 ledgers), purely so the full `queue_vk_update` → wait → `execute_vk_update` path could be exercised live in one sitting, including a real, non-zero wait. **This feature must never be built into a production/mainnet artifact.**

### Setup

| Step | Tx hash |
| --- | --- |
| `verifier.initialize(admin=governance)` | `339199d67efccc223279173e5e8db37a0daba65ffa7bd5927ec055081c0d36b4` |
| `governance.initialize(admin=deployer, verifier)` | `3f2624e797a5d0e64de2078e4e2865e16d5e0e45807f32603eca084fa4020cda` |
| `compliance.initialize(verifier)` | `62521977b17c60b46be3d02640d5470b9fb93cf9e1235d787b678761ae898ac8` |
| `token.initialize(admin=deployer, verifier)` | `ab70903c9f0527f6df2c071b186194bfe8fdb4cd8a5a37a80b55a894a5a38d15` |
| `swap.initialize(admin=deployer, verifier, token)` | `48dc7e433762a427cf91a45bc33125892654a4c40bd01837f493c3991edc1a96` |
| `swap.set_relayer(deployer, true)` | `bc51948abfe16875502f8af6292573f2338072f3caf488ebefd8aafd6a0ef9c9` |

### Governance timelock, exercised end to end (closes the High finding)

`register_vk`'s untimelocked fast path is gone; every VK registration — including a circuit's very first key — now goes through the same 7-day-timelocked `queue_vk_update`/`execute_vk_update` path as a rotation:

| Step | Tx hash | Ledger |
| --- | --- | --- |
| `queue_vk_update(circuit=Shield)` | `cc4809befb3742c283f612cc061e9006722968e3ec8005ae8375fe1074af3201` | eta 4141735 |
| `queue_vk_update(circuit=Unshield)` | `1c6f4870fa52218c760e7581e0f71be985f3eabe7dc1a94e3f56b852b448290d` | eta 4141748 |
| `queue_vk_update(circuit=SwapFairness)` | `f253fad14be8f7c1797840c45fa07eede32321a4bb204d77311812e4bacf9c8d` | eta 4141749 |
| *(real wait for the ledger to reach each eta — no fast-forwarding on public Testnet)* | | |
| `execute_vk_update(circuit=Shield)` | `0131928d88132a34d1e79f8ad262389e5e83095fe61b281d64517a24ff990d42` | |
| `execute_vk_update(circuit=Unshield)` | `41449cbea7e8bd32a17c6cf97d18ffa320b38b58310bfae088ed5a0b26bee20f` | |
| `execute_vk_update(circuit=SwapFairness)` | `a2ca1fbf2d8b2a3d5b3a0747fbc4a85fc9457f618d3d341dfbaab78aee3ad372` | |

Each `execute_vk_update` correctly took the registration branch (no prior key existed for that circuit), not the rotation branch.

### Real shield transactions (two notes)

Each is a genuine `circom`/`snarkjs` Groth16 proof against the real compiled `shield.circom`, independently verified before submission:

| # | Amount (stroops) | Leaf index | Tx hash |
| --- | --- | --- | --- |
| A | `5000000` (0.5 XLM) | 0 | `0722df0e01bd81ee256fb317c44a97a4e713c19fe019e27460216887fb7cacee` |
| B | `5000000` (0.5 XLM) | 1 | `bbeecaeaba30517bd3a2cbc4c2f7512fd9f57b3e3b0e28b9f3bcb2998a55e945` |

### Shielded swap — full commit-reveal lifecycle, exercising both Critical proof-binding fixes live

Uses note B (leaf 1). `commit_swap`'s ownership proof was generated with the new `binding_tag = Poseidon2(intent_commitment, refund_to)` folded into `recipient_hash` (the fix for the proof-replay Critical finding), and `reveal_and_claim`'s fairness proof carries the same `intent_commitment` committed at `commit_swap` time (the fix for the previously-missing binding check):

| Step | What happened | Tx hash |
| --- | --- | --- |
| `commit_swap` | Real `unshield.circom` ownership proof, bound via `binding_tag`; escrowed 5,000,000 stroops into `swap` | `21c4380b39685c9674edabb2f2830d931e8ead0d557adcff1a4aecdf66bc8038` |
| `execute_swap` | Relayer fronted 4,950,000 stroops into escrow | `5bfef119f8503f66782f0a22a4942fa43fc83c497ae71b7031ffc0025fa9fb75` |
| `reveal_and_claim` | Real `swap_fairness.circom` proof, checked against `state.intent_commitment`; relayer paid 5,000,000 stroops; a second, separate real `shield.circom` proof re-shielded the output note as leaf 2 | `88aebe0e9cb0239d74a746facf2af18cdbe2921d1e7d9dbdaa12c6862a91648d` |

`swap_id` for this run: `64c3f9d46aa1ccb1d9ed0dc7e83a780194b67f477c53495838d5542df4e18cef`.

**Final verified on-chain state:** `leaf_count() = 3`, `merkle_root() = 7dac71b56ca54ea2f74c4694f89617c3446d3c7c5d95c280dfa07e66a0199208`, `shielded_supply(native XLM) = 9950000` — note A's 5,000,000 (still shielded, untouched) plus the swap's 4,950,000 output note; note B's 5,000,000 correctly dropped out when spent into escrow.

### `swap.initialize`'s re-init guard and `reveal_and_claim`'s overflow guard

Both fixes change what's *rejected*, not what a successful call looks like, so they have no on-chain transaction of their own here — they're covered by dedicated regression tests (`initialize_cannot_be_called_twice`, `commit_swap_rejects_expiry_ledger_that_would_overflow_the_claim_window`) rather than a live demonstration, consistent with how a rejection path doesn't need a live transaction to prove it works.

---

## Cryptographic values for independent reproduction (Epoch 4)

For a reviewer who wants to recompute these values independently rather than trust the transaction outcomes alone:

| Value | Hex |
| --- | --- |
| Note A commitment | `05574938bfd9bc403dcbd00c10fc0a103a34e7bd08cd5eee53ea3be9a5aabf17` |
| Note A `rho` | `bfbe617c8dc4b17ccf446483c98a02910d8f8aadf2e6a8cdcd1ab29b59abde00` |
| Note A `rcm` | `e778e3cd264868b954508269e7666a84a27b9f89a60eebf0c8a0721bb550b200` |
| Note B commitment | `e09868e71d25405feb4338b71e589b84bbb44ec58249944e099eb4d02b561728` |
| Note B `rho` | `902d0c522dfb86f4e68a163c0672f76de5734ac08011a17ad25e5ee444961500` |
| Note B `rcm` | `0d7d1ae73109110f434a2fae3a28b8c5c3c9bf965726d89c344ccbad90bfb300` |
| Note B nullifier (spent into `commit_swap` escrow) | `c341d22efd686dfda704b038b2c94b32c68b101586051eb19be51df995f45228` |
| `intent_commitment` (swap) | `66e3a5c3900ceba2f3076d2e43ac666c4e6c9a43bbc8527a2a85a296c04ae311` |
| `binding_tag` (`Poseidon2(intent_commitment, refund_to)`) | `c9b209182bc8971301ddc27a8953a08fe4b59df5fc04936e9f3c16f4dfda250c` |
| `recipient_hash` (`Poseidon2(swap_address_field, binding_tag)`) | `69eaaceac34c2c54383a19451b0b4493926de5ee6be31914b7c065f5b25ab320` |
| Output note commitment (leaf 2) | `9d3162af8662c3bc5bba65146b3d64fc363722e2fea7c4adc7432aee79f40a14` |
| Final `merkle_root()` | `7dac71b56ca54ea2f74c4694f89617c3446d3c7c5d95c280dfa07e66a0199208` |

Swap fairness values: `amount_in = 5000000`, `max_slippage_bps = 1000`, `min_amount_out = 4500000`, `amount_out = 4950000`.

---

## Epoch 5 — Transfer VK registration and a real, live transfer() transaction (September 2, 2026)

Same deployment as Epoch 4 (no redeployment needed). Direct response to reviewer feedback asking for the heavier transfer path to be proven live, not just measured locally.

| Step | Tx hash |
| --- | --- |
| `queue_vk_update(circuit=Transfer)` | `bd1e03efcb9f496790ae782acd36b1101869cf7367209be3357d8881917f696f` |
| `queue_vk_update(circuit=Transfer4x4)` | `3c8fb0c8bcff1a8e6fbe99be14f4b6af9cb0473280858a6be3d8999621ac5fa8` |
| *(real wait for the ledger to reach each eta)* | |
| `execute_vk_update(circuit=Transfer)` | `4bd193f369c94b66145bba90697532f95fe9da41eb06cd87030a848220526bc8` |
| `execute_vk_update(circuit=Transfer4x4)` | `320c4f2058df75acbd837b78dcaa5ede7bca1422aee708c53a9deea6bbc2ea59` |
| `shield()` — note C, 0.3 XLM, leaf 3 | `23d681296467f36021b2adca87d8f648acec821d1db34d37950a23816b28a711` |
| `shield()` — note D, 0.2 XLM, leaf 4 | `041460cf1932384a8ada14aa36801f314bcfbb1e1a27ce7582ce72c216f32f60` |
| `transfer()` — notes C+D spent, two new output notes at leaves 5–6 | `90fe4d1996815f77c7c87b06a29141e01fab293dc44d01b56364be7c7e4fcf14` |

Notes C and D were shielded fresh (rather than reusing Epoch 1's notes) specifically so their secret openings would be available to spend from — Epoch 1's notes had no persisted secret data. The `transfer()` proof was generated via the TypeScript SDK's own `generateTransferProof` (`sdk/src/prover/transfer.ts`), using Merkle paths fetched directly from the deployed contract's `merkle_path()` view function — the same SDK code path an application would use, not a CLI side-channel. Full narrative in `docs/TESTNET_DEPLOYMENT.md`'s "Update: Transfer VK registration and a real, live transfer() transaction".

`Transfer4x4`'s VK is now live-registered but a live 4-in/4-out transaction has not yet been run — see `docs/SCF_READINESS.md` for its real-WASM instruction-budget measurement (97% of the mainnet limit) standing in for it today, and for the update noting that re-measuring against current Rust toolchains puts this entrypoint marginally over budget rather than under it, a compiler-sensitivity finding, not a code change.

## What is not yet demonstrated live

Being explicit about the gap between "regression-tested" and "shown on a real transaction," consistent with the rest of this documentation:

- **Root-history window's actual stale-anchor-acceptance behavior.** Every live transaction above, in every epoch, submitted its proof anchored to the *current* root at submission time. The window mechanism (`is_known_root` accepting any of the last 32 roots, not only the newest) is verified by two Rust regression tests, not by a transaction deliberately built against a stale-but-in-window anchor.
- **SDK-level and indexer-level fixes from the external review** (`ZKELLAWallet.shield()`'s `opts.to` recipient handling, the indexer's `pagingToken`-based sync and `/notes` limit clamp) — these aren't "redeployed" the way a contract is; they ship whenever a consumer updates to the current SDK/indexer code, and haven't been separately re-demonstrated against a live two-party shield or a real high-load indexer run since the fix. Their regression tests (`tests/unit/wallet-shield-recipient.test.ts`, `tests/unit/indexer-http-limit.test.ts`) remain the evidence for those two specifically.
- **Multi-operator indexer deployment, horizontal scaling.** Never attempted; explicitly out of scope for this PoC.
- **A real (non-dev) Groth16 trusted-setup ceremony.** Every proof in every epoch above used a local, single-contributor development Powers-of-Tau/Phase-2 ceremony (`circuits/*/build/`), not a production, multi-party ceremony.
- **An external, independent security review.** The "external technical review" referenced throughout this document and `docs/POC_IMPLEMENTATION.md` was performed by the team building the protocol, adopting an external-reviewer standard of scrutiny — not by a genuinely independent third party. See `docs/POC_IMPLEMENTATION.md`'s "What remains in the delivery roadmap" for this same caveat stated directly.

## Reproducing this record

`deployments.json` at the repository root always holds the current address set. Every transaction hash above resolves at `https://stellar.expert/explorer/testnet/tx/<hash>`. `docs/RUNBOOK.md` §3 documents the operational procedure (VK rotation, timelock handling) that generated the governance transactions in Epoch 4.
