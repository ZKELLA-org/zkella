# ZKELLA — Integration Guide

**Version:** 0.1.0  
**Audience:** Soroban developers building on top of the ZKELLA Protocol

**Implementation status:** the Soroban contracts (shield/transfer/unshield, the shielded swap, the verifier/governance/compliance/viewing-key registries) and the SDK's core crypto, note, and prover modules are real and exercised on live Stellar Testnet — not stubs. Some SDK convenience wrapper classes are still stubs, though (`ZKELLASwap`, `ZKELLAAuditor`, `ZKELLACompliance` — flagged explicitly in the relevant sections below). None of this has been through an *external* security review or a production (multi-party) trusted-setup ceremony yet, so treat everything here as real but not yet production-hardened, and check `docs/POC_IMPLEMENTATION.md` for exactly what's validated where before building anything that handles real value.

---

## 1. Overview

This guide covers how to integrate ZKELLA's confidential token standard, viewing key system, and shielded swap primitive into your own Soroban application or TypeScript frontend.

Until the implementation leaves the soft PoC stage, use these examples as technical design references and testnet scaffolding only.

ZKELLA exposes:
- A **`ShieldedToken` contract** on Soroban — interact with it from any Soroban contract or client
- A **TypeScript SDK** (`@zkella/sdk`) — handles proof generation, key management, and note sync
- A **REST indexer API** — provides Merkle paths and note history beyond the 7-day RPC window

---

## 2. Prerequisites

- Node.js 20+
- Stellar Soroban SDK (`@stellar/stellar-sdk` ≥ 12.0)
- A Soroban RPC endpoint (testnet: `https://soroban-testnet.stellar.org`)
- A funded Stellar testnet account

---

## 3. SDK Installation

`@zkella/sdk` (`sdk/package.json`) has **not been published to the npm registry yet**. Today it's consumed by importing directly from the monorepo:

```typescript
import { ZKELLAKeys } from '<path-to-repo>/sdk/src/keys/keys'
import { ZKELLAWallet } from '<path-to-repo>/sdk/src/wallet/wallet'
```

`npm install @zkella/sdk` will work once a real release is published — there is no evidence of one in this repository yet, so don't rely on that command today.

The SDK does *not* bundle Groth16 WASM proving artifacts — you point `wallet`'s circuit config at the compiled `.wasm`/`.zkey` files under `circuits/<name>/build/` yourself (see §5). Those artifacts are from a local, single-contributor dev trusted-setup ceremony, not a production one — see `docs/POC_IMPLEMENTATION.md`.

---

## 4. Key Generation

```typescript
import { ZKELLAKeys } from '../sdk/src/keys/keys'

// Generate a new wallet from a random seed — async (derives sk/nk/vk/tk)
const keys = await ZKELLAKeys.generate()

// Or restore from an existing seed (32 bytes)
const keys = await ZKELLAKeys.fromSeed(seedBytes)

// Derive a shielded address (diversified — multiple addresses from one key)
const address = await keys.deriveAddress(0)  // diversifier index
console.log(address.toString())
// zkella1abc...xyz
```

**Never store `keys.spendingKey` in plaintext.** Encrypt it using the user's wallet password before persisting.

**Known gap:** shielded addresses currently have no checksum (see `docs/TECHNICAL_SPEC.md` §4.2) — a typo produces a silently wrong address, not a decode error. Verify addresses out-of-band before sending funds.

---

## 5. Wallet Initialization

```typescript
import { ZKELLAWallet } from '../sdk/src/wallet/wallet'

const wallet = new ZKELLAWallet({
  keys: keys.spendingKey,
  network:      'testnet',                               // 'testnet' | 'mainnet'
  sorobanRpc:   'https://soroban-testnet.stellar.org',
  indexerUrl:   'http://localhost:8787',                  // self-hosted — see §14, no hosted endpoint exists
  tokenAddress:  'CXXX...YYY',                             // see §15 for the current live testnet address
  stellarSecret: mySecretKey,                             // signs the transactions wallet.*() submits
  shieldCircuit:   { wasmPath: '.../circuits/shield/build/shield_js/shield.wasm', zkeyPath: '.../circuits/shield/build/shield.zkey' },
  transferCircuit: { wasmPath: '.../circuits/transfer_2in2out/build/transfer_js/transfer.wasm', zkeyPath: '.../circuits/transfer_2in2out/build/transfer.zkey' },
  unshieldCircuit: { wasmPath: '.../circuits/unshield/build/unshield_js/unshield.wasm', zkeyPath: '.../circuits/unshield/build/unshield.zkey' },
})

// Sync note set (call on startup and periodically)
await wallet.sync()
```

