# ZKELLA SDK developer guide

For developers building on `@zkella/sdk` against the Stellar Testnet deployment. Mainnet is out of scope for this release.

## Installation

```bash
npm install @zkella/sdk@0.1.0
```

Pin the exact version (see `docs/RUNBOOK.md`, "SDK artifacts"). The SDK needs Node.js 20 or later.

## Testnet configuration

The Testnet addresses are exported as `TESTNET_CONTRACTS` and must match `deployments.json` (`testnet_final`):

```js
const { TESTNET_CONTRACTS, TESTNET_SOROBAN_RPC } = require('@zkella/sdk')
// TESTNET_CONTRACTS.token, .swap, .compliance, .governance, .verifier, .viewingKeys
```

Proofs need the circuit artifacts (`.wasm` witness generator and `.zkey` proving key) for each circuit. Pass their paths to the wallet's `shieldCircuit`, `transferCircuit`, `unshieldCircuit`, and to the swap and compliance helpers. The artifacts must match the verifying keys deployed on Testnet.

## Public API

| Class or function | Purpose |
| --- | --- |
| `ZKELLAKeys.fromSeed(seed)` / `generate()` | Derive spending keys from a 32-byte seed. |
| `keys.deriveAddress(index)` / `deriveAddressForEpoch(epoch, index)` | Shielded addresses. |
| `keys.exportViewingKey(birthday, network)` | Viewing-key export for an auditor. |
| `ZKELLAWallet` | `sync()`, `balance(asset)`, `shield()`, `transfer()`, `unshield()`, `rotateViewingKey()`, `exportCurrentViewingKey()`, `receiveAddress()`, `spendableNotes(asset)`. |
| `ZKELLASwap` | `commitSwap()`, `revealAndClaim(intent)`, `cancelSwap(swapId)`. |
| `ZKELLACompliance` | `generateNonSanctionedProof()`, `publishProof(proof)`. |
| `ZKELLAAuditor` | `sync()`, `transactionHistory(asset)`, with a granted viewing key. |
| `IndexerClient` | `getNotes(fromLedger, limit)`, `getMerklePath(leafIndex)`, nullifier checks. |

Every on-chain call returns a `submit` function. Calling it sends the transaction, waits for confirmation, and returns the result. Proofs are generated when the method is called, not when `submit` runs.

## Indexer usage

The wallet and the auditor read notes and Merkle paths from an indexer. Run one with `indexer/` (see `indexer/README.md`) pointed at `TESTNET_CONTRACTS.token`. Set `INDEXER_API_KEYS` for any instance other clients can reach, and pass its base URL as `indexerUrl`.

## Examples

`examples/` has one script per flow: keys, shield, indexer query, viewing-key audit, transfer, unshield, and swap. Each script reads its secrets from environment variables. See `examples/README.md`.

## Troubleshooting

The table in `docs/RUNBOOK.md` ("SDK artifacts") lists the common failures and what to check for each: anchor errors, proof verification failures, sanctions-root mismatches, paused contracts, transient RPC errors, viewing-key epochs, and npm audit findings in a fresh install.

## What to keep between sessions

- The wallet's current viewing-key epoch (`currentEpoch`) and last synced ledger. A restarted wallet without them can miss notes.
- The swap intent returned by `commitSwap`, including `outNote` — the output note's randomness is committed on-chain at commit time, so `revealAndClaim` must reveal the exact same note, not a freshly built one; keep the whole `SwapIntent` object, not just the nonce and fairness proof.
- Spending keys. Losing them makes notes unspendable; nothing on-chain can recover them.
