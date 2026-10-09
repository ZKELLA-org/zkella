# ZKELLA SDK examples (Stellar Testnet)

Each script runs against the Testnet stack in `deployments.json` (`testnet_final`). Build the SDK first, and set the environment variables each script lists.

```bash
npm run build --workspace=sdk
```

Circuit artifacts are read from `circuits/*/build`, which exists after `circuits/build.sh` has been run for each circuit.

Wallet seed: the wallet examples read a 32-byte seed from `SPENDING_SEED` (64 hex characters) or from the file named by `SPENDING_SEED_FILE`. If the file does not exist, it is created with mode 0600. Keep that file: a note is spendable only with the key derived from its seed. Stellar account secrets go in the environment only, never in a file.

| Script | What it shows | Needs |
| --- | --- | --- |
| `01-keys-and-address.cjs` | Derive keys, a shielded address, and a viewing-key export. Offline. | nothing |
| `02-shield.cjs` | Shield a native amount into the pool through the wallet. | `STELLAR_SECRET`, `SHIELD_AMOUNT` |
| `03-indexer-query.cjs` | Query notes and Merkle paths from an indexer. | `INDEXER_URL` |
| `04-viewing-key-audit.cjs` | Recover receipts with a granted viewing key. | `INDEXER_URL`, `VIEWING_KEY_EXPORT` (JSON file) |
| `05-transfer.cjs` | Transfer shielded value to another wallet. Needs two spendable notes. | `STELLAR_SECRET`, `INDEXER_URL`, `RECIPIENT_TK`, `RECIPIENT_OWNER_KEY`, `TRANSFER_AMOUNT`, `ASSET_ID` |
| `06-unshield.cjs` | Withdraw part of a note to a public address. | `STELLAR_SECRET`, `INDEXER_URL`, `UNSHIELD_AMOUNT`, `UNSHIELD_TO` |
| `07-swap.cjs` | Commit a note into a swap intent and cancel it after expiry. Reveal needs a relayer's execution, which is outside this script. | `STELLAR_SECRET`, `INDEXER_URL`, `SWAP_AMOUNT` |

Status: all seven scripts have been run end to end against live Testnet, including a full re-run of the entire set against the current `testnet_final` stack — `01`/`02` (keys, shield), `03` (indexer query), `04` (viewing-key audit, real receipts recovered), `05` (transfer), `06` (unshield), and `07` (swap, exercised via the full commit/execute/reveal-and-claim lifecycle in `docs/TRANCHE3_DELIVERABLES.md`, not only the commit/cancel path this script itself demonstrates). See `docs/TRANCHE3_DELIVERABLES.md` and `docs/POC_TESTNET_VALIDATION.md` for the transaction hashes.
