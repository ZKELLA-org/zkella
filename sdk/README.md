# @zkella/sdk

[![npm](https://img.shields.io/npm/v/%40zkella%2Fsdk.svg)](https://www.npmjs.com/package/@zkella/sdk)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://github.com/ZKELLA-org/zkella/blob/main/LICENSE)

TypeScript SDK for **ZKELLA**, confidential finance infrastructure for Stellar Soroban: a shielded token, auditor viewing keys, sanctions non-membership proofs, and a private commit-reveal swap — all backed by real Groth16 proofs, live on Stellar Testnet.

This package is the client: key derivation, Groth16 proof generation (via `snarkjs`), Soroban transaction construction/submission, and the wallet/swap/auditor/compliance classes that wrap the deployed contracts. For the contracts, circuits, and full protocol source, see the [main repository](https://github.com/ZKELLA-org/zkella).

**Scope:** Stellar Testnet only. No third-party security audit has been performed yet, and the circuits' trusted setup is a development ceremony — not suitable for custody of real funds. See [`docs/RUNBOOK.md`](https://github.com/ZKELLA-org/zkella/blob/e93154bd9efd40cf6e4d3d035c8797b378658f29/docs/RUNBOOK.md) and ["Roadmap to mainnet"](https://github.com/ZKELLA-org/zkella/blob/e93154bd9efd40cf6e4d3d035c8797b378658f29/README.md#roadmap-to-mainnet) for the precise boundary.

- [What's in this package](#whats-in-this-package)
- [Install](#install)
- [Quick start](#quick-start)
- [Public API](#public-api)
- [Live Testnet deployment](#live-testnet-deployment)
- [Links](#links)
- [License](#license)

## What's in this package

- **Keys** — spending/nullifier/viewing/transmission key derivation, diversified shielded addresses, viewing-key export for an auditor.
- **Wallet** — `shield()`/`transfer()`/`unshield()` with real Groth16 proving and real Soroban transaction submission, not stubs.
- **Swap** — `commitSwap()`/`revealAndClaim()`/`cancelSwap()` against the real commit-reveal shielded swap contract.
- **Compliance** — sanctions non-membership proof generation and publishing against the real compliance contract.
- **Auditor** — decrypts real note history from a granted viewing key, with resumable sync.
- **Relayer RFQ** — a quote client/handler pair for off-chain price discovery that enforces the exact same slippage floor the swap's on-chain fairness circuit checks.
- **Indexer client** — typed access to notes, Merkle paths, and nullifier checks from a `zkella` indexer instance.
- Full TypeScript types for every public method and payload (`dist/index.d.ts`), CommonJS output, no native/Node-only dependencies — runnable in Node.js or bundled for the browser (exercised in CI against a real headless Chromium via a Web Worker check).

**What this package does not include:** the Groth16 circuit artifacts themselves (`.wasm`/`.zkey` — build them from [`circuits/`](https://github.com/ZKELLA-org/zkella/tree/e93154bd9efd40cf6e4d3d035c8797b378658f29/circuits) in the main repository, or point at your own), a relayer server (only the client/handler contract for one), and a reference wallet UI.

## Install

```bash
npm install @zkella/sdk
```

Requires Node.js 20 or later. Pin the exact version — see [`docs/RUNBOOK.md`](https://github.com/ZKELLA-org/zkella/blob/e93154bd9efd40cf6e4d3d035c8797b378658f29/docs/RUNBOOK.md), "SDK artifacts".

## Quick start

```js
const { ZKELLAKeys, ZKELLAWallet, TESTNET_CONTRACTS, TESTNET_SOROBAN_RPC } = require('@zkella/sdk')

const keys = await ZKELLAKeys.fromSeed(mySeed) // or ZKELLAKeys.generate()
const wallet = new ZKELLAWallet({
  keys: keys.spendingKey,
  network: 'testnet',
  sorobanRpc: TESTNET_SOROBAN_RPC,
  indexerUrl: 'http://localhost:8080', // run one from indexer/ in the main repo
  tokenAddress: TESTNET_CONTRACTS.token,
  stellarSecret: 'S...', // the Stellar account that signs and pays for transactions
  shieldCircuit: { wasmPath: 'circuits/shield/build/shield_js/shield.wasm', zkeyPath: 'circuits/shield/build/shield.zkey' },
})

await wallet.sync()
// asset is a SEP-41 contract address, e.g. native XLM's own Stellar Asset Contract:
//   stellar contract id asset --asset native --network testnet
const { submit } = await wallet.shield({ asset: nativeAssetContract, amount: 10_000_000n })
await submit()
```

Every on-chain call follows this shape: the method builds the transaction and generates the proof, `submit()` sends it and waits for confirmation. Circuit artifacts (`.wasm` witness generator, `.zkey` proving key) are not bundled in this package — pass their paths from the main repository's `circuits/` directory, or your own build of the same circuits.

## Public API

| Class or function | Purpose |
| --- | --- |
| `ZKELLAKeys.fromSeed(seed)` / `.generate()` | Derive spending/viewing keys from a 32-byte seed. |
| `keys.deriveAddress(index)` / `.deriveAddressForEpoch(epoch, index)` | Shielded addresses. |
| `keys.exportViewingKey(birthday, network)` | Viewing-key export for an auditor. |
| `ZKELLAWallet` | `sync()`, `balance(asset)`, `shield()`, `transfer()`, `unshield()`, `rotateViewingKey()`, `exportCurrentViewingKey()`, `receiveAddress()`, `spendableNotes(asset)`. |
| `ZKELLASwap` | `commitSwap()`, `revealAndClaim(intent)`, `cancelSwap(swapId)` — real calls against `contracts/swap`. |
| `ZKELLACompliance` | `generateNonSanctionedProof()`, `publishProof(proof)` — real calls against `contracts/compliance`. |
| `ZKELLAAuditor` | `sync()`, `transactionHistory(asset)` — decrypts real note history from a granted viewing key. |
| `IndexerClient` | `getNotes(fromLedger, limit)`, `getMerklePath(leafIndex)`, nullifier checks. |
| `SwapQuoteClient` | `requestQuote(req)` — off-chain price discovery from a relayer, validated against the same slippage floor the chain enforces (`quoteRespectsSlippage()`, `QuoteValidationError`). |
| `TESTNET_CONTRACTS`, `TESTNET_SOROBAN_RPC` | The current live Testnet addresses and RPC endpoint. |

Full API reference, troubleshooting, and what to persist between sessions: [`docs/SDK_DEVELOPER.md`](https://github.com/ZKELLA-org/zkella/blob/e93154bd9efd40cf6e4d3d035c8797b378658f29/docs/SDK_DEVELOPER.md). Runnable examples for every flow (keys, shield, indexer query, viewing-key audit, transfer, unshield, swap): [`examples/`](https://github.com/ZKELLA-org/zkella/tree/e93154bd9efd40cf6e4d3d035c8797b378658f29/examples).

## Live Testnet deployment

`TESTNET_CONTRACTS` tracks the current stack (also recorded in the main repository's `deployments.json`, `testnet_final`):

| Contract | Address |
| --- | --- |
| Token | `CA5TFEVODC25SSEZII2XHB2XMCKFNXNLXRNFTKWPKMT5PCWYZUMLPRUZ` |
| Verifier | `CC2LQPXH3L5YKRP7YJ6UIC57AOGJXBQN4DEKNRU4Y32ABXJZOENCDAX3` |
| Governance | `CDTJLTBEKBXRJJKHVI32A5UMBB4UC6VBMDOF7WR43H2SKCCRAVRJWY5Q` |
| Compliance | `CDP5SRSUFDVEYHUCUX53SM4PZVTOIHDZR3Z5C7G4TFFKAQSLX64FOZVJ` |
| Viewing keys | `CDT776JLXU5GWRIY6WXLZGVKZ5V4TG32HAITNPFZVX5UCJSMMFHNMEEE` |
| Swap | `CBN7JJEPAEA5NCKOECPPGHETAK4CCCUFOBUJCDZ7K7HPIV7Y6ILOC524` |

## Links

- [Main repository](https://github.com/ZKELLA-org/zkella) — contracts, circuits, indexer, full documentation
- [Delivery record](https://github.com/ZKELLA-org/zkella/blob/e93154bd9efd40cf6e4d3d035c8797b378658f29/docs/TRANCHE3_DELIVERABLES.md) — every claim backed by a GitHub link or an on-chain transaction
- [Report a vulnerability](https://github.com/ZKELLA-org/zkella/security/advisories/new)
- [Issues](https://github.com/ZKELLA-org/zkella/issues)

## License

Apache 2.0 — included as `LICENSE` in this package, and at [`LICENSE`](https://github.com/ZKELLA-org/zkella/blob/main/LICENSE) in the main repository.
