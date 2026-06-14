import { ZKELLAKeys }      from '../keys/keys'
import { IndexerClient }    from '../indexer/client'
import { buildNote, computeCommitment, computeNullifier } from '../notes/builder'
import { encryptNote, tryDecryptNote }                    from '../notes/encrypt'
import { generateShieldProof, ShieldPublicInputs }        from '../prover/shield'
import { Note, WalletConfig, TransferOptions, ViewingKeyExport } from '../types'

function toHex(buf: Uint8Array): string {
  return Array.from(buf).map(b => b.toString(16).padStart(2, '0')).join('')
}

export class ZKELLAWallet {
  private keys:    ZKELLAKeys
  private indexer: IndexerClient
  private notes:   Note[] = []
  private lastSyncLedger = 0
  private config:  WalletConfig

  constructor(config: WalletConfig) {
    this.config  = config
    this.keys    = ZKELLAKeys.fromSpendingKey(config.keys)
    this.indexer = new IndexerClient(config.indexerUrl)
  }

  async sync(): Promise<void> {
    const vk = this.keys.toViewingKey(this.lastSyncLedger)
    let cursor = this.lastSyncLedger

    while (true) {
      const { notes, nextLedger } = await this.indexer.getNotes(cursor)
      if (notes.length === 0) break

      for (const raw of notes) {
        const bundle = Buffer.from(raw.encryptedNote, 'hex')
        const plaintext = tryDecryptNote(bundle, vk.raw)
        if (!plaintext) continue

        const commitment = await computeCommitment(
          plaintext.value,
          plaintext.assetId,
          plaintext.rho,
          plaintext.rcm,
        )
        const expectedHex = toHex(commitment)
        if (expectedHex !== raw.commitment) continue

        this.notes.push({
          ...plaintext,
          leafIndex:  raw.leafIndex,
          commitment,
        })
      }
      cursor = nextLedger
    }

    // Filter spent notes
    const nfMap: Record<string, number> = {}
    const nullifiers: string[] = []
    for (let i = 0; i < this.notes.length; i++) {
      const nf = await computeNullifier(this.config.keys.nullifierKey, this.notes[i].rho)
      const hex = toHex(nf as unknown as Uint8Array)
      nullifiers.push(hex)
      nfMap[hex] = i
    }

    const spent = await this.indexer.batchCheckNullifiers(nullifiers)
    this.notes = this.notes.filter((_, i) => !spent[nullifiers[i]])
    this.lastSyncLedger = cursor
  }

  async balance(asset: string): Promise<{ shielded: bigint }> {
    const total = this.notes
      .filter(n => n.assetId === asset)
      .reduce((sum, n) => sum + n.value, 0n)
    return { shielded: total }
  }

  /**
   * Shield (deposit) public funds into the shielded pool.
   *
   * Steps:
   *  1. Build a fresh note with cryptographic randomness.
   *  2. Compute the note commitment (Poseidon2 tree).
   *  3. Encrypt the note to the recipient's transmission key (self by default).
   *  4. Generate a Groth16 shield proof (stub for 20%; real in M2).
   *  5. Return a `submit()` thunk that broadcasts the Soroban tx.
   *
   * @param opts.asset   SEP-41 contract address of the asset being shielded.
   * @param opts.amount  Amount in base units (u64).
   * @param opts.to      Optional recipient shielded address (defaults to self).
   */
  async shield(opts: {
    asset:  string
    amount: bigint
    to?:    string
  }): Promise<{ note: Note; submit: () => Promise<{ leafIndex: number }> }> {
    const { asset, amount } = opts

    // 1. Build note
    const note = await buildNote(amount, asset)

    // 2. Derive recipient transmission key (self by default)
    const transmissionKey = this.config.keys.transmissionKey

    // 3. Encrypt note
    const encryptedBundle = encryptNote(note, transmissionKey)
    const encryptedHex    = toHex(encryptedBundle)
    const commitmentHex   = toHex(note.commitment)

    // 4. Build public inputs for the proof
    const publicInputs: ShieldPublicInputs = {
      commitment: commitmentHex,
      asset,
      amount,
    }

    // 5. Generate proof (snarkjs stub)
    const { proof, publicSignals } = await generateShieldProof(note, publicInputs)

    // 6. Return submit thunk — broadcasts Soroban tx
    const submit = async (): Promise<{ leafIndex: number }> => {
      const result = await this.submitShieldTx({
        commitmentHex,
        encryptedHex,
        proof,
        publicSignals,
        asset,
        amount,
      })
      note.leafIndex = result.leafIndex
      this.notes.push(note)
      return result
    }

    return { note, submit }
  }

