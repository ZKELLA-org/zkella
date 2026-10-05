# ZKELLA SDK examples (Stellar Testnet)

Each script runs against the Testnet stack in `deployments.json` (`testnet_final`). Build the SDK first, and set the environment variables each script lists.

```bash
npm run build --workspace=sdk
```

Circuit artifacts are read from `circuits/*/build`, which exists after `circuits/build.sh` has been run for each circuit. Never put a real secret in a file; pass it through the environment.

| Script | What it shows | Needs |
| --- | --- | --- |
| `01-keys-and-address.cjs` | Derive keys, a shielded address, and a viewing-key export. Offline. | nothing |
| `02-shield.cjs` | Shield a native amount into the pool through the wallet. | `STELLAR_SECRET`, `SHIELD_AMOUNT` |
| `03-indexer-query.cjs` | Query notes and Merkle paths from an indexer. | `INDEXER_URL` |
| `04-viewing-key-audit.cjs` | Recover receipts with a granted viewing key. | `INDEXER_URL`, `VIEWING_KEY_EXPORT` (JSON file) |
| `05-transfer.cjs` | Transfer shielded value to another wallet. Needs two spendable notes. | `STELLAR_SECRET`, `INDEXER_URL`, `RECIPIENT_TK`, `TRANSFER_AMOUNT` |
| `06-unshield.cjs` | Withdraw part of a note to a public address. | `STELLAR_SECRET`, `INDEXER_URL`, `UNSHIELD_AMOUNT`, `UNSHIELD_TO` |
| `07-swap.cjs` | Commit a note into a swap intent and cancel it after expiry. Reveal needs a relayer's execution, which is outside this script. | `STELLAR_SECRET`, `INDEXER_URL`, `SWAP_AMOUNT` |

Each script prints the Testnet transaction link for every on-chain step.