`sync()` fetches all encrypted notes from the indexer since the last sync ledger, attempts to decrypt each with the viewing key, and filters to notes belonging to this wallet (checking spent status via the indexer's nullifier-batch endpoint too).

---

## 6. Checking Shielded Balance

```typescript
const USDC = 'CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA'

const balance = await wallet.balance(USDC)
console.log(balance.shielded)  // bigint — amount in the shielded pool
```

There is no `balance.unshielded` field — the wallet's public Stellar balance is a normal Stellar account balance, tracked outside this SDK (e.g. via `server.getAccount(...)`), not something `ZKELLAWallet` reports.

---

## 7. Shield (Move tokens into the shielded pool)

```typescript
const { note, submit } = await wallet.shield({
  asset:  USDC,
  amount: 100_000_000n,  // 100 USDC (7 decimals)
})

// note.commitment, note.rho, note.rcm are already computed here;
// note.leafIndex is -1 until submit() resolves

const { leafIndex } = await submit()
console.log('Note committed at leaf index:', leafIndex)
```

There is no separate "inspect the XDR before submitting" step — `submit()` builds, signs (with `stellarSecret`), sends, and polls the transaction to completion in one call. Under the hood:
1. Builds a fresh note `(value, asset, rho, rcm)` and its commitment.
2. Encrypts the note to the wallet's own transmission key.
3. Generates a real Groth16 shield proof via `snarkjs` against the configured circuit artifacts.
4. Submits the `shield()` Soroban transaction and waits for it to confirm.

---

## 8. Private Transfer

`wallet.transfer()` needs **at least 2 spendable notes** of the asset (the transfer circuit has no dummy-input support), and today only picks the two largest unspent notes (simple, not fee-optimal coin selection) — the 4-in/4-out prover exists (`sdk/src/prover/transfer4.ts`) but isn't wired into the wallet's own note selection yet.

```typescript
const { submit } = await wallet.transfer({
  to:     recipientTransmissionKeyHex,   // raw hex transmission key today, not a full zkella1... diversified address — that decoding isn't wired into wallet.ts yet
  asset:  USDC,
  amount: 50_000_000n,
})

const { leafIndices } = await submit()
// Input nullifiers are now spent; output notes appear in the recipient's wallet on next sync
```

---

## 9. Unshield (Move tokens out of the shielded pool)

`wallet.unshield()` withdraws a single note's **full value** — there is no unshield-with-change entrypoint; split a note with `transfer()` first if you need to withdraw a partial amount.

```typescript
const { submit } = await wallet.unshield({
  asset:  USDC,
  amount: 25_000_000n,   // must exactly match an existing note's value
  to:     'GABCD...WXYZ',  // public Stellar address
})

await submit()
// Tokens appear at the public Stellar address
```

---

## 10. Shielded Swap

**The underlying `contracts/swap` contract is real, audited, and has been run end-to-end on live Stellar Testnet** (real escrow, real relayer-fronted liquidity, real fairness-proof-gated payout and re-shield — see `docs/POC_IMPLEMENTATION.md` for transaction hashes). **The `ZKELLASwap` SDK wrapper class shown in earlier drafts of this guide is not implemented** — `sdk/src/wallet/swap.ts`'s `commitSwap`/`waitForExecution`/`revealAndClaim`/`cancelSwap` all return placeholder values today. Do not build against it yet.

Until it's wired up, integrating with the swap contract means calling it directly, the way the live-Testnet demonstration did: build a real `unshield.circom` ownership proof (`generateUnshieldProof` from `sdk/src/prover/unshield.ts`, with `recipient` set to the swap contract's own address) and a real `swap_fairness.circom` proof (`generateSwapFairnessProof` from `sdk/src/prover/swapFairness.ts`), then invoke `commit_swap` / `execute_swap` / `reveal_and_claim` on the swap contract (via `stellar contract invoke` or your own `@stellar/stellar-sdk` transaction-building code — `ZKELLAWallet`'s private `submitContractCall` helper shows the pattern, though it isn't exposed publicly for swap calls yet). See `docs/TECHNICAL_SPEC.md` §6.5 and §9.3 for the exact contract signatures and call sequence.

