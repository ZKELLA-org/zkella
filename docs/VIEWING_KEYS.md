# Viewing keys: disclosure, auditing, and revocation

This document describes how an account holder grants an auditor read access to their shielded history, and what revoking that access does and does not guarantee.

## What a viewing key can and cannot do

A viewing key is a scalar derived from the holder's spending key. The holder can give it to an auditor, who then decrypts the value and commitment-opening secrets of every note encrypted to that key. A viewing key cannot spend a note, cannot compute nullifiers, and cannot compute the owner key. Spend authority stays with the holder.

Decryption is an off-chain operation. Once an auditor has a key, nothing on-chain can take the ability to decrypt back. Revocation therefore works by rotation, and only for notes encrypted after the rotation.

## Epochs

The holder's viewing key is split into epochs:

- **Epoch 0** is the original key. Addresses and notes created before epochs existed are unchanged.
- **Epoch `e > 0`** is an independent key, derived from the spending key and `e`. Its shielded addresses are derived from the same spending key, with the epoch included in the diversifier, so an address from one epoch cannot be linked to another epoch's key.

Notes are encrypted to the transmission key of the epoch that the sender used. A wallet decrypts incoming notes with every epoch key up to its current epoch, so it still receives funds sent to any earlier address.

The owner key used in note commitments does not depend on the epoch. Circuits, verifiers, and contracts see no change.

## Granting access

1. The holder calls `exportCurrentViewingKey()` on the wallet. The export contains the epoch's viewing key, transmission key, network and birthday ledger.
2. The holder gives the export to the auditor.
3. The auditor runs `ZKELLAAuditor.sync()` and reads `transactionHistory(asset)`.

The auditor sees receipts only: value, asset, and leaf position. It cannot see spends, because a spend needs a nullifier, and a nullifier needs the nullifier key. Balances and spends are therefore not part of what a viewing key reveals.

## Revoking access

1. The holder calls `wallet.rotateViewingKey()`. This starts a new epoch.
2. Future notes are encrypted to the new epoch's key and receive addresses come from the new epoch. An auditor holding only an earlier epoch's key cannot decrypt them.
3. The holder calls `revoke(owner)` on the viewing-key registry to withdraw the advertised commitment. The registry's `register` also replaces a commitment, so rotating can publish the new one.

### What revocation does not do

- **Notes encrypted before the rotation remain decryptable** by any holder of the earlier key. Those ciphertexts are published on-chain and cannot be re-encrypted by the holder. The roadmap criterion "the designated party can no longer decrypt note history after revocation" cannot be met for history under this scheme. The criterion should read "the designated party cannot decrypt notes received after revocation". This needs the roadmap's author to accept the rewording.
- **Senders who still hold an old address** can still send to it, and the old auditor will see those notes. The holder should stop publishing old addresses; the wallet cannot enforce that.
- **The on-chain `revoke` call** does not stop an auditor from decrypting anything. It removes the registry entry, which lets verifiers and integrators see that the holder no longer advertises that commitment.

## Tests

- `tests/unit/viewing-key-rotation.test.ts` checks that epoch 0 is unchanged, that an earlier epoch's auditor cannot decrypt notes from a later epoch, that the earlier auditor still decrypts earlier notes, and that each epoch derives a different address.
- `tests/unit/auditor.test.ts` checks that a granted key recovers the note value and position, that another wallet's key recovers nothing, that `sync` collects only matching notes, and that `sync` stops when the indexer does not advance its cursor.
- `contracts/viewing_keys` tests check that `revoke` removes the commitment, that re-registering replaces it, and that both `register` and `revoke` require the owner's authorization.

Wallet-level sync across epochs is exercised by the code path but has no dedicated live test yet.

## Sanctions list maintenance

The compliance contract accepts a non-membership proof only against the sanctions-list root the admin has set. The admin is the list maintainer. Each `publish_compliance_proof` call is rejected with `UnknownSanctionsRoot` unless its root matches the admin's current root, so a prover cannot choose a root of their own. Changing the root is `set_sanctions_root`, which requires the admin's authorization. Publishing is also blocked while the compliance contract is paused.

Still to decide, and not part of this change:

- **Update cadence:** a fixed schedule (for example weekly) and an urgent path for listings that must take effect sooner.
- **Root provenance:** the source list should be published with its hash so anyone can recompute the root.
- **Maintainer identity:** the admin is currently a single key. Moving it to a multisig is a deployment decision, as described in `docs/GOVERNANCE.md`.
