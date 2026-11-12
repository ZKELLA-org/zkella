# ZKELLA Data Flows and Trust Boundaries

Produced 2026-11-13, extending the data-flow diagram first drafted in `docs/EXTERNAL_AUDIT_PREPARATION.md` with explicit state stores and the trust-boundary detail `docs/audit/THREAT_MODEL.md`'s threat table maps against. Entities and crossings below were checked against the current contract/SDK/indexer source, not just the diagram.

```mermaid
flowchart TD
  subgraph Wallet["Wallet boundary — spending/viewing secrets never leave this box"]
    K["Spending key (nullifier-deriving) + viewing key (decrypt-only)\nsdk/src/keys/keys.ts — domain-separated KDF outputs from one root secret"]
    P["Note/proof/transaction construction\nsdk/src/prover/*, sdk/src/notes/*"]
    K --> P
  end
  subgraph Services["RPC + indexer boundary — availability-affecting, never spend-authorizing"]
    R["Soroban RPC (public, e.g. soroban-testnet.stellar.org)"]
    I["Indexer HTTP API\nindexer/src/main.ts"]
    DBSTORE[("Indexer DB — SQLite or PostgreSQL\nstores ONLY: leaf_index, commitment, encrypted_note, ledger\n(indexer/src/db-sqlite.ts:26, db-postgres.ts:42) — ciphertext, never plaintext")]
    I <--> DBSTORE
  end
  subgraph Chain["Public Soroban boundary — six contracts, each with its own instance storage"]
    T[["token\nstate: NextLeafIndex, ShieldedSupply(asset), CommitmentSeen(cm), Merkle tree nodes, AssetApproved, ApprovedRelayer, MinShieldAmount, Admin/PendingAdmin, Paused"]]
    V[["verifier\nstate: VerifyingKey(circuit) x6, PreviousVerifyingKey+Expiry, Admin (immutable after init — no transfer_admin exists), Paused"]]
    S[["swap\nstate: SwapState per swap_id (escrow, expiry, binding), ApprovedRelayer, Admin, Paused"]]
    G[["governance\nstate: queued actions + ready-ledger per action, timelock_ledgers, Admin, Guardian"]]
    C[["compliance\nstate: SanctionsRoot (admin-set, NOT governance-timelocked — see Elevation.1), ComplianceRecord per owner"]]
    W[["viewing_keys\nstate: vk_commitment per owner — no spend capability, confirmed no VerifierClient call in this contract at all"]]
    A["SEP-41 asset contracts (native XLM + any governance-approved asset)\nexternal, not ZKELLA code — issuer-level clawback/freeze rights apply"]
    T -->|"verify() / verify_batch()"| V
    S -->|"verify()"| V
    C -->|"verify()"| V
    S -->|"cross-contract shield()/unshield()\n(token-interface, reuses token's own audited proof path)"| T
    G -->|"queue_*/execute_* (timelocked)"| T
    G -->|"update_verifying_key (timelocked)"| V
    T <-->|"transfer() (SEP-41)"| A
    S <-->|"transfer() (SEP-41)"| A
  end
  subgraph Actors["Separate authority boundaries — distinct keys, distinct blast radius"]
    L["Approved relayer\n(token.set_relayer / swap.set_relayer allowlist)"]
    OA["Admin identity\n(deployer key on testnet_final; should be governance/multisig on mainnet)"]
    OG["Guardian identity\n(can cancel a queued governance action, cannot execute one)"]
    U["Viewing-key holder\n(read-only disclosure; cannot derive nullifiers, cannot spend — sdk/src/wallet/auditor.ts)"]
  end
  P -->|"Proofs + plaintext public args\n(shield/unshield: from/to/amount/asset in the clear — see THREAT_MODEL.md Info.1)"| R
  R -->|"Contract calls/reads"| T
  R -->|"Contract calls"| S
  R -->|"Contract calls"| C
  R -->|"Contract calls"| W
  T -->|"NoteCommitmentEvent via RPC getEvents\n(retention-windowed, see Repudiate.2)"| I
  I -->|"Encrypted note history + spent status"| P
  P -->|"Exported viewing key (user's own choice, out of band)"| U
  I -->|"Encrypted note history (ciphertext only)"| U
  L -->|"Execution authorization via RPC"| S
  L -->|"Output liquidity"| A
  OA -->|"Privileged calls via RPC"| G
  OA -.->|"Direct admin call, NOT via governance — the Elevation.1 gap"| C
  OG -->|"cancel_queued(action_id)"| G
```

## Entities and what crosses each boundary

| Entity | What it can affect | What it cannot affect |
|---|---|---|
| **Wallet** | Constructs everything that leaves it; holds the only copy of the spending key. | Cannot be impersonated without the spending key (Spoof.1) — see THREAT_MODEL.md. |
| **Soroban RPC** | Availability of reads/writes; can be slow, down, or (if a specific provider is malicious) lie about simulation results before a real submission — but cannot forge a valid signature or proof. | Cannot authorize a spend or alter accepted on-chain state once a transaction lands. |
| **Indexer** | Availability and completeness of note-recovery for whoever relies on it (Repudiate.2, still open). Stores ciphertext/commitments only — confirmed via `indexer/src/db-sqlite.ts:26`, `db-postgres.ts:42`. | Cannot decrypt note contents (no key material held), cannot forge a spend, cannot alter on-chain state. |
| **Relayer** | Receives a transfer fee or fronts swap output liquidity; decouples the fee-payer identity from the note owner for `transfer`/`transfer4` only. | Cannot redirect swap output (Spoof.3, closed by binding-tag design) or spend a note it doesn't own. |
| **Admin identity** | Every admin-gated entrypoint across all six contracts. On `testnet_final`, this is a bare deployer key for `compliance`/`swap`, and `governance`'s contract address for `token`/`verifier` (per `deployments.json`). | Cannot repoint `verifier`'s admin at all (no such function exists) or `token`'s admin in practice (no governance action calls it) — Elevation.2. |
| **Guardian identity** | Can cancel a queued governance action before it executes. | Cannot execute an action itself, cannot bypass the timelock to act immediately. |
| **Viewing-key holder** | Can decrypt every note the holder's wallet can reach, back to `birthday_ledger`, forward until revoked (and retroactively for anything already decrypted — Info.2). | Cannot derive nullifiers, cannot spend, cannot detect spends (receipts only). |
| **SEP-41 asset issuer** (for a non-native approved asset) | Retains whatever clawback/freeze rights the underlying Stellar Asset Contract grants, including against `token`'s own custodied balance after a deposit — `contracts/token/src/lib.rs:211-225`'s own doc comment calls this "a deliberate acceptance of that issuer's clawback risk." | Has no visibility into or control over note contents, nullifiers, or the shielded pool's internal accounting beyond the balance it can claw back. |

## State stores, explicitly

1. **Contract instance/persistent storage** (six separate contracts, listed in the diagram above) — the only state that determines spendability, supply, and authorization. Subject to Soroban's TTL/archival mechanics; see THREAT_MODEL.md DoS.3 for why this doesn't currently lapse unexpectedly.
2. **Indexer database** (SQLite or PostgreSQL) — a derived, rebuildable cache of on-chain events, holding ciphertext and commitments only. Losing it loses convenience, not funds or secrets — a wallet can in principle re-scan on-chain events directly (Repudiate.2's open question is about whether that fallback actually exists in the SDK today, not whether it's theoretically possible).
3. **Wallet-local storage** (outside this repo's control — however the SDK's consumer persists keys) — the only place spending-key material exists. Not inventoried further here since it's consumer-defined, not part of ZKELLA's own codebase.