---

## 11. Viewing Key Export (for auditors)

```typescript
// Export viewing key — safe to share with auditors
const vkJson = wallet.exportViewingKey()
// {
//   "version": 1,
//   "network": "testnet",
//   "viewing_key": "...",
//   "transmission_key": "...",
//   "birthday_ledger": 12345678
// }
```

`ZKELLAAuditor` (`sdk/src/wallet/auditor.ts`) exists but is a **stub** — `sync()` runs without error, but its note-decryption step always returns `null`, so it never actually recovers any history yet. There is no working auditor-side import/sync flow today; treat this class as a placeholder for the intended API shape, not something to integrate against.

---

## 12. Sanctions Compliance Proof

The on-chain side is real: `contracts/compliance::publish_compliance_proof` verifies a real Groth16 non-membership proof against `contracts/verifier`'s `NonMembership` circuit before storing anything. **The `ZKELLACompliance` SDK wrapper (`sdk/src/compliance/compliance.ts`) is a stub**, though — `generateNonSanctionedProof()` returns 192 zero bytes and an empty `ComplianceProof`, and `publishProof()` doesn't actually submit anything. There is no working end-to-end compliance-proof flow at the SDK level yet; integrating today means calling `contracts/compliance` directly with your own real `non_membership.circom` proof, the same way §10 describes for swaps.

---

## 13. Calling the ShieldedToken Contract from Another Soroban Contract

If you are building a Soroban contract that interacts with ZKELLA (e.g., a DeFi protocol that accepts shielded deposits), use the `ShieldedToken` contract interface:

```rust
// In your Soroban contract
use soroban_sdk::{contract, contractimpl, Address, BytesN, Env};

// Import the token client (generated from the compiled WASM's own spec).
// Prefer depending on `zkella-token-interface` (a #[contractclient]-only crate,
// no #[contract] of its own) instead if you're building in the same Rust
// workspace — depending on `zkella-token` directly, or importing raw WASM like
// this from within another Soroban contract crate, risks a `duplicate symbol`
// WASM linker error if your own contract happens to export a function with
// the same name as one of ShieldedToken's (Soroban contract exports are
// unconditional, never dead-code-eliminated). This raw `contractimport!`
// pattern is fine for a standalone external contract that isn't itself part
// of this workspace.
mod zkella_token {
    soroban_sdk::contractimport!(
        file = "../../contracts/target/wasm32v1-none/release/zkella_token.wasm"
    );
}

#[contract]
pub struct MyProtocol;

#[contractimpl]
impl MyProtocol {
    // Check whether a nullifier has been spent (e.g. note was consumed in a transfer to us)
    pub fn verify_shielded_deposit(env: Env, token_contract: Address, nullifier: BytesN<32>) -> bool {
        let client = zkella_token::Client::new(&env, &token_contract);
        client.is_spent(&nullifier)
    }

    // Read current Merkle root for use in your own circuit proofs
    pub fn get_merkle_root(env: Env, token_contract: Address) -> BytesN<32> {
        let client = zkella_token::Client::new(&env, &token_contract);
        client.merkle_root()
    }
}
```

---

## 14. Running a Local Indexer

The real indexer is a **TypeScript/Node service** (`indexer/`) using Node's built-in `node:sqlite` — no external database, no separate build/migration step, no Go or Rust toolchain involved:

```bash
git clone https://github.com/ZKELLA-org/zkella
cd ZKELLA
npm install

TOKEN_CONTRACT_ID=C...                                   \
SOROBAN_RPC_URL=https://soroban-testnet.stellar.org     \
ZKELLA_NETWORK=testnet                                  \
INDEXER_START_LEDGER=<token's deploy ledger>             \
npm run indexer
# Listening on :8787 by default (INDEXER_HTTP_PORT to override)
```