  async transfer(_opts: TransferOptions): Promise<{ submit: () => Promise<void> }> {
    // Note selection + Groth16 proof + Soroban tx — M2
    return { submit: async () => {} }
  }

  async unshield(_opts: { asset: string; amount: bigint; to: string }): Promise<{ submit: () => Promise<void> }> {
    // Unshield proof + Soroban tx — M2
    return { submit: async () => {} }
  }

  exportViewingKey(): ViewingKeyExport {
    return this.keys.exportViewingKey(this.lastSyncLedger, this.config.network)
  }

  // ── Soroban transaction builder ───────────────────────────────────────────────

  /**
   * Construct and submit a Soroban `shield` invocation.
   * In M1 this calls the ct20 contract's `shield(commitment, encrypted_note, amount, asset)`.
   * Full Soroban SDK wiring (signing, fee bump, ledger polling) is deferred to M2.
   */
  private async submitShieldTx(params: {
    commitmentHex:  string
    encryptedHex:   string
    proof:          Uint8Array
    publicSignals:  string[]
    asset:          string
    amount:         bigint
  }): Promise<{ leafIndex: number }> {
    // TODO(M2): replace with @stellar/stellar-sdk SorobanServer call
    //   const server = new SorobanServer(this.config.sorobanRpc)
    //   const tx = buildShieldTx(server, this.config.ct20Address, params)
    //   const result = await server.sendTransaction(tx)
    //   return { leafIndex: result.leafIndex }

    // Stub: simulate a broadcast by posting to the Soroban RPC endpoint format
    const body = {
      jsonrpc: '2.0',
      id:      1,
      method:  'sendTransaction',
      params:  {
        transaction: buildShieldXdr(params),
      },
    }

    const res = await fetch(`${this.config.sorobanRpc}`, {
      method:  'POST',
      headers: { 'Content-Type': 'application/json' },
      body:    JSON.stringify(body),
    })

    if (!res.ok) throw new Error(`Soroban RPC error: ${res.status}`)
    const data = await res.json()

    if (data.error) throw new Error(`shield tx failed: ${data.error.message}`)

    // TODO(M2): extract the leaf index from the contract's return value in the
    // transaction result XDR — `soroban_sdk::contractimpl` returns it as ScVal.
    //
    // The leaf index MUST come from the tx result, not from a subsequent
    // getMerkleRoot() poll. The poll is racy: another shield could have been
    // included between our tx and the read, giving us the wrong index.
    //
    // Real flow:
    //   const txResult = await server.getTransaction(data.result.hash)
    //   const leafIndex = scValToNative(txResult.returnValue) as number
    //   return { leafIndex }
    //
    // Stub: parse the leaf index from the simulated RPC response if present,
    // otherwise fall back to reading the commitment's actual tree position via
    // the indexer's commitment-lookup endpoint (deterministic, not racy).
    if (data.result?.leafIndex !== undefined) {
      return { leafIndex: data.result.leafIndex as number }
    }
    // Fallback for stub: the commitment uniquely identifies the leaf, so
    // querying by commitment hash is race-free (unlike querying leafCount - 1).
    // TODO(M2): replace with txResult.returnValue parse.
    const { leafIndex } = await this.indexer.getLeafByCommitment(params.commitmentHex)
    return { leafIndex }
  }
}

// ── XDR builder stub ──────────────────────────────────────────────────────────

/**
 * Serialize shield call arguments into a base64-encoded Soroban XDR envelope.
 * Real implementation uses @stellar/stellar-sdk's contract.call() builder.
 * This stub produces a recognizable placeholder that tests can assert on.
 */
function buildShieldXdr(params: {
  commitmentHex: string
  encryptedHex:  string
  proof:         Uint8Array
  publicSignals: string[]
  asset:         string
  amount:        bigint
}): string {
  const payload = JSON.stringify({
    fn:         'shield',
    commitment: params.commitmentHex,
    asset:      params.asset,
    amount:     params.amount.toString(),
    // proof omitted from stub XDR (verified on-chain via verifying key)
  })
  return Buffer.from(payload).toString('base64')
}
