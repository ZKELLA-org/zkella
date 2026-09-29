# ZKELLA — Testnet Deployment

Network: Stellar Testnet (`Test SDF Network ; September 2015`)
Deployer account: `GD76DVHMUR5GTTOKAD54LRBUQKHSENJYLFODIGF45YOU7XXN36FXTSAW`
Native XLM Stellar Asset Contract (used as `asset_in`/`asset_out` throughout): `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC`

This document lists the current live contract addresses and every on-chain transaction run against them. See `deployments.json` at the repository root for the machine-readable current address set — addresses here are redeployed whenever a contract or circuit change requires it, so treat this file as a point-in-time record, not a permanent reference. For the full transaction history across every past deployment, including superseded ones, see `docs/POC_TESTNET_VALIDATION.md`.

## Legacy live contracts (August 14, 2026 stack)

This stack is legacy. It was built before owner-key notes, the canonical public-input check, batch transcript challenges and `revoke_previous_vk`, so it is not source-equivalent to the current code. The only stack built from current source is the "Tranche 1 live validation stack" at the end of this document (also recorded as `testnet_tranche1` in `deployments.json`).

| Contract | Address |
| --- | --- |
| verifier | `CAD7I5VEXC6QXO6A4K3PP5GLCLY6EJZ6LXLAPDR4WILBRJFINXDGQOER` |
| governance | `CCO72PR2RHEUWXWKB5D5UTHMSJOWNLNA3FELUSAFVXXGTCDGVEUQL4MS` |
| compliance | `CAA6GVANAT7GBWBA3CRXIL7WX4O62NEGBC6XHMPTFMEZPBHMM5PKRNOS` |
| token | `CACD4IA6OJQPG3AVGPQPJT3SJKP7YQQM4BIHUD7F7NG74KDJQLGIZQOQ` |
| swap | `CBGG3UND7P6GMHCUSSYVGIOB6FUO5KK7OZVBA7LI7K4K7CJEV5T3ZRXN` |

