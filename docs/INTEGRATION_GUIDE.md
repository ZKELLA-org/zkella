# ZKELLA — Integration Guide

**Version:** 0.1.0  
**Audience:** Soroban developers building on top of the ZKELLA Protocol

---

## 1. Overview

This guide covers how to integrate ZKELLA's confidential token standard, viewing key system, and shielded swap primitive into your own Soroban application or TypeScript frontend.

ZKELLA exposes:
- A **CT-20 token contract** on Soroban — interact with it from any Soroban contract or client
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

```bash
npm install @zkella/sdk
```

The SDK bundles all Groth16 WASM proving artifacts. No separate download required.

---

## 4. Key Generation

```typescript
import { ZKELLAKeys, ZKELLAWallet } from '@zkella/sdk'

// Generate a new wallet from a random seed
const keys = ZKELLAKeys.generate()

// Or restore from an existing seed (32 bytes)
const keys = ZKELLAKeys.fromSeed(seedBytes)

// Derive a shielded address (diversified — multiple addresses from one key)
const address = keys.deriveAddress(0)  // diversifier index
console.log(address.toString())
// zkella1abc...xyz
```

**Never store `keys.spendingKey` in plaintext.** Encrypt it using the user's wallet password before persisting.

---

## 5. Wallet Initialization

```typescript
import { ZKELLAWallet } from '@zkella/sdk'

const wallet = new ZKELLAWallet({
  keys,
  network:     'testnet',                               // 'testnet' | 'mainnet'
  sorobanRpc:  'https://soroban-testnet.stellar.org',
  indexerUrl:  'https://testnet-indexer.zkella.io/v1',
  ct20Address: 'CXXX...YYY',                            // CT-20 contract address
})

// Sync note set (call on startup and periodically)
await wallet.sync()
```

`sync()` fetches all encrypted notes from the indexer since the last sync ledger, attempts to decrypt each with the viewing key, and filters to notes belonging to this wallet.

---

## 6. Checking Shielded Balance

```typescript
const USDC = 'CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA'

const balance = await wallet.balance(USDC)
console.log(balance.shielded)    // bigint — amount in shielded pool
console.log(balance.unshielded)  // bigint — public Stellar balance
```

---

## 7. Shield (Move tokens into the shielded pool)

```typescript
const tx = await wallet.shield({
  asset:  USDC,
  amount: 100_000_000n,  // 100 USDC (7 decimals)
})

// Inspect the transaction before submitting
console.log(tx.toXDR())

// Submit
const result = await tx.submit()
console.log('Note committed at leaf index:', result.leafIndex)
```

The SDK:
1. Generates a fresh note `(value, asset, rho, rcm)`
2. Generates a Groth16 shield proof (~200ms)
3. Approves the SEP-41 token transfer
4. Submits the `shield()` Soroban transaction

---

## 8. Private Transfer

```typescript
const tx = await wallet.transfer({
  to:     'zkella1recipient...address',
  asset:  USDC,
  amount: 50_000_000n,
})

const result = await tx.submit()
// Input nullifiers are now spent; output notes appear in recipient's wallet on next sync
```

The SDK automatically selects input notes, constructs change notes, and chooses 2x2 or 4x4 circuit based on the number of input notes needed.

---

## 9. Unshield (Move tokens out of the shielded pool)

```typescript
const tx = await wallet.unshield({
  asset:  USDC,
  amount: 25_000_000n,
  to:     'GABCD...WXYZ',  // public Stellar address
})

await tx.submit()
// Tokens appear at the public Stellar address
```

---

## 10. Shielded Swap

```typescript
import { ZKELLASwap } from '@zkella/sdk'

const swap = new ZKELLASwap({
  wallet,
  swapContractAddress: 'CSWAP...XYZ',
  relayerUrl: 'https://relayer.zkella.io',
})

const tx = await swap.commitSwap({
  assetIn:        USDC,
  assetOut:       XLM_CONTRACT,
  amountIn:       50_000_000n,
  maxSlippageBps: 50,           // 0.5% slippage tolerance
  expiryLedgers:  720,          // ~1 hour
})

const { swapId } = await tx.submit()

// Poll for execution (relayer executes asynchronously)
const executed = await swap.waitForExecution(swapId, { timeoutMs: 300_000 })

if (executed) {
  const claimTx = await swap.revealAndClaim(swapId)
  await claimTx.submit()
  // Output tokens are now a shielded note in your wallet
} else {
  // Expired — reclaim input
  const cancelTx = await swap.cancelSwap(swapId)
  await cancelTx.submit()
}
```

---

## 11. Viewing Key Export (for auditors)

