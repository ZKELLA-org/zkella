# ZKELLA — Testnet Deployment

Network: Stellar Testnet (`Test SDF Network ; September 2015`)
Deployer account: `GD76DVHMUR5GTTOKAD54LRBUQKHSENJYLFODIGF45YOU7XXN36FXTSAW`
Native XLM Stellar Asset Contract (used as `asset_in`/`asset_out` throughout): `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC`

This document lists the current live contract addresses and every on-chain transaction run against them. See `deployments.json` at the repository root for the machine-readable current address set — addresses here are redeployed whenever a contract or circuit change requires it, so treat this file as a point-in-time record, not a permanent reference. For the full transaction history across every past deployment, including superseded ones, see `docs/POC_TESTNET_VALIDATION.md`.

## Current live contracts (as of August 14, 2026)

| Contract | Address |
| --- | --- |
| verifier | `CAD7I5VEXC6QXO6A4K3PP5GLCLY6EJZ6LXLAPDR4WILBRJFINXDGQOER` |
| governance | `CCO72PR2RHEUWXWKB5D5UTHMSJOWNLNA3FELUSAFVXXGTCDGVEUQL4MS` |
| compliance | `CAA6GVANAT7GBWBA3CRXIL7WX4O62NEGBC6XHMPTFMEZPBHMM5PKRNOS` |
| token | `CACD4IA6OJQPG3AVGPQPJT3SJKP7YQQM4BIHUD7F7NG74KDJQLGIZQOQ` |
| swap | `CBGG3UND7P6GMHCUSSYVGIOB6FUO5KK7OZVBA7LI7K4K7CJEV5T3ZRXN` |

