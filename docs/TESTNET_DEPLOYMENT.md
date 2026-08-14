# ZKELLA — Testnet Deployment

Network: Stellar Testnet (`Test SDF Network ; September 2015`)
Deployer account: `GD76DVHMUR5GTTOKAD54LRBUQKHSENJYLFODIGF45YOU7XXN36FXTSAW`
Native XLM Stellar Asset Contract (used as `asset_in`/`asset_out` throughout): `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC`

This document lists the current live contract addresses and every on-chain transaction run against them. See `deployments.json` at the repository root for the machine-readable current address set — addresses here are redeployed whenever a contract or circuit change requires it, so treat this file as a point-in-time record, not a permanent reference.

## Current live contracts (as of August 3, 2026)

| Contract | Address |
| --- | --- |
| verifier | `CCRLI4EAT62QVMTJR62NNJUZCERCGSYGNM534Z5R6RYSFRKELUZIG2MG` |
| governance | `CDCSHTT3R75M3BEOEDPETB3RDB4BFXI5Q2KDI2KFT3O6M73WBVUBSZWD` |
| compliance | `CA2EU46YYEBJW5C3JCRD3IAGTUD7UBPFBPYTT3I7UTESBK7FYXFCVG7Q` |
| token | `CDE7U6HTLMDFAEQOT5BIZ3W7VJKAQN2MFQKYVV5E3W5YIPUBSRBHAXCE` |
| swap | `CDPPRPAVKUJGNYE3AVFIBSTV7LCEOUPMM7USL7XARS2L2QRLUIMC53K3` |

`verifier`'s admin is `governance`'s own contract address (a self-authorizing pattern: cross-contract calls from `governance` satisfy `verifier`'s `admin.require_auth()` without a separate signature). `ShieldedToken`, `governance`, and `compliance` all point at this same `verifier` instance; `swap` points at this `verifier` and `token`.

**Known drift as of this writing: the contracts above predate several real fixes that exist in source only.** Nothing below this line is affected by any of them — every transaction on this page happened before these source changes — but don't assume any deployed contract here has a fix just because the source repository does:

- `token`'s Merkle root-history tolerance window (`contracts/token/src/merkle.rs`, see `docs/POC_IMPLEMENTATION.md`'s "Update: Merkle root-history window") — this instance still enforces exact-root-equality on `transfer()`/`unshield()`, not the widened 32-root window.
- `token`'s `unshield()` new `binding_tag` parameter, and everything that depends on it — `swap`'s `commit_swap` proof-replay fix, and `unshield()`'s updated signature generally (see `docs/POC_IMPLEMENTATION.md`'s "Update: external audit" for the full account). The deployed `token`/`swap` pair still has the exploitable version.
- `swap`'s `initialize` re-initialization guard, `reveal_and_claim`'s `intent_commitment` binding check, and `reclaim_expired_swap`'s overflow guard — none of these exist in the deployed `swap` instance above.
- `governance`'s `register_vk` removal (first-time VK registration is no longer a fast, untimelocked path) — the deployed `governance` instance above still has the old, faster (and less safe) behavior. This is directly relevant here: this page's own setup transactions used the now-removed `register_vk` for `Shield`/`Unshield`/`SwapFairness` — an accurate historical record of what was actually called, not something to retroactively rewrite, but a future re-registration (e.g. for `Transfer`/`Transfer4x4`, still unregistered as of this writing) against a redeployed `governance` would need to go through `queue_vk_update`/`execute_vk_update` instead.

Redeploying the full contract stack (and updating `deployments.json` and this file accordingly) remains open before any of this can be described as live-validated rather than locally-validated.

A prior `swap` instance, `CA4NYL2ZA67NSYOVPZMDA3YC62ARWYD52JA5NHYXRBP4TGSX3UNHBRPH`, is superseded — see "Swap redeployment" below.

## Setup transactions

| Step | Tx hash |
| --- | --- |
| `verifier.initialize(admin=governance)` | `5c3f32910b15676cc4c3c7bdd2f026618d0842dbaf9774b935a59dd1d3bc1f32` |
| `governance.initialize(admin=deployer, verifier)` | `2fc9f667dbcb9ec48f4789938ddc768762951dc1b29fd7238461e20b0b81f06c` |
| `compliance.initialize(verifier)` | `5430a9c94e73d46f58712e0bb7fef790943adce71b7ddeeb26c915a7eaa9b956` |
| `token.initialize(admin=deployer, verifier)` | `f6148816cff77b1faa9ecd36b9839debd79e5aa32a43fe7683fc5bd12fb4f8b8` |
| `swap.initialize(admin=deployer, verifier, token)` | `336ee8b43a57b8e40051c0b44223230792176903a0ce5ce7c9ef1f4445097a30` |
| `governance.register_vk(circuit=Shield)` — real `shield.circom` VK | `bf0481057d685acfc591df3c6d3ee6205b8204d11f802d22e4b917732422f2ea` |
| `governance.register_vk(circuit=Unshield)` — real `unshield.circom` VK | `ef453931b6e7a7d43ad6e313fcb990e0baa2ef9b83252ea93e1cd50d5dea2680` |
| `governance.register_vk(circuit=SwapFairness)` — real `swap_fairness.circom` VK | `dfa30dd42a07a3f3802b353023bac9e76f307b335eabff88c04d1ff7115955cb` |
| `swap.set_relayer(deployer, true)` | `2af826e06e4fb1e284c4dc958ec9baf88975d579e62c575aecbd184d04792c0b` |