```typescript
// Export viewing key — safe to share with auditors
const vkJson = wallet.exportViewingKey()
// {
//   "version": 1,
//   "network": "mainnet",
//   "viewing_key": "0x...",
//   "transmission_key": "0x...",
//   "birthday_ledger": 12345678
// }

// Auditor: import and sync
import { ZKELLAAuditor } from '@zkella/sdk'

const auditor = new ZKELLAAuditor({
  viewingKeyExport: vkJson,
  indexerUrl: 'https://indexer.zkella.io/v1',
})

await auditor.sync()
const history = await auditor.transactionHistory(USDC)
// [{ type: 'receive', amount: 100_000_000n, ledger: 12345700 }, ...]
```

---

## 12. Sanctions Compliance Proof

```typescript
import { ZKELLACompliance } from '@zkella/sdk'

const compliance = new ZKELLACompliance({ wallet })

// Fetch latest published sanctions list (from a compliance provider)
const sanctionsList = await ZKELLACompliance.fetchSanctionsList(
  'https://sanctions.zkella.io/latest'
)

// Generate ZK proof that this wallet is not sanctioned
// (spending key never leaves the client)
const proof = await compliance.generateNonSanctionedProof(sanctionsList)

// Submit proof on-chain (optional — for Travel Rule compliance)
const tx = await compliance.publishProof(proof)
await tx.submit()

// Share proof with counterparty or VASP
console.log(proof.toJSON())
```

---

## 13. Calling the CT-20 Contract from Another Soroban Contract

If you are building a Soroban contract that interacts with ZKELLA (e.g., a DeFi protocol that accepts shielded deposits), use the CT-20 contract interface:

```rust
// In your Soroban contract
use soroban_sdk::{contract, contractimpl, Address, Bytes, BytesN, Env, Vec};

// Import the CT-20 client (generated from the contract's ABI)
mod ct20 {
    soroban_sdk::contractimport!(
        file = "../../contracts/ct20/target/wasm32-unknown-unknown/release/ct20.wasm"
    );
}

#[contract]
pub struct MyProtocol;

#[contractimpl]
impl MyProtocol {
    // Accept a shielded note as a deposit proof
    pub fn verify_shielded_deposit(
        env:      Env,
        ct20:     Address,
        nullifier: BytesN<32>,
    ) -> bool {
        let client = ct20::Client::new(&env, &ct20);
        // Check that the nullifier has been spent (note was consumed in a transfer to us)
        client.is_spent(&nullifier)
    }

    // Read current Merkle root for use in your own circuit proofs
    pub fn get_merkle_root(env: Env, ct20: Address) -> BytesN<32> {
        let client = ct20::Client::new(&env, &ct20);
        client.merkle_root()
    }
}
```

---

## 14. Running a Local Indexer

For development or self-hosting:

```bash
# Clone and build
git clone https://github.com/Frihat-dev/ZKELLA
cd ZKELLA/indexer
cargo build --release

# Configure
cp config.example.toml config.toml
# Edit config.toml:
#   soroban_rpc = "https://soroban-testnet.stellar.org"
#   ct20_contract = "CXXX...YYY"
#   database_url = "postgres://localhost/zkella_indexer"
#   start_ledger = 12345678

# Run migrations
./target/release/zkella-indexer migrate

# Start
./target/release/zkella-indexer serve
# Listening on 0.0.0.0:3000
```

API is available at `http://localhost:3000/v1`.

---

## 15. Contract Addresses

| Network | Contract | Address |
|---|---|---|
| Testnet | CT-20 | TBD (post-deployment) |
| Testnet | Viewing Key Registry | TBD |
| Testnet | Shielded Swap | TBD |
| Mainnet | CT-20 | TBD (post-audit) |
| Mainnet | Viewing Key Registry | TBD |
| Mainnet | Shielded Swap | TBD |

Addresses will be published in `deployments.json` at the root of the repository after each deployment.

---

## 16. Error Reference

| Error Code | Description | Resolution |
|---|---|---|
| `InvalidProof` | Groth16 verification failed | Regenerate proof; check anchor is current |
| `NullifierAlreadySpent` | Note was already consumed | Sync wallet to update note set |
| `InvalidAnchor` | Merkle root in proof is too old | Fetch latest root and regenerate proof |
| `InsufficientBalance` | Not enough notes to cover amount | Check balance and sync |
| `SwapExpired` | Swap was not executed before expiry | Cancel swap and retry |
| `ContractPaused` | CT-20 contract is paused | Monitor governance for unpause |

---

*ZKELLA Integration Guide v0.1.0*
