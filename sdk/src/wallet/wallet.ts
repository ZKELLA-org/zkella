import {
  Contract, Keypair, Networks, SorobanRpc,
  TransactionBuilder, nativeToScVal, scValToNative, xdr,
} from '@stellar/stellar-sdk'
import { ZKELLAKeys }      from '../keys/keys'
import { IndexerClient }    from '../indexer/client'
import { buildNote, computeCommitment, computeNullifier } from '../notes/builder'
import { encryptNote, tryDecryptNote }                    from '../notes/encrypt'
import { generateShieldProof, ShieldPublicInputs }        from '../prover/shield'
import { generateTransferProof, TransferInputNote }       from '../prover/transfer'
import { generateUnshieldProof }                          from '../prover/unshield'
import { Note, WalletConfig, TransferOptions, ViewingKeyExport } from '../types'

// transfer4() (4-in-4-out) isn't wired into the wallet yet — 2-in-2-out
// transfer() covers the common case, and transfer4's only real use (note
// consolidation across 4 inputs at once) doesn't have wallet-side selection
// logic built for it yet. `sdk/src/prover/transfer4.ts` is ready whenever
// that lands; the pattern is identical to transfer() below, just arity 4.

const MERKLE_DEPTH = 32

function toHex(buf: Uint8Array): string {
  return Array.from(buf).map(b => b.toString(16).padStart(2, '0')).join('')
}

function hexToBytes(hex: string): Uint8Array {
  const buf = new Uint8Array(hex.length / 2)
  for (let i = 0; i < buf.length; i++) buf[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16)
  return buf
}

export class ZKELLAWallet {
  private keys:      ZKELLAKeys
  private indexer:   IndexerClient
  private notes:     Note[] = []
  private lastSyncLedger = 0
  private config:    WalletConfig
  private sourceKeypair: Keypair

  constructor(config: WalletConfig) {
    this.config        = config
    this.keys          = ZKELLAKeys.fromSpendingKey(config.keys)
    this.indexer       = new IndexerClient(config.indexerUrl)
    this.sourceKeypair = Keypair.fromSecret(config.stellarSecret)
  }