(`https://stellar.expert/explorer/testnet/tx/<hash>` for each.)

## Real shield transactions

Each is a genuine `circom`/`snarkjs`-generated Groth16 proof against the real compiled `shield.circom` circuit, independently verified with `snarkjs groth16 verify` before submission, moving real native-XLM SEP-41 value into the shielded pool.

| # | Amount (stroops) | Leaf index | Tx hash |
| --- | --- | --- | --- |
| 1 | `5000000` (0.5 XLM) | 0 | `99aabc85fd3b3abc7a437dd2330b6bc9b12a646e9b696a53633248891eacc117` |
| 2 | `5000000` (0.5 XLM) | 1 | `09337d4f15c659dcc45d004caa7c423b0984b501168b97482b2abc0bd4f91944` |

Post-run state confirmed via `merkle_root()`/`leaf_count()`/`shielded_supply()` reads after each call.

## Shielded swap — full commit-reveal lifecycle, live

The complete real value-moving lifecycle of `contracts/swap` — `commit_swap` → `execute_swap` → `reveal_and_claim` — run twice: once against the swap contract as it existed before a fix (which failed at the last step, described below), and once against the fixed, currently-live contract (which succeeded end to end).

### Live run (current `swap` contract, succeeded end to end)

| Step | What happened | Tx hash |
| --- | --- | --- |
| `commit_swap` | Real `unshield.circom` ownership proof for the leaf-1 note above; cross-call into `ShieldedToken::unshield` verified it on-chain and escrowed 5,000,000 stroops into `swap`'s own balance | `0bfb955de61f19564d793ce62dd21d12c5fc5b95b0a58bd81aa63128bbe2a971` |
| `execute_swap` | Relayer (the deployer, self-approved via `set_relayer`) fronted 4,955,000 stroops into escrow | `0fe6c726448e874005306bb9e8c637f958f5773205c0136f245c001336504574` |
| `reveal_and_claim` | Real `swap_fairness.circom` proof verified on-chain (binding the revealed `amount_out`/`min_amount_out` back to the `intent_commitment` from `commit_swap`, without either having been revealed at commit time); relayer paid 5,000,000 stroops; a second, separate real `shield.circom` proof verified the new output note, re-shielded into `token` as leaf 2 | `cc3d8a0bacfe1e70092fffba68fc81aca28d05c5aee185cea2cff337f0e60c4e` |

`swap_id` for this run: `4a850d42c2d42171884f836e9576c40d218c39f57fa0f073cff356fc10f099c3`.

### Swap redeployment

An earlier attempt at this same lifecycle, against `swap` at `CA4NYL2ZA67NSYOVPZMDA3YC62ARWYD52JA5NHYXRBP4TGSX3UNHBRPH`, completed `commit_swap` and `execute_swap` successfully (tx `7c1f7fe60120902a8062b756dfe674e7148c4552cef6a0e97eb0e018a3b790f8` and `b701041942470e91b91ae2c4bb276cd97a9e05e8b43410505dc0757538a0482e`) but failed at `reveal_and_claim` with `HostError: Error(Auth, InvalidAction)`. Root cause and fix are described in `contracts/swap/src/lib.rs`'s `reveal_and_claim` (a nested cross-contract authorization gap: `ShieldedToken::shield`'s own inner `token::transfer` call needed an explicit `authorize_as_current_contract` entry two call-stack levels deep, which Soroban doesn't grant automatically). The fix was applied, the workspace test suite re-verified, and `swap` was redeployed to its current address, where the full lifecycle above completed successfully.

That superseded instance's escrowed funds (5,000,000 stroops of `asset_in`, 4,955,000 stroops of `asset_out` — both the deployer's own testnet funds, since the deployer is both shielder and relayer in this demonstration) are not lost: they become recoverable via that contract's own `reclaim_expired_swap` once its claim window passes.

## Circuit trusted setup

Every verifying key and proof referenced above comes from a local, single-contributor development Powers-of-Tau/Phase-2 ceremony (`circuits/*/build/`) — not a production, multi-party ceremony. This is appropriate for testnet validation but not for a deployment handling real user funds.
