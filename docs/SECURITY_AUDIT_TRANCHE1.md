# Tranche 1 security audit: findings and resolution

An internal review of the verifier and token contracts, the circuits, and the SDK and scripts, run as three independent read-only audits. Every finding below was checked against the code before it was acted on.

## Fixed

| Severity | Finding | Resolution |
| --- | --- | --- |
| Critical | The nullifier key `nk` was a free private input in the spend circuits, so one note could be spent repeatedly under fresh nullifiers, and anyone who knew a note's contents could spend it. | Notes commit to an owner key `pk = Poseidon2(nk, "zkella_pk")`; unshield, transfer and transfer4 derive `pk` from `nk` and require the spent note to contain it. Regression tests in `tests/unit/circuit-owner-binding.test.ts`. |
| Critical | The verifier reduced public inputs modulo the field order, so `x` and `x + r` verified identically and a nullifier could be aliased to spend a note again. Confirmed with a test before fixing. | The verifier rejects any public input at or above the modulus (`NonCanonicalInput`). Asset fields are reduced before use as public inputs. |
| High | `verify_batch` derived its challenges from each proof alone, so public inputs could be chosen after the challenges were known and invalid items could cancel. | Challenges hash the circuit and every item's public inputs and proof. |
| Medium | A rotated-out verifying key stayed acceptable for the retention window with no way to revoke it. | `revoke_previous_vk`; saturating expiry arithmetic. |
| Low | Negative transfer fee not rejected at the contract boundary; fee and swap remainder lacked in-circuit range checks. | Contract check added; `Num2Bits` range checks added in the circuits. |
| Low | `shield_batch` was unbounded, but 3 items already use 347M of the 400M instruction limit. | `MAX_SHIELD_BATCH = 3`, with a real-WASM cost test. |

## Accepted and documented

- **`initialize` can be front-run** between deploy and init (medium). Initialize in the same step as deploying, and confirm the admin afterwards. Converting the contracts to constructors would remove the window.
- **Merkle sibling TTL.** Sibling nodes read during an insert are not TTL-bumped; after a long idle period an insert can fail until the entry is restored. Restoring is permissionless, so this is a liveness risk only. Bumping siblings would push `transfer4` (about 99% of the instruction limit) over budget.
- **Nullifier uniqueness relies on sender-chosen `rho`.** A sender who reuses a `rho` can create a note that cannot be spent. Deriving `rho` in-circuit (as Zcash does) is the long-term fix.
- **Trusted setup.** All circuits use development keys from a single local contribution. A real multi-party ceremony is required before mainnet.
- **Centralisation.** The pause switch also blocks withdrawals, and whoever administers the verifier can replace a verifying key.

## Second review

A second re-audit of the same code found further issues. Fixed items are described first, then the risks that were accepted.

### Fixed

- **Compliance circuit (High).** The sanctions non-membership circuit used non-strict bounds, had no adjacency check between the two bracketing leaves, and compared 64 bits of a 254-bit value. A sanctioned address could therefore produce a proof of non-membership, and honest addresses could fail to. The circuit was rewritten: strict `lower < address < upper`, adjacency enforced as `upper_idx = lower_idx + 1`, values truncated to 248 bits with `Num2Bits_strict`, and sentinel leaves `0` and `2^248 - 1`. It was rebuilt (17,533 constraints, public inputs `sanctions_root`, `tk_commitment`) and has 8 new circuit tests in `tests/unit/circuit-compliance.test.ts`. Its verifying key has not been registered on a live stack.
- **Swap claimant front-running.** A third party could front-run `reveal_and_claim` and direct the output note to themselves. The claimant's owner key (`out_owner_pk`) is now committed at `commit_swap`, `reveal_and_claim` must use it, and `asset_out` and the expiry are bound into the ownership proof's binding tag. Swap state moved from instance storage to persistent storage with TTL bumps.
- **Governance.** Gains a `revoke_previous_vk` entrypoint and a timelock getter.
- **Wallet and indexer.** The wallet dedupes notes by commitment and by nullifier and strictly validates hex and field encoding of recipient keys. Indexer paging no longer skips events at ledger and page boundaries.

### Accepted

- **`reclaim_expired_swap` pays two transfers in one call.** A committer who chooses an unpayable `refund_to` can strand an asset fronted by a relayer. Relayers are admin-approved, which limits exposure.
- **Faerie-gold `rho` reuse.** A sender can reuse a `rho`; the only notes harmed are ones that sender created.
- **`unshield`'s `recipient_hash` has no R1CS constraint of its own.** Its binding relies on the Groth16 public-input (`IC`) term, which was checked to be non-zero in the built verifying key.
- **`value_commit` is a Poseidon hash, not homomorphic.** Balance is enforced inside the circuit, not by a homomorphic check.
- **Swap `min_amount_out` and `amount_out` are not range-bound in the circuit.** The contract passes `u128` values, so this is not exploitable through the contract.
- **Transfer fee is proven but not collected.** The fee is constrained in the circuit; no contract code pays it to anyone.
