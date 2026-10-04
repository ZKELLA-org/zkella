# Governance: pause, guardian, and deferred features

This document describes how the governance contract (`contracts/governance`) and the pause mechanism added across the contracts control protocol changes. It also lists what is deliberately not included.

## Roles

- **Admin.** Queues and executes verifying-key updates, pauses and unpauses governance, and transfers admin (two-step: propose, then accept from the new key).
- **Guardian.** Set once at `initialize`. Can cancel a queued verifying-key update through `guardian_cancel_vk_update`, and nothing else. A guardian cannot queue, execute, pause, or transfer admin.

## Timelock

Every verifying-key change goes through `queue_vk_update`, then `execute_vk_update` after `timelock_ledgers()` ledgers (7 days at 5 seconds per ledger). This applies to a circuit's first key as well as to rotations. Revoking a retained previous key is not timelocked, because it only narrows what the verifier accepts.

## Pause

Each of the four contracts (verifier, governance, compliance, swap) has `pause` and `unpause`, restricted to its admin. The flag is stored in instance storage. When the flag is set, the gated entrypoints below reject the call with an error or a `paused` panic. Reads are never gated.

| Contract | Gated while paused | Callable while paused, and why |
| --- | --- | --- |
| verifier | `register_verifying_key`, `update_verifying_key` | `revoke_previous_vk`: narrows what `verify` accepts, and is the response a pause is meant to enable. |
| governance | `queue_vk_update`, `execute_vk_update`, `transfer_admin`, `accept_admin` | `cancel_vk_update` and `guardian_cancel_vk_update`: cancelling a bad queued update is the response a pause is often declared for. `revoke_previous_vk`: narrowing, as above. |
| compliance | `publish_compliance_proof` | Reads. |
| swap | `commit_swap`, `execute_swap`, `reveal_and_claim`, `set_relayer` | `cancel_swap`, `reclaim_expired_swap`: these return escrowed funds to users, and blocking them would trap user funds. |

### What a pause does not stop

- **Proof acceptance in the verifier.** `verify` and `verify_batch` are not gated. The verifier is a shared registry, and token, swap, and compliance call it directly. To stop proof acceptance in an emergency, pause the contracts that consume proofs (token, swap, compliance). The verifier's own pause only stops key changes.
- **Token.** The token contract's pause is separate and was added in an earlier tranche. It gates `shield`, `shield_batch`, `transfer`, `transfer4`, and `unshield`.

### Deviation from the roadmap's count

The roadmap lists 18 gated entrypoints (verifier 3, governance 6, compliance 2, swap 7). Under one consistent rule (exclude the one-shot `initialize`, count only functions that write state), the count is verifier 3, governance 6, compliance 1, swap 6. Of those, this implementation gates 3, 4, 1, and 4 respectively. The remaining ones are the cancel and revoke paths listed above, which must stay callable during a pause. Compliance has one state-changing entrypoint, not two. The roadmap's total of 18 counts `initialize` for compliance and swap, but not for verifier and governance; this document applies the rule consistently.

## Governance-settable parameters

`MIN_SHIELD_AMOUNT` is a storage value in the token contract, changed by `set_min_shield_amount`, which only the token admin can call. Making it governance-settable requires the token admin to be the governance contract, and a timelocked governance entrypoint that forwards the change. That path is not yet implemented. See the open items in `docs/SECURITY_TOOLING_REPORT.md`.

## Deferred to a later governance upgrade

These are out of scope for this deliverable, as the roadmap states:

- Admin multisig. Moving admin control to a multisig account is a deployment decision for mainnet.
- Circuit upgrade authorization beyond verifying-key rotation.
- Fine-grained parameter adjustment beyond the items named above.

## Tests

Each gated entrypoint has a test that the pause blocks it, and each exempt entrypoint has a test that the pause does not block it. Unpause is tested to restore access in the verifier and compliance contracts for their gated entrypoints, and for one gated entrypoint each in governance (`queue_vk_update`) and swap (`set_relayer`). Restoring every gated entrypoint after unpause in governance and swap is not yet tested. Run the tests with `cd contracts && cargo test --workspace --release`.