`verifier`'s admin is `governance`'s own contract address (a self-authorizing pattern: cross-contract calls from `governance` satisfy `verifier`'s `admin.require_auth()` without a separate signature). `ShieldedToken`, `governance`, and `compliance` all point at this same `verifier` instance; `swap` points at this `verifier` and `token`.

**This deployment includes all seven fixes from the external technical review** — see `docs/POC_IMPLEMENTATION.md`'s "Update: external audit" for the findings, and "Update: live redeployment closes all seven findings on real Testnet" below for what running each fix live actually looked like. This deployment does not match the current source: it predates the owner-key note format, the non-canonical-input rejection, the batch transcript change and `revoke_previous_vk`, so its verifying keys and note commitments are incompatible with the current circuits.

**One deliberate, explicitly-flagged exception: this `governance` binary was built with the `testnet-fast-timelock` feature** (`contracts/governance/Cargo.toml`), shortening `VK_TIMELOCK_LEDGERS` from the real 7-day production value to ~5 minutes (60 ledgers) — purely so the full `queue_vk_update` → wait → `execute_vk_update` path could be exercised live in one sitting, including a real, non-zero wait, rather than skipped or faked. **Never build a production/mainnet artifact with this feature enabled.** The setup transactions below show the real queue → wait → execute sequence, timestamps included.

A prior contract stack (`verifier` `CCRLI4EAT62QVMTJR62NNJUZCERCGSYGNM534Z5R6RYSFRKELUZIG2MG`, `governance` `CDCSHTT3R75M3BEOEDPETB3RDB4BFXI5Q2KDI2KFT3O6M73WBVUBSZWD`, `compliance` `CA2EU46YYEBJW5C3JCRD3IAGTUD7UBPFBPYTT3I7UTESBK7FYXFCVG7Q`, `token` `CDE7U6HTLMDFAEQOT5BIZ3W7VJKAQN2MFQKYVV5E3W5YIPUBSRBHAXCE`, `swap` `CDPPRPAVKUJGNYE3AVFIBSTV7LCEOUPMM7USL7XARS2L2QRLUIMC53K3`) is superseded — it predates all seven fixes below. An earlier `swap` instance before that, `CA4NYL2ZA67NSYOVPZMDA3YC62ARWYD52JA5NHYXRBP4TGSX3UNHBRPH`, is also superseded — see "Prior swap redeployment (nested-auth fix)" below.

## Setup transactions

| Step | Tx hash |
| --- | --- |
| `verifier.initialize(admin=governance)` | https://stellar.expert/explorer/testnet/tx/339199d67efccc223279173e5e8db37a0daba65ffa7bd5927ec055081c0d36b4 |
| `governance.initialize(admin=deployer, verifier)` | https://stellar.expert/explorer/testnet/tx/3f2624e797a5d0e64de2078e4e2865e16d5e0e45807f32603eca084fa4020cda |
| `compliance.initialize(verifier)` | https://stellar.expert/explorer/testnet/tx/62521977b17c60b46be3d02640d5470b9fb93cf9e1235d787b678761ae898ac8 |
| `token.initialize(admin=deployer, verifier)` | https://stellar.expert/explorer/testnet/tx/ab70903c9f0527f6df2c071b186194bfe8fdb4cd8a5a37a80b55a894a5a38d15 |
| `swap.initialize(admin=deployer, verifier, token)` | https://stellar.expert/explorer/testnet/tx/48dc7e433762a427cf91a45bc33125892654a4c40bd01837f493c3991edc1a96 |
| `swap.set_relayer(deployer, true)` | https://stellar.expert/explorer/testnet/tx/bc51948abfe16875502f8af6292573f2338072f3caf488ebefd8aafd6a0ef9c9 |

## Update: live redeployment closes all seven findings on real Testnet

Every fix from the external technical review — three Critical in `contracts/swap`, one High in `contracts/governance`, three lower-severity — was exercised for real on this deployment, not just re-tested locally. This section is the live evidence for `docs/POC_IMPLEMENTATION.md`'s "Update: external audit."

### The governance timelock, exercised end to end (High finding)

`register_vk`'s old untimelocked fast path is gone; every VK registration — including a circuit's very first key — now goes through the same 7-day-timelocked `queue_vk_update`/`execute_vk_update` path as a rotation. Real consequence for this session: getting any circuit live required actually queuing and waiting, not just calling one function. Using the `testnet-fast-timelock` build (~5 minutes instead of 7 days — see above) so the full path could run in one sitting:

| Step | Tx hash | Ledger |
| --- | --- | --- |
| `queue_vk_update(circuit=Shield)` | https://stellar.expert/explorer/testnet/tx/cc4809befb3742c283f612cc061e9006722968e3ec8005ae8375fe1074af3201 | eta 4141735 |
| `queue_vk_update(circuit=Unshield)` | https://stellar.expert/explorer/testnet/tx/1c6f4870fa52218c760e7581e0f71be985f3eabe7dc1a94e3f56b852b448290d | eta 4141748 |
| `queue_vk_update(circuit=SwapFairness)` | https://stellar.expert/explorer/testnet/tx/f253fad14be8f7c1797840c45fa07eede32321a4bb204d77311812e4bacf9c8d | eta 4141749 |
| *(real wait for the ledger to reach the eta above — no fast-forwarding on public Testnet)* | | |
| `execute_vk_update(circuit=Shield)` | https://stellar.expert/explorer/testnet/tx/0131928d88132a34d1e79f8ad262389e5e83095fe61b281d64517a24ff990d42 | |
| `execute_vk_update(circuit=Unshield)` | https://stellar.expert/explorer/testnet/tx/41449cbea7e8bd32a17c6cf97d18ffa320b38b58310bfae088ed5a0b26bee20f | |
| `execute_vk_update(circuit=SwapFairness)` | https://stellar.expert/explorer/testnet/tx/a2ca1fbf2d8b2a3d5b3a0747fbc4a85fc9457f618d3d341dfbaab78aee3ad372 | |

Each `execute_vk_update` correctly took the *registration* branch (the circuit had no prior key), not the rotation branch — the real regression case `execute_vk_update_performs_first_time_registration_through_the_timelock` covers locally, now also demonstrated live.

### Real shield transactions (two notes, leaves 0 and 1)

Each is a genuine `circom`/`snarkjs`-generated Groth16 proof against the real compiled `shield.circom` circuit, independently verified with `snarkjs groth16 verify` before submission, moving real native-XLM SEP-41 value into the shielded pool of the newly-registered `Shield` VK.

| # | Amount (stroops) | Leaf index | Tx hash |
| --- | --- | --- | --- |
| A | `5000000` (0.5 XLM) | 0 | https://stellar.expert/explorer/testnet/tx/0722df0e01bd81ee256fb317c44a97a4e713c19fe019e27460216887fb7cacee |
| B | `5000000` (0.5 XLM) | 1 | https://stellar.expert/explorer/testnet/tx/bbeecaeaba30517bd3a2cbc4c2f7512fd9f57b3e3b0e28b9f3bcb2998a55e945 |

### Shielded swap — full commit-reveal lifecycle, exercising the proof-replay fix and the intent_commitment binding fix live

Uses note B (leaf 1) as the input note. `commit_swap`'s ownership proof was generated with the *new* `binding_tag = Poseidon2(intent_commitment, refund_to)` folded into `recipient_hash` — the real fix for the proof-replay Critical finding — and `reveal_and_claim`'s fairness proof carries the *same* `intent_commitment` committed at `commit_swap` time, exercising the second Critical fix (the previously-missing binding check).

| Step | What happened | Tx hash |
| --- | --- | --- |
| `commit_swap` | Real `unshield.circom` ownership proof for the leaf-1 note, bound via the new `binding_tag` mechanism; cross-call into `ShieldedToken::unshield` verified it on-chain and escrowed 5,000,000 stroops into `swap`'s own balance | https://stellar.expert/explorer/testnet/tx/21c4380b39685c9674edabb2f2830d931e8ead0d557adcff1a4aecdf66bc8038 |
| `execute_swap` | Relayer (the deployer, self-approved via `set_relayer`) fronted 4,950,000 stroops into escrow | https://stellar.expert/explorer/testnet/tx/5bfef119f8503f66782f0a22a4942fa43fc83c497ae71b7031ffc0025fa9fb75 |
| `reveal_and_claim` | Real `swap_fairness.circom` proof verified on-chain, checked against `state.intent_commitment` (the fixed binding); relayer paid 5,000,000 stroops; a second, separate real `shield.circom` proof verified the new output note, re-shielded into `token` as leaf 2 | https://stellar.expert/explorer/testnet/tx/88aebe0e9cb0239d74a746facf2af18cdbe2921d1e7d9dbdaa12c6862a91648d |

`swap_id` for this run (a contract state identifier, not a transaction hash): `64c3f9d46aa1ccb1d9ed0dc7e83a780194b67f477c53495838d5542df4e18cef`.

Post-run state, confirmed via real view calls: `leaf_count() = 3`, `merkle_root() = 7dac71b56ca54ea2f74c4694f89617c3446d3c7c5d95c280dfa07e66a0199208`, `shielded_supply(native XLM) = 9950000` — exactly note A's 5,000,000 (still shielded, untouched) plus the swap's 4,950,000 output note; note B's 5,000,000 correctly dropped out of the shielded pool when it was spent into escrow.

### `swap.initialize`'s re-initialization guard and `reveal_and_claim`'s overflow guard

Both fixes are structural (they change what's rejected, not what a successful call looks like), so they don't have their own on-chain transaction here the way the findings above do — they're covered by the dedicated regression tests (`initialize_cannot_be_called_twice`, `commit_swap_rejects_expiry_ledger_that_would_overflow_the_claim_window`) rather than a live demonstration, consistent with how `docs/POC_IMPLEMENTATION.md` already frames these as verified-in-source, not requiring a live transaction to prove a rejection path works.

## Prior swap redeployment (nested-auth fix)

An earlier attempt at the swap lifecycle, against `swap` at `CA4NYL2ZA67NSYOVPZMDA3YC62ARWYD52JA5NHYXRBP4TGSX3UNHBRPH`, completed `commit_swap` and `execute_swap` successfully (tx https://stellar.expert/explorer/testnet/tx/7c1f7fe60120902a8062b756dfe674e7148c4552cef6a0e97eb0e018a3b790f8 and https://stellar.expert/explorer/testnet/tx/b701041942470e91b91ae2c4bb276cd97a9e05e8b43410505dc0757538a0482e) but failed at `reveal_and_claim` with `HostError: Error(Auth, InvalidAction)`. Root cause: a nested cross-contract authorization gap (`ShieldedToken::shield`'s own inner `token::transfer` call needed an explicit `authorize_as_current_contract` entry two call-stack levels deep, which Soroban doesn't grant automatically). The fix, and its later regression test (`reveal_and_claim_authorize_as_current_contract_satisfies_real_non_mocked_auth`, built on a testing pattern confirmed directly with an OpenZeppelin engineer), are described in `contracts/swap/src/lib.rs`'s `reveal_and_claim`.

That superseded instance's escrowed funds are not lost: they remain recoverable via that contract's own `reclaim_expired_swap`.

## Update: Transfer VK registration and a real, live transfer() transaction

Proves the heavier transfer path live, not just measured. `Transfer` and `Transfer4x4` had circuits, contract entrypoints, and local test coverage, but their verifying keys had never been registered on the live verifier and neither had ever been run as a live Testnet transaction. Both gaps are now closed for real.

### VK registration, through the real timelock

| Step | Tx hash |
| --- | --- |
| `queue_vk_update(circuit=Transfer)` | https://stellar.expert/explorer/testnet/tx/bd1e03efcb9f496790ae782acd36b1101869cf7367209be3357d8881917f696f |
| `queue_vk_update(circuit=Transfer4x4)` | https://stellar.expert/explorer/testnet/tx/3c8fb0c8bcff1a8e6fbe99be14f4b6af9cb0473280858a6be3d8999621ac5fa8 |
| *(real wait for the ledger to reach each eta — no fast-forwarding on public Testnet)* | |
| `execute_vk_update(circuit=Transfer)` | https://stellar.expert/explorer/testnet/tx/4bd193f369c94b66145bba90697532f95fe9da41eb06cd87030a848220526bc8 |
| `execute_vk_update(circuit=Transfer4x4)` | https://stellar.expert/explorer/testnet/tx/320c4f2058df75acbd837b78dcaa5ede7bca1422aee708c53a9deea6bbc2ea59 |

Confirmed live afterward via a direct `get_verifying_key` read against the deployed `verifier` contract for both circuits — no longer `VkNotRegistered`.

### Two new real shielded notes, to serve as genuine transfer inputs

The two notes already on the live tree (leaves 0–1) had no persisted secret opening to spend from, so two fresh notes were shielded — with their `rho`/`rcm` generated and retained this time — to serve as real, spendable inputs:

| # | Amount (stroops) | Leaf index | Tx hash |
| --- | --- | --- | --- |
| C | `3000000` (0.3 XLM) | 3 | https://stellar.expert/explorer/testnet/tx/23d681296467f36021b2adca87d8f648acec821d1db34d37950a23816b28a711 |
| D | `2000000` (0.2 XLM) | 4 | https://stellar.expert/explorer/testnet/tx/041460cf1932384a8ada14aa36801f314bcfbb1e1a27ce7582ce72c216f32f60 |

### A real, live `transfer()` transaction

Using the TypeScript SDK's `generateTransferProof` (`sdk/src/prover/transfer.ts`) end to end — real Merkle paths fetched directly from the deployed contract's own `merkle_path()` view function for leaves 3 and 4, the real live `merkle_root()` as anchor, real nullifier derivation, and a genuine `circom`/`snarkjs` Groth16 proof against `transfer_2in2out/transfer.circom` — notes C and D (0.5 XLM combined) were spent and re-split into two fresh output notes (0.35 XLM and 0.15 XLM):

| Step | What happened | Tx hash |
| --- | --- | --- |
| `transfer` | Real 2-in/2-out proof verified on-chain; both input nullifiers marked spent; two new output notes inserted at leaves 5 and 6 | https://stellar.expert/explorer/testnet/tx/90fe4d1996815f77c7c87b06a29141e01fab293dc44d01b56364be7c7e4fcf14 |

Post-run state, confirmed via real view calls: `leaf_count() = 7`, both spent nullifiers confirmed via `is_spent()`. This is the first live-Testnet evidence for the standalone `transfer()` entrypoint specifically (as opposed to `unshield`'s proof type, previously exercised only indirectly via the swap's `commit_swap`).

`Transfer4x4`'s VK is now also live-registered, and a live 4-in/4-out transaction has since been run on the Tranche 1 stack (see the last section: tx `a7858b390a6351d7ef8798fce58af377c16f956f98896071fb972cb02c3503cf`). The measured real-WASM cost is now 80.8M instructions (20% of the 400M limit) after the optimisation in `docs/PERFORMANCE_OPTIMISATION.md`; the live transaction on the Tranche 1 stack declared 86.3M. Before the optimisation it was 397.9M.

## Circuit trusted setup

Every verifying key and proof referenced above comes from a local, single-contributor development Powers-of-Tau/Phase-2 ceremony (`circuits/*/build/`) — not a production, multi-party ceremony. This is appropriate for testnet validation but not for a deployment handling real user funds.

## Update: Tranche 1 live validation stack (optimised build)

A fresh, isolated set of contracts built from the current source and circuits. The verifier is administered directly by the deployer, not `governance`, so this stack validates the proving and on-chain verification path, not the governance timelock (exercised on the legacy stack above). It is a validation deployment; the contracts listed at the top of this document were built before the fixes and optimisations below and are superseded by them.

What this stack contains that the legacy stack does not:
- Owner-key notes (a note can only be spent with its owner's nullifier key), canonical-input rejection in the verifier, batch-transcript challenges, `revoke_previous_vk`.
- The instruction-cost optimisations: precomputed empty-subtree roots, batched Merkle insertion, a leaner Poseidon call, and byte-level point negation in the verifier (transfer4 fell from 378.7M to 86.3M instructions on this stack; see the resource table below).
- Swap claimant binding and persistent swap state; the rewritten compliance circuit.

| Contract | Address |
| --- | --- |
| verifier | `CBHQUNPD42ZODQWCEK2SKLAARHHY75SGCVWHW6QLWLGLXWJ5JS2QORUY` |
| token | `CDDM46ZV3KLULXUGUOWSCR5BGZ6BC5XJDDMVTV4JXOLZBJXD6EQCJ75Q` |
| swap | `CB7TRLNTX6G3QNVDTHQHL46VNDQMMUUE4ZM5O6AIFFU6PWKGPKIQ7PYY` |
| compliance | `CAUZB3RTW23QQ5CT6W7KLINZDYVO56DSUZQ5AHKL56KWBH64QD5LNA3Q` |

### Shield, transfer4 and unshield

Produced by `scripts/testnet_live_validation.cjs`, each proof generated by the SDK and submitted as its own transaction. The `transfer4` and `unshield` proofs are built against Merkle paths read back from the batch-built tree, so they also confirm on-chain that batched insertion stores exactly the tree sequential insertion would.

| Step | Result | Tx |
| --- | --- | --- |
| `shield` (leaf 0) | real Groth16 verified on-chain | https://stellar.expert/explorer/testnet/tx/e51f3be33e335917b663cb1969a7d9812cbac1c5e97e797c7fb2cdcb43abd2b1 |
| `shield` (leaf 1) | | https://stellar.expert/explorer/testnet/tx/048f02332f51a0f5efe99c40e01f4b80b073d61755328630c4b28637d150b084 |
| `shield` (leaf 2) | | https://stellar.expert/explorer/testnet/tx/0ac97d14310a692a35a7a3c9da71bc84e03e96eed0dcf0c97120a52df2183700 |
| `shield` (leaf 3) | | https://stellar.expert/explorer/testnet/tx/244b995070978701a382202355d76c352d63e1b150092541da77d8c9a4910b1c |
| `transfer4` (4-in/4-out, new leaves 4-7) | real 19-signal Groth16 verified on-chain | https://stellar.expert/explorer/testnet/tx/15cbeef9533724df6ea96d3e96152255039664b11dac8640dd3d4c01370678ba |
| `unshield` | real Groth16 verified on-chain; shielded supply fell from 4,000,000 to 3,000,000 | https://stellar.expert/explorer/testnet/tx/c99b6b23dd068d3c12c77697cb614efa06c805349233716851efc85a22f80891 |

### `shield_batch` at its maximum size

Eight real shield proofs deposited in one transaction with one aggregated token transfer (`scripts/testnet_shield_batch.cjs`): https://stellar.expert/explorer/testnet/tx/22e3c4e31121a045f319e965a04761edca3d527f3dfb077423aaf0e5eac5964d (declared 335.0M instructions, 84% of the limit, 64 ledger entries, 58 written).

### Swap lifecycle

`commit_swap` takes the claimant's owner key and folds it, the output asset and the expiry into the ownership proof's binding tag; `reveal_and_claim` must use that key, so a copied proof cannot redirect the output. Produced by `scripts/testnet_swap_validation.cjs` with a real unshield ownership proof, a real swap-fairness proof and a real shield proof for the output note:

| Step | Result | Tx |
| --- | --- | --- |
| `shield` (input note) | | https://stellar.expert/explorer/testnet/tx/ff8756d3320ae98a03abf76562e3b7fb980283624ca50700e1979c939e1d527d |
| `commit_swap` | ownership proof bound to intent, claimant key, asset and expiry verified on-chain | https://stellar.expert/explorer/testnet/tx/96ac0a773395a31b36521abe81fa2ff933e6cadf0502399a467e5ec3738b1340 |
| `execute_swap` | relayer fronts `asset_out` | https://stellar.expert/explorer/testnet/tx/994f97fdf6b73dbcb2fd4bf8467a49be63c6d3dd2a5a7483aacfd6c314949797 |
| `reveal_and_claim` | swap-fairness proof and output-note shield proof verified on-chain; new note at leaf 9 | https://stellar.expert/explorer/testnet/tx/56cf20e1bed210acc1548e32514ccfc59b8f6dc31ffd788d5c609e9054321297 |

### Compliance contract (rewritten non-membership circuit, persistent records)

Produced by `scripts/testnet_compliance_validation.cjs` against a sorted sanctions tree with sentinel leaves:

| Step | Result | Tx |
| --- | --- | --- |
| Sanctioned address (its own leaf as a neighbour) | cannot build a witness, so no proof exists | (local, no transaction) |
| `publish_compliance_proof` | real non-membership proof verified on-chain; record stored in persistent storage and read back | https://stellar.expert/explorer/testnet/tx/514b9abca55beeb41d56f739f11d83ee9cb8d3a5be90f33cb0736318e3eb5385 |

### Resource profile of the live transactions

Declared Soroban resources of each transaction (from the transaction envelope, printed by `scripts/tx_resource_profile.cjs`). The instruction figure is the simulation result plus the safety margin the client adds, so it sits a few percent above the measured cost; every figure is well under the 400M limit. The four shields on a fresh deployment stayed within 80.6M to 81.2M each, consistent across all four.

| Transaction | Instructions | Before the optimisation | Ledger entries (footprint) | Written entries | Write bytes |
| --- | --- | --- | --- | --- | --- |
| `shield` #0 to #3 | 80.6M, 81.1M, 81.1M, 81.2M | 126.1M to 126.3M | 41 to 43 | 37 | 5,980 to 6,100 |
| `shield_batch` (8 items) | 335.0M | not possible (3 items was the maximum) | 64 | 58 | 9,256 |
| `transfer4` | 86.3M | 378.7M | 50 | 46 | 7,312 |
| `unshield` | 34.4M | 35.5M | 8 | 4 | 1,384 |
| swap `commit_swap` | 42.9M | 44.0M | 11 | 5 | 2,192 |
| swap `reveal_and_claim` | 112.2M | 158.5M | 47 | 39 | 7,168 |
| compliance `publish_compliance_proof` | 29.1M | 30.4M | 5 | 1 | 336 |

### Governance-settable minimum shield amount, changed live without a redeploy

Run on the previous validation token (`CDQ53BGU...`; the behaviour is unchanged in the current build): the minimum was read (1,000), raised to 2,000,000 and read back, a shield below it was rejected, and the minimum was restored. No contract was redeployed.

| Step | Tx |
| --- | --- |
| `set_min_shield_amount(2000000)` | https://stellar.expert/explorer/testnet/tx/db1b3a9cd4f6ab8aa92d6e27708bc67947643e4d48c165e4a3fe9e0336dfe890 |
| shield of 1,000,000 while the minimum was 2,000,000 | rejected by the contract during simulation (no transaction) |
| `set_min_shield_amount(1000)` (restore) | https://stellar.expert/explorer/testnet/tx/122368076cfda683116fe997de29b418f39efa309412c291686c9a69d435be25 |

`scripts/testnet_min_shield_check.cjs` performs the rejected shield.

## Tranche 2 live validation stack

A fresh stack (see `deployments.json`'s `testnet_tranche2` block), redeployed for the
interface changes Tranche 2 introduces: `token::transfer`/`transfer4` gain a `relayer:
Option<Address>` parameter and pay a positive `pub_inputs.fee` to it; `token::unshield`
gains `change_commitment`/`encrypted_change_note` parameters and a rebuilt, 7-public-input
`Unshield` circuit (`circuits/unshield/unshield.circom`); `swap::commit_swap` gains a
`min_amount_out` parameter. Verifying keys for Shield, Transfer(2x2), Unshield, Transfer4x4
and SwapFairness are registered directly (deployer-administered, same as Tranche 1's stack —
no governance timelock on this validation stack). Produced by
`scripts/testnet_tranche2_validation.cjs`.

### Deliverable 1: transfer() pays its proof-declared fee to an approved relayer

The relayer (`set_relayer`-approved beforehand), not the note owner, submits and signs the
transaction — its own Stellar account is the transaction source. Its real, on-chain balance
change nets two opposite transfers in the same transaction (it pays the real Stellar network
fee as the submitting account, and separately receives the proof-declared application-level
fee via `token::transfer`'s own internal SEP-41 transfer); isolating the latter means adding
back the real network fee actually charged (from Horizon) to the raw balance delta.

| Step | Result | Tx |
| --- | --- | --- |
| `shield` x2 (funding two input notes) | | https://stellar.expert/explorer/testnet/tx/2d6e0a5713638177161ea70490310ee4e7326d3fc78c5368f64819db3ea38f35, https://stellar.expert/explorer/testnet/tx/d939025546c6d1abdc99747e7c739493f743cc25b9f89f2991ab8a60ac65c888 |
| `transfer` (submitted by the relayer, fee = 10,000) | relayer's real balance: net -10,916,109 stroops, network fee charged 10,926,109 stroops, so the isolated application-level fee received is exactly 10,000 stroops | https://stellar.expert/explorer/testnet/tx/96733979922ee1dfe1d1f276818c5b1931550aa87630e66980508af50f787d0e |

### Deliverable 2: unshield() accepts a partial withdrawal and creates a real change note

Also the standalone, directly-invoked `unshield()` this deliverable's other criterion asks
for (not a sub-step of swap's commit flow).

| Step | Result | Tx |
| --- | --- | --- |
| `unshield` (withdraw half of a note's value) | `leaf_count` grew from 15 to 16 — the change note landed at leaf 15, a real new commitment, not a no-op | https://stellar.expert/explorer/testnet/tx/cacc35d885681328978b2841449883af363ee6f6ec805dad35ff598291abe56c |

### Deliverable 3: the stalled-swap recovery path (cancel_swap), live

A swap committed and never executed by any relayer; once its `expiry_ledger` passed, the
committer reclaimed the escrowed `asset_in` via `cancel_swap` — not exercised only in unit
tests. Same fee-isolation technique as the transfer above (the refund and `cancel_swap`'s own
real network fee net out in one balance delta).

| Step | Result | Tx |
| --- | --- | --- |
| `commit_swap` (escrowing 1,500,000, `min_amount_out = 0`) | | https://stellar.expert/explorer/testnet/tx/bccecc9269e086ad8bde4ee1931739a7771a71ca3d6aebfe8e81fb8b176b07bb |
| `cancel_swap` (after `expiry_ledger`) | net balance change +1,483,313 stroops, network fee charged 16,687 stroops → isolated refund exactly 1,500,000 stroops | https://stellar.expert/explorer/testnet/tx/8fd02a1579add5c196d843bcaf676f2d809ef4841d2d166cb8fa18e0bb51caa7 |

### Deliverable 3: execute_swap's min_amount_out bound and two concurrent swaps

Unit-tested, not (yet) run live: `execute_swap_rejects_an_amount_out_below_the_committed_minimum`,
`execute_swap_accepts_an_amount_out_at_exactly_the_committed_minimum`,
`two_swaps_can_execute_concurrently_against_the_same_relayer` and
`a_relayer_without_enough_combined_liquidity_fails_only_the_second_execute`
(`contracts/swap/src/lib.rs`) — all against the real compiled contract logic, just not
submitted as Testnet transactions. See `docs/TRANCHE2_DELIVERABLES.md` for the concurrent-swap
behavior this documents.

### Developer note: swap never touches the Stellar DEX

`contracts/swap` has no dependency on, or call into, any DEX contract (classic Stellar DEX,
Soroswap, or otherwise) anywhere in its source. Every asset movement in `commit_swap`,
`execute_swap`, `reveal_and_claim`, `cancel_swap` and `reclaim_expired_swap` is a direct SEP-41
`transfer` between the swap contract's own balance, the committer's shielded note (via
`token::unshield`/`token::shield`), and the relayer's account — the relayer supplies
`asset_out` liquidity directly from its own balance (a real SEP-41 transfer at `execute_swap`
time) and is compensated with the escrowed `asset_in` at `reveal_and_claim`. There is no
on-chain price discovery and no order book; the executed price is whatever the relayer offers,
constrained only by `commit_swap`'s `min_amount_out` floor (Tranche 2) and, at reveal,
`swap_fairness.circom`'s proof that the revealed price matches what was actually committed to.