Optional env vars: `INDEXER_DB_PATH` (default `./indexer.db`), `INDEXER_HTTP_PORT` (default `8787`), `INDEXER_POLL_MS` (default `5000`). Requires Node 22.5+ (for `node:sqlite`, still an experimental API). See `indexer/README.md` for the current, authoritative instructions and status.

API is available at `http://localhost:8787` — `GET /health`, `GET /notes`, `GET /merkle/root`, `GET /merkle/path/:leafIndex`, `GET /commitment/:hex`, `POST /nullifiers/batch`. There is no hosted `indexer.zkella.io` endpoint; every wallet/auditor client points at a self-run instance today.

---

## 15. Contract Addresses

The current live Stellar Testnet addresses (redeployed whenever a contract/circuit change requires it — always check `deployments.json` at the repository root for the latest, since addresses in a static doc go stale):

| Network | Contract | Address |
|---|---|---|
| Testnet | verifier | `CCRLI4EAT62QVMTJR62NNJUZCERCGSYGNM534Z5R6RYSFRKELUZIG2MG` |
| Testnet | governance | `CDCSHTT3R75M3BEOEDPETB3RDB4BFXI5Q2KDI2KFT3O6M73WBVUBSZWD` |
| Testnet | compliance | `CA2EU46YYEBJW5C3JCRD3IAGTUD7UBPFBPYTT3I7UTESBK7FYXFCVG7Q` |
| Testnet | ShieldedToken | `CDE7U6HTLMDFAEQOT5BIZ3W7VJKAQN2MFQKYVV5E3W5YIPUBSRBHAXCE` |
| Testnet | swap | `CDPPRPAVKUJGNYE3AVFIBSTV7LCEOUPMM7USL7XARS2L2QRLUIMC53K3` |
| Testnet | viewing_keys | not currently deployed alongside the rest — deploy separately if needed |
| Mainnet | (all) | not deployed — mainnet deployment requires an external security review and a real multi-party trusted-setup ceremony first |

These are evidence of a working implementation, not permanent infrastructure — treat any specific address as likely to be redeployed. See `docs/POC_IMPLEMENTATION.md` for the full deployment history and the transactions run against the current set.

---

## 16. Error Reference

Real `ShieldedToken::Error` variants (`contracts/token/src/types.rs`) — the codes below are a representative subset most integrators hit; the full enum has 17 variants:

| Error | Meaning | Resolution |
|---|---|---|
| `InvalidProof` (4) | Groth16 verification failed | Regenerate the proof; check the anchor/public inputs match the call's actual arguments |
| `InvalidAnchor` (5) | Merkle root in proof doesn't match current state | Fetch the latest `merkle_root()` and regenerate the proof |
| `NullifierSpent` (6) | Note was already consumed | Sync the wallet to update its note set |
| `CommitmentMismatch` (7) | Recomputed commitment doesn't match the one supplied | Check `rho`/`rcm`/`value`/`asset` match what the proof was built for |
| `AssetMismatch` (8) / `AmountMismatch` (9) | Public inputs don't match the call's `asset`/`amount` arguments | Ensure the proof and the call arguments were built from the same values |
| `DuplicateCommitment` (14) | Same commitment submitted twice | Use fresh `rho`/`rcm` randomness per note |
| `RecipientMismatch` (16) | `unshield`'s `recipient_hash` doesn't bind the given `to` address | Recompute `recipient_hash = Poseidon2(address_field(to), 0)` for the actual `to` |
| `Paused` (3) | Contract is paused | Wait for an admin `unpause()` call |
| `NotInitialized` (2) / `AlreadyInitialized` (1) | Contract lifecycle error | Check you're calling `initialize()` exactly once, after deployment |

`contracts/swap` panics with descriptive messages rather than a typed error enum for most of its checks (e.g. `"swap already committed for this intent_commitment"`, `"swap not in committed state"`, `"expiry must be in the future"`) — inspect the transaction's diagnostic events for the exact panic message rather than a numeric code.

---

*ZKELLA Integration Guide v0.1.0*
