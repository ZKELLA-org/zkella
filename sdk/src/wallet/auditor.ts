import { IndexerClient, IndexerNote } from '../indexer/client'
import { tryDecryptNote } from '../notes/encrypt'
import { ViewingKeyExport } from '../types'

/**
 * A note recovered with a viewing key. Carries only what the viewing key can
 * reveal: the plaintext value and the commitment-opening secrets, plus the
 * public leaf position. The owner key is absent because computing it needs the
 * nullifier key, which a viewing key does not hold.
 */
export interface AuditedNote {
  value:      bigint
  assetId:    string
  rho:        Uint8Array
  rcm:        Uint8Array
  leafIndex:  number
  commitment: string
  ledger:     number
}

/**
 * Decrypts one indexer record with a viewing key (hex). Returns null when the
 * record is not encrypted to this key — the MAC check fails for any other key.
 */
export async function decryptNoteForAuditor(
  viewingKeyHex: string,
  raw:           IndexerNote,
): Promise<AuditedNote | null> {
  const plaintext = await tryDecryptNote(
    Buffer.from(raw.encryptedNote, 'hex'),
    Buffer.from(viewingKeyHex, 'hex'),
  )
  if (!plaintext) return null
  return {
    value:      plaintext.value,
    assetId:    plaintext.assetId,
    rho:        plaintext.rho,
    rcm:        plaintext.rcm,
    leafIndex:  raw.leafIndex,
    commitment: raw.commitment,
    ledger:     raw.ledger,
  }
}

export class ZKELLAAuditor {
  private indexer: IndexerClient
  private vkExport: ViewingKeyExport
  private notes: AuditedNote[] = []
  private lastSyncLedger: number

  constructor(config: { viewingKeyExport: ViewingKeyExport; indexerUrl: string }) {
    this.vkExport = config.viewingKeyExport
    this.indexer  = new IndexerClient(config.indexerUrl)
    this.lastSyncLedger = config.viewingKeyExport.birthday_ledger
  }

  /**
   * Resumes from `lastSyncLedger`, not the viewing key's birthday — calling
   * `sync()` again (the normal way an auditor keeps `transactionHistory`
   * current as new notes land) must pick up only what's new. Starting over
   * from the birthday every time would re-decrypt and re-push every note
   * already in `this.notes`, silently doubling (tripling, ...) every
   * reported receipt on each additional call — exactly the kind of drift a
   * compliance report can't afford. The commitment check mirrors
   * `ZKELLAWallet.sync()`'s own dedup, so a page the indexer re-serves (e.g.
   * a resumed sync starting at a ledger already covered) is also harmless.
   */
  async sync(): Promise<void> {
    let cursor = this.lastSyncLedger
    const seen = new Set(this.notes.map(n => n.commitment))

    while (true) {
      const { notes, nextLedger } = await this.indexer.getNotes(cursor)
      for (const raw of notes) {
        if (seen.has(raw.commitment)) continue
        const note = await decryptNoteForAuditor(this.vkExport.viewing_key, raw)
        if (note) {
          seen.add(raw.commitment)
          this.notes.push(note)
        }
      }
      if (notes.length === 0 || nextLedger <= cursor) break
      cursor = nextLedger
    }
    this.lastSyncLedger = cursor
  }

  /**
   * Receipts visible to this viewing key. Spends are not reported: they need a
   * nullifier, which is derived from the nullifier key, not the viewing key.
   * Zero-value notes are padding outputs of a transfer or unshield, not receipts.
   */
  transactionHistory(asset: string): Array<{ type: 'receive'; amount: bigint; ledger: number }> {
    return this.notes
      .filter(n => n.assetId === asset && n.value > 0n)
      .map(n => ({ type: 'receive' as const, amount: n.value, ledger: n.ledger }))
  }
}