  async sync(): Promise<void> {
    const vk = this.keys.toViewingKey(this.lastSyncLedger)
    let cursor = this.lastSyncLedger

    while (true) {
      const { notes, nextLedger } = await this.indexer.getNotes(cursor)
      if (notes.length === 0) break

      for (const raw of notes) {
        const bundle = Buffer.from(raw.encryptedNote, 'hex')
        const plaintext = await tryDecryptNote(bundle, vk.raw)
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
   *  4. Generate a real Groth16 shield proof.
   *  5. Return a `submit()` thunk that broadcasts the real Soroban tx.
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
    requireCircuit(this.config.shieldCircuit, 'shieldCircuit', 'shield()')

    const note = await buildNote(amount, asset)
    const transmissionKey = this.config.keys.transmissionKey
    const encryptedBundle = await encryptNote(note, transmissionKey)

    const publicInputs: ShieldPublicInputs = { commitment: note.commitment, asset, amount }
    const { proof, valueCommit } = await generateShieldProof(
      note, publicInputs,
      this.config.shieldCircuit!.wasmPath, this.config.shieldCircuit!.zkeyPath,
    )

    const submit = async (): Promise<{ leafIndex: number }> => {
      const shieldPub = {
        commitment:   note.commitment,
        pub_asset_id: asset,
        pub_value:    amount,
        value_commit: valueCommit,
      }
      const result = await this.submitContractCall(this.config.ct20Address, 'shield', [
        nativeToScVal(this.sourceKeypair.publicKey(), { type: 'address' }),
        nativeToScVal(asset,                           { type: 'address' }),
        nativeToScVal(amount,                          { type: 'i128' }),
        nativeToScVal(note.rho,                        { type: 'bytes' }),
        nativeToScVal(note.rcm,                        { type: 'bytes' }),
        nativeToScVal(note.commitment,                 { type: 'bytes' }),
        nativeToScVal(encryptedBundle,                 { type: 'bytes' }),
        nativeToScVal(proof,                           { type: 'bytes' }),
        structScVal(shieldPub, {
          commitment: 'bytes', pub_asset_id: 'address', pub_value: 'i128', value_commit: 'bytes',
        }),
      ])
      const leafIndex = scValToNative(result) as number
      note.leafIndex = leafIndex
      this.notes.push(note)
      return { leafIndex }
    }

    return { note, submit }
  }

  /**
   * Transfer shielded value to a recipient, privately.
   *
   * ZKELLA's transfer circuits always take exactly 2 (or 4, via
   * `transfer4()`) real, already-shielded input notes — there's no "dummy
   * input" flag the way some UTXO-shielded-pool designs have, so a wallet
   * needs at least 2 spendable notes of the target asset to call this at
   * all. This picks the two largest unspent notes (simple, not
   * fee-optimal, coin selection), sends `opts.amount` to `opts.to`, and
   * returns any leftover as a fresh change note back to this wallet.
   *
   * `opts.to` is a raw hex-encoded transmission key (32-byte compressed
   * BN254 G1 point) for now — full `zkella1...` diversified-address
   * decoding (base58 + `ShieldedAddress` unpacking) is a separate,
   * still-open piece of wallet UX, not a protocol gap (the underlying ECDH
   * already supports diversified addresses — see `sdk/src/notes/encrypt.ts`'s
   * `basePoint` parameter and `ZKELLAKeys.deriveAddress`).
   */
  async transfer(opts: TransferOptions): Promise<{ submit: () => Promise<{ leafIndices: number[] }> }> {
    const { to, asset, amount } = opts
    requireCircuit(this.config.transferCircuit, 'transferCircuit', 'transfer()')

    const candidates = this.notes
      .filter(n => n.assetId === asset)
      .sort((a, b) => (b.value > a.value ? 1 : -1))

    if (candidates.length < 2) {
      throw new Error(
        `transfer() needs at least 2 spendable notes of ${asset}; found ${candidates.length}. ` +
        `ZKELLA's transfer circuit has no dummy-input support, so a single note can only be ` +
        `spent via unshield() (fully) today.`
      )
    }
    const [inA, inB] = candidates
    const fee = 0n
    const sumIn = inA.value + inB.value
    if (sumIn < amount + fee) {
      throw new Error(`transfer() insufficient balance: have ${sumIn}, need ${amount + fee}`)
    }
    const changeAmount = sumIn - amount - fee

    const anchor = await this.getMerkleRoot()
    const [pathA, pathB] = await Promise.all([
      this.getMerklePathBytes(inA.leafIndex),
      this.getMerklePathBytes(inB.leafIndex),
    ])

    const recipientTk = hexToBytes(to)
    const changeTk    = this.config.keys.transmissionKey

    const inputs: [TransferInputNote, TransferInputNote] = [
      { note: inA, merklePath: pathA },
      { note: inB, merklePath: pathB },
    ]

    const result = await generateTransferProof(
      { inputs, nk: this.config.keys.nullifierKey,
        outputs: [{ value: amount, assetId: asset }, { value: changeAmount, assetId: asset }],
        fee },
      { anchor, assetId: asset },
      this.config.transferCircuit!.wasmPath, this.config.transferCircuit!.zkeyPath,
    )

    const [outToRecipient, outChange] = result.outputNotes
    const [encryptedToRecipient, encryptedChange] = await Promise.all([
      encryptNote(outToRecipient, recipientTk),
      encryptNote(outChange, changeTk),
    ])

    const submit = async (): Promise<{ leafIndices: number[] }> => {
      const pubInputs = {
        anchor,
        nullifiers:        result.nullifiers,
        out_commitments:   result.outputNotes.map(n => n.commitment),
        in_value_commits:  result.inValueCommits,
        out_value_commits: result.outValueCommits,
        fee,
        asset_id: asset,
      }
      const returned = await this.submitContractCall(this.config.ct20Address, 'transfer', [
        vecScVal(result.nullifiers, 'bytes'),
        vecScVal(result.outputNotes.map(n => n.commitment), 'bytes'),
        vecScVal([encryptedToRecipient, encryptedChange], 'bytes'),
        nativeToScVal(result.proof, { type: 'bytes' }),
        structScVal(pubInputs, {
          anchor: 'bytes', nullifiers: 'vec-bytes', out_commitments: 'vec-bytes',
          in_value_commits: 'vec-bytes', out_value_commits: 'vec-bytes', fee: 'i128', asset_id: 'address',
        }),
      ])
      const leafIndices = scValToNative(returned) as number[]
      outToRecipient.leafIndex = leafIndices[0]
      outChange.leafIndex      = leafIndices[1]
      this.notes = this.notes.filter(n => n !== inA && n !== inB)
      this.notes.push(outChange)
      return { leafIndices }
    }

    return { submit }
  }

  /**
   * Unshield (withdraw) a single note's full value to a public Stellar
   * address. Partial-amount unshielding requires a preceding `transfer()`
   * to split the note into the exact amount first — there's no
   * unshield-with-change entry point on `ct20` today.
   */
  async unshield(opts: { asset: string; amount: bigint; to: string }): Promise<{ submit: () => Promise<void> }> {
    requireCircuit(this.config.unshieldCircuit, 'unshieldCircuit', 'unshield()')

    const note = this.notes.find(n => n.assetId === opts.asset && n.value === opts.amount)
    if (!note) {
      throw new Error(
        `unshield() found no note of ${opts.asset} worth exactly ${opts.amount}. ` +
        `Use transfer() to split an existing note into the exact amount first.`
      )
    }

    const anchor = await this.getMerkleRoot()
    const merklePath = await this.getMerklePathBytes(note.leafIndex)

    const result = await generateUnshieldProof(
      { note, nk: this.config.keys.nullifierKey, merklePath },
      { anchor, recipient: opts.to },
      this.config.unshieldCircuit!.wasmPath, this.config.unshieldCircuit!.zkeyPath,
    )

    const submit = async (): Promise<void> => {
      const pubInputs = {
        anchor,
        nullifier:      result.nullifier,
        pub_value:      note.value,
        pub_asset_id:   opts.asset,
        recipient_hash: result.recipientHash,
      }
      await this.submitContractCall(this.config.ct20Address, 'unshield', [
        nativeToScVal(result.nullifier, { type: 'bytes' }),
        nativeToScVal(opts.to,          { type: 'address' }),
        nativeToScVal(result.proof,     { type: 'bytes' }),
        structScVal(pubInputs, {
          anchor: 'bytes', nullifier: 'bytes', pub_value: 'i128',
          pub_asset_id: 'address', recipient_hash: 'bytes',
        }),
      ])
      this.notes = this.notes.filter(n => n !== note)
    }

    return { submit }
  }

  exportViewingKey(): ViewingKeyExport {
    return this.keys.exportViewingKey(this.lastSyncLedger, this.config.network)
  }

  // ── Soroban RPC ──────────────────────────────────────────────────────────────

  private getServer(): SorobanRpc.Server {
    return new SorobanRpc.Server(this.config.sorobanRpc)
  }

  private getNetworkPassphrase(): string {
    return this.config.network === 'mainnet' ? Networks.PUBLIC : Networks.TESTNET
  }

  /** Read-only contract call: simulate only, never submitted or signed for. */
  private async callView(contractId: string, method: string, args: xdr.ScVal[]): Promise<unknown> {
    const server  = this.getServer()
    const account = await server.getAccount(this.sourceKeypair.publicKey())
    const tx = new TransactionBuilder(account, { fee: '100', networkPassphrase: this.getNetworkPassphrase() })
      .addOperation(new Contract(contractId).call(method, ...args))
      .setTimeout(10)
      .build()

    const sim = await server.simulateTransaction(tx)
    if (SorobanRpc.Api.isSimulationError(sim)) {
      throw new Error(`${method} simulation error: ${sim.error}`)
    }
    return scValToNative((sim as SorobanRpc.Api.SimulateTransactionSuccessResponse).result!.retval)
  }

  /**
   * Build, sign, submit, and poll a real Soroban contract invocation to
   * completion, returning the contract function's raw return value
   * (`ScVal`, not yet converted to a JS value — callers pass it to
   * `scValToNative` themselves since the target shape differs per call).
   *
   * The leaf index (or similar) is read from the transaction's actual
   * return value, not from a subsequent state read — a subsequent
   * `merkle_root()`/`leaf_count()` poll is racy (another shield/transfer
   * could land between our tx and the read), the return value isn't.
   */
  private async submitContractCall(
    contractId: string,
    method:     string,
    args:       xdr.ScVal[],
  ): Promise<xdr.ScVal> {
    const server  = this.getServer()
    const account = await server.getAccount(this.sourceKeypair.publicKey())

    const tx = new TransactionBuilder(account, { fee: '10000000', networkPassphrase: this.getNetworkPassphrase() })
      .addOperation(new Contract(contractId).call(method, ...args))
      .setTimeout(30)
      .build()

    const prepared = await server.prepareTransaction(tx)
    prepared.sign(this.sourceKeypair)

    const response = await server.sendTransaction(prepared)
    if (response.status === 'ERROR') {
      throw new Error(`${method} submission error: ${JSON.stringify(response.errorResult)}`)
    }

    let result: SorobanRpc.Api.GetTransactionResponse | null = null
    const deadline = Date.now() + 60_000
    while (Date.now() < deadline) {
      await sleep(2000)
      const r = await server.getTransaction(response.hash)
      if (r.status !== 'NOT_FOUND') { result = r; break }
    }
    if (!result) throw new Error(`${method} tx ${response.hash} not confirmed within timeout`)
    if (result.status !== 'SUCCESS') {
      throw new Error(`${method} tx ${response.hash} failed: ${result.status}`)
    }
    return result.returnValue!
  }

  private async getMerkleRoot(): Promise<Uint8Array> {
    const root = await this.callView(this.config.ct20Address, 'merkle_root', [])
    return root as Uint8Array
  }

  private async getMerklePathBytes(leafIndex: number): Promise<Uint8Array[]> {
    const path = await this.callView(this.config.ct20Address, 'merkle_path', [
      nativeToScVal(leafIndex, { type: 'u32' }),
    ]) as Uint8Array[]
    if (path.length !== MERKLE_DEPTH) {
      throw new Error(`merkle_path(${leafIndex}) returned ${path.length} entries, expected ${MERKLE_DEPTH}`)
    }
    return path;
  }
}

// ── ScVal helpers ────────────────────────────────────────────────────────────

type FieldKind = 'bytes' | 'address' | 'i128' | 'vec-bytes'

/**
 * Build a `#[contracttype] struct`'s ScVal encoding: a map keyed by field
 * symbol. Soroban structs are encoded as maps sorted by field name — the
 * SDK's `nativeToScVal` handles the sort automatically as long as every
 * field's target type is given explicitly (untyped `{type:'map'}` guesses
 * from the JS value's own shape, which doesn't reliably match a specific
 * contract struct's field types, e.g. an `Address` string vs. a `Bytes`
 * hex string are both plain strings in JS).
 */
function structScVal(obj: Record<string, unknown>, fields: Record<string, FieldKind>): xdr.ScVal {
  // Soroban structs are encoded as ScVal maps sorted by field-name symbol.
  // Built by hand (rather than nativeToScVal's own struct type-hint shape)
  // because `vec-bytes` fields (Vec<BytesN<32>>) aren't expressible in that
  // shape — `nativeToScVal`'s per-field hints only cover scalar leaf types.
  const entries = Object.entries(fields).map(([key, kind]) => {
    const value = obj[key]
    const scVal = kind === 'vec-bytes'
      ? vecScVal(value as Uint8Array[], 'bytes')
      : nativeToScVal(value, { type: kind })
    return new xdr.ScMapEntry({ key: xdr.ScVal.scvSymbol(key), val: scVal })
  })
  entries.sort((a, b) => a.key().sym().toString().localeCompare(b.key().sym().toString()))
  return xdr.ScVal.scvMap(entries)
}

function vecScVal(items: Uint8Array[], kind: 'bytes'): xdr.ScVal {
  return xdr.ScVal.scvVec(items.map(item => nativeToScVal(item, { type: kind })))
}

function requireCircuit(
  circuit: { wasmPath: string; zkeyPath: string } | undefined,
  configKey: string,
  method: string,
): void {
  if (!circuit) {
    throw new Error(`ZKELLAWallet.${method} requires config.${configKey} (wasmPath/zkeyPath)`)
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms))
}
