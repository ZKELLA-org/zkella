# Governance: pause, guardian, and deferred features

This document describes how the governance contract (`contracts/governance`) and the pause mechanism added across the contracts control protocol changes. It also lists what is deliberately not included.

## Roles

- **Admin.** Queues and executes verifying-key updates, pauses and unpauses governance, and transfers admin (two-step: propose, then accept from the new key).
- **Guardian.** Set once at `initialize`. Can cancel a queued verifying-key update (`guardian_cancel_vk_update`) or a queued token-admin action (`guardian_cancel_token_action`), and nothing else. A guardian cannot queue, execute, pause, or transfer admin.

## Timelock

Every verifying-key change goes through `queue_vk_update`, then `execute_vk_update` after `timelock_ledgers()` ledgers (7 days at 5 seconds per ledger). This applies to a circuit's first key as well as to rotations. Revoking a retained previous key is not timelocked, because it only narrows what the verifier accepts.

## Pause

Each of the four contracts (verifier, governance, compliance, swap) has `pause` and `unpause`, restricted to its admin. The flag is stored in instance storage. When the flag is set, the gated entrypoints below reject the call with an error or a `paused` panic. Reads are never gated.

| Contract | Gated while paused | Callable while paused, and why |
| --- | --- | --- |
| verifier | `register_verifying_key`, `update_verifying_key`, `verify`, `verify_batch` | `revoke_previous_vk`: narrows what `verify` accepts, and is the response a pause is meant to enable. |
| governance | `queue_vk_update`, `execute_vk_update`, `transfer_admin`, `accept_admin` | `cancel_vk_update` and `guardian_cancel_vk_update`: cancelling a bad queued update is the response a pause is often declared for. `revoke_previous_vk`: narrowing, as above. |
| compliance | `publish_compliance_proof` | Reads. |
| swap | `commit_swap`, `execute_swap`, `reveal_and_claim`, `set_relayer` | `cancel_swap`, `reclaim_expired_swap`: these return escrowed funds to users, and blocking them would trap user funds. |

### What a pause does not stop

- **Proof acceptance is stopped by the verifier's pause, fail-closed.** `verify` and `verify_batch` return `Paused` while the verifier is paused. Token, swap, and compliance call the verifier directly, so their proof-consuming calls abort too. Reads (`get_verifying_key`) stay available.
- **Each token-side contract keeps its own flag.** Token's pause (`shield`, `shield_batch`, `transfer`, `transfer4`, `unshield`) and swap's and compliance's pauses are separate from the verifier's. Pausing the verifier alone is enough to stop proof acceptance; pausing the others stops state changes in those contracts even when the verifier is live.
- **Token.** The token contract's pause is separate and was added in an earlier tranche. It gates `shield`, `shield_batch`, `transfer`, `transfer4`, and `unshield`.

### Deviation from the roadmap's count

The roadmap lists 18 gated entrypoints (verifier 3, governance 6, compliance 2, swap 7). Under one consistent rule (exclude the one-shot `initialize`, count only functions that write state), the count is verifier 3, governance 6, compliance 1, swap 6. This implementation gates 4 verifier entrypoints (the three state-changing ones plus `verify` and `verify_batch`, so proof acceptance stops), 4 governance, 1 compliance, and 4 swap. The cancel and revoke paths listed above stay callable during a pause. Compliance has one state-changing entrypoint, not two. The roadmap's total of 18 counts `initialize` for compliance and swap, but not for verifier and governance; this document applies the rule consistently.

## Governance-controlled token settings

The token's admin-only settings are changed through governance, so the token's admin is the governance contract. Governance applies three kinds of change, each through the same timelock as a verifying-key update:

- **Minimum shield amount** (`MinShieldAmount`), so `MIN_SHIELD_AMOUNT` changes without a redeploy.
- **Asset approval** (`AssetApproval`), so an asset can be shielded only after governance approves it.
- **Relayer approval** (`Relayer`), so a fee-paying relayer is allowed only after governance approves it.

The flow is the same for each:

1. `queue_token_action(action)` (admin, gated by pause). A new queued action replaces any pending one and restarts the timelock. Non-positive minimum shield amounts are rejected.
2. `execute_token_action()` after `timelock_ledgers()` ledgers (admin, gated by pause).
3. `guardian_cancel_token_action()` (guardian only, not gated by pause) cancels a queued action.

Tested against the real token contract: values are unchanged before the timelock and change after it, a cancelled action never applies, and asset and relayer approvals take effect after the timelock. Deployment must set governance as the token's admin.

## Deferred to a later governance upgrade

These are out of scope for this deliverable, as the roadmap states:

- Admin multisig. Moving admin control to a multisig account is a deployment decision for mainnet.
- Circuit upgrade authorization beyond verifying-key rotation.
- Fine-grained parameter adjustment beyond the items named above.

## Tests

Each gated entrypoint has a test that the pause blocks it, and each exempt entrypoint has a test that the pause does not block it. Unpause is tested to restore every gated entrypoint in all four contracts. Pause and unpause are tested to require admin authorization, and the guardian's cancel is tested to require guardian authorization. Run the tests with `cd contracts && cargo test --workspace --release`.
