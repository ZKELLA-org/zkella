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