`verifier`'s admin is `governance`'s own contract address (a self-authorizing pattern: cross-contract calls from `governance` satisfy `verifier`'s `admin.require_auth()` without a separate signature). `ShieldedToken`, `governance`, and `compliance` all point at this same `verifier` instance; `swap` points at this `verifier` and `token`.

**This deployment includes all seven fixes from the external technical review** — see `docs/POC_IMPLEMENTATION.md`'s "Update: external audit" for the findings, and "Update: live redeployment closes all seven findings on real Testnet" below for what running each fix live actually looked like. No known drift between this deployment and the current source as of this writing.

**One deliberate, explicitly-flagged exception: this `governance` binary was built with the `testnet-fast-timelock` feature** (`contracts/governance/Cargo.toml`), shortening `VK_TIMELOCK_LEDGERS` from the real 7-day production value to ~5 minutes (60 ledgers) — purely so the full `queue_vk_update` → wait → `execute_vk_update` path could be exercised live in one sitting, including a real, non-zero wait, rather than skipped or faked. **Never build a production/mainnet artifact with this feature enabled.** The setup transactions below show the real queue → wait → execute sequence, timestamps included.

A prior contract stack (`verifier` `CCRLI4EAT62QVMTJR62NNJUZCERCGSYGNM534Z5R6RYSFRKELUZIG2MG`, `governance` `CDCSHTT3R75M3BEOEDPETB3RDB4BFXI5Q2KDI2KFT3O6M73WBVUBSZWD`, `compliance` `CA2EU46YYEBJW5C3JCRD3IAGTUD7UBPFBPYTT3I7UTESBK7FYXFCVG7Q`, `token` `CDE7U6HTLMDFAEQOT5BIZ3W7VJKAQN2MFQKYVV5E3W5YIPUBSRBHAXCE`, `swap` `CDPPRPAVKUJGNYE3AVFIBSTV7LCEOUPMM7USL7XARS2L2QRLUIMC53K3`) is superseded — it predates all seven fixes below. An earlier `swap` instance before that, `CA4NYL2ZA67NSYOVPZMDA3YC62ARWYD52JA5NHYXRBP4TGSX3UNHBRPH`, is also superseded — see "Prior swap redeployment (nested-auth fix)" below.

## Setup transactions

| Step | Tx hash |
| --- | --- |
| `verifier.initialize(admin=governance)` | `339199d67efccc223279173e5e8db37a0daba65ffa7bd5927ec055081c0d36b4` |
| `governance.initialize(admin=deployer, verifier)` | `3f2624e797a5d0e64de2078e4e2865e16d5e0e45807f32603eca084fa4020cda` |
| `compliance.initialize(verifier)` | `62521977b17c60b46be3d02640d5470b9fb93cf9e1235d787b678761ae898ac8` |
| `token.initialize(admin=deployer, verifier)` | `ab70903c9f0527f6df2c071b186194bfe8fdb4cd8a5a37a80b55a894a5a38d15` |
| `swap.initialize(admin=deployer, verifier, token)` | `48dc7e433762a427cf91a45bc33125892654a4c40bd01837f493c3991edc1a96` |
| `swap.set_relayer(deployer, true)` | `bc51948abfe16875502f8af6292573f2338072f3caf488ebefd8aafd6a0ef9c9` |

(`https://stellar.expert/explorer/testnet/tx/<hash>` for each.)

## Update: live redeployment closes all seven findings on real Testnet

Every fix from the external technical review — three Critical in `contracts/swap`, one High in `contracts/governance`, three lower-severity — was exercised for real on this deployment, not just re-tested locally. This section is the live evidence for `docs/POC_IMPLEMENTATION.md`'s "Update: external audit."

### The governance timelock, exercised end to end (High finding)

`register_vk`'s old untimelocked fast path is gone; every VK registration — including a circuit's very first key — now goes through the same 7-day-timelocked `queue_vk_update`/`execute_vk_update` path as a rotation. Real consequence for this session: getting any circuit live required actually queuing and waiting, not just calling one function. Using the `testnet-fast-timelock` build (~5 minutes instead of 7 days — see above) so the full path could run in one sitting:

| Step | Tx hash | Ledger |
| --- | --- | --- |
| `queue_vk_update(circuit=Shield)` | `cc4809befb3742c283f612cc061e9006722968e3ec8005ae8375fe1074af3201` | eta 4141735 |
| `queue_vk_update(circuit=Unshield)` | `1c6f4870fa52218c760e7581e0f71be985f3eabe7dc1a94e3f56b852b448290d` | eta 4141748 |
| `queue_vk_update(circuit=SwapFairness)` | `f253fad14be8f7c1797840c45fa07eede32321a4bb204d77311812e4bacf9c8d` | eta 4141749 |
| *(real wait for the ledger to reach the eta above — no fast-forwarding on public Testnet)* | | |
| `execute_vk_update(circuit=Shield)` | `0131928d88132a34d1e79f8ad262389e5e83095fe61b281d64517a24ff990d42` | |
| `execute_vk_update(circuit=Unshield)` | `41449cbea7e8bd32a17c6cf97d18ffa320b38b58310bfae088ed5a0b26bee20f` | |
| `execute_vk_update(circuit=SwapFairness)` | `a2ca1fbf2d8b2a3d5b3a0747fbc4a85fc9457f618d3d341dfbaab78aee3ad372` | |

Each `execute_vk_update` correctly took the *registration* branch (the circuit had no prior key), not the rotation branch — the real regression case `execute_vk_update_performs_first_time_registration_through_the_timelock` covers locally, now also demonstrated live.

### Real shield transactions (two notes, leaves 0 and 1)

Each is a genuine `circom`/`snarkjs`-generated Groth16 proof against the real compiled `shield.circom` circuit, independently verified with `snarkjs groth16 verify` before submission, moving real native-XLM SEP-41 value into the shielded pool of the newly-registered `Shield` VK.

| # | Amount (stroops) | Leaf index | Tx hash |
| --- | --- | --- | --- |
| A | `5000000` (0.5 XLM) | 0 | `0722df0e01bd81ee256fb317c44a97a4e713c19fe019e27460216887fb7cacee` |
| B | `5000000` (0.5 XLM) | 1 | `bbeecaeaba30517bd3a2cbc4c2f7512fd9f57b3e3b0e28b9f3bcb2998a55e945` |

### Shielded swap — full commit-reveal lifecycle, exercising the proof-replay fix and the intent_commitment binding fix live

Uses note B (leaf 1) as the input note. `commit_swap`'s ownership proof was generated with the *new* `binding_tag = Poseidon2(intent_commitment, refund_to)` folded into `recipient_hash` — the real fix for the proof-replay Critical finding — and `reveal_and_claim`'s fairness proof carries the *same* `intent_commitment` committed at `commit_swap` time, exercising the second Critical fix (the previously-missing binding check).

| Step | What happened | Tx hash |
| --- | --- | --- |
| `commit_swap` | Real `unshield.circom` ownership proof for the leaf-1 note, bound via the new `binding_tag` mechanism; cross-call into `ShieldedToken::unshield` verified it on-chain and escrowed 5,000,000 stroops into `swap`'s own balance | `21c4380b39685c9674edabb2f2830d931e8ead0d557adcff1a4aecdf66bc8038` |
| `execute_swap` | Relayer (the deployer, self-approved via `set_relayer`) fronted 4,950,000 stroops into escrow | `5bfef119f8503f66782f0a22a4942fa43fc83c497ae71b7031ffc0025fa9fb75` |
| `reveal_and_claim` | Real `swap_fairness.circom` proof verified on-chain, checked against `state.intent_commitment` (the fixed binding); relayer paid 5,000,000 stroops; a second, separate real `shield.circom` proof verified the new output note, re-shielded into `token` as leaf 2 | `88aebe0e9cb0239d74a746facf2af18cdbe2921d1e7d9dbdaa12c6862a91648d` |

`swap_id` for this run: `64c3f9d46aa1ccb1d9ed0dc7e83a780194b67f477c53495838d5542df4e18cef`.

Post-run state, confirmed via real view calls: `leaf_count() = 3`, `merkle_root() = 7dac71b56ca54ea2f74c4694f89617c3446d3c7c5d95c280dfa07e66a0199208`, `shielded_supply(native XLM) = 9950000` — exactly note A's 5,000,000 (still shielded, untouched) plus the swap's 4,950,000 output note; note B's 5,000,000 correctly dropped out of the shielded pool when it was spent into escrow.

### `swap.initialize`'s re-initialization guard and `reveal_and_claim`'s overflow guard

Both fixes are structural (they change what's rejected, not what a successful call looks like), so they don't have their own on-chain transaction here the way the findings above do — they're covered by the dedicated regression tests (`initialize_cannot_be_called_twice`, `commit_swap_rejects_expiry_ledger_that_would_overflow_the_claim_window`) rather than a live demonstration, consistent with how `docs/POC_IMPLEMENTATION.md` already frames these as verified-in-source, not requiring a live transaction to prove a rejection path works.

## Prior swap redeployment (nested-auth fix)

An earlier attempt at the swap lifecycle, against `swap` at `CA4NYL2ZA67NSYOVPZMDA3YC62ARWYD52JA5NHYXRBP4TGSX3UNHBRPH`, completed `commit_swap` and `execute_swap` successfully (tx `7c1f7fe60120902a8062b756dfe674e7148c4552cef6a0e97eb0e018a3b790f8` and `b701041942470e91b91ae2c4bb276cd97a9e05e8b43410505dc0757538a0482e`) but failed at `reveal_and_claim` with `HostError: Error(Auth, InvalidAction)`. Root cause: a nested cross-contract authorization gap (`ShieldedToken::shield`'s own inner `token::transfer` call needed an explicit `authorize_as_current_contract` entry two call-stack levels deep, which Soroban doesn't grant automatically). The fix, and its later regression test (`reveal_and_claim_authorize_as_current_contract_satisfies_real_non_mocked_auth`, built on a testing pattern confirmed directly with an OpenZeppelin engineer), are described in `contracts/swap/src/lib.rs`'s `reveal_and_claim`.

That superseded instance's escrowed funds are not lost: they remain recoverable via that contract's own `reclaim_expired_swap`.

## Circuit trusted setup

Every verifying key and proof referenced above comes from a local, single-contributor development Powers-of-Tau/Phase-2 ceremony (`circuits/*/build/`) — not a production, multi-party ceremony. This is appropriate for testnet validation but not for a deployment handling real user funds.
