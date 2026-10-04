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

  constructor(config: { viewingKeyExport: ViewingKeyExport; indexerUrl: string }) {
    this.vkExport = config.viewingKeyExport
    this.indexer  = new IndexerClient(config.indexerUrl)
  }

  async sync(): Promise<void> {
    let cursor = this.vkExport.birthday_ledger

    while (true) {
      const { notes, nextLedger } = await this.indexer.getNotes(cursor)
      for (const raw of notes) {
        const note = await decryptNoteForAuditor(this.vkExport.viewing_key, raw)
        if (note) this.notes.push(note)
      }
      if (notes.length === 0 || nextLedger <= cursor) break
      cursor = nextLedger
    }
  }

  /**
   * Receipts visible to this viewing key. Spends are not reported: they need a
   * nullifier, which is derived from the nullifier key, not the viewing key.
   */
  transactionHistory(asset: string): Array<{ type: 'receive'; amount: bigint; ledger: number }> {
    return this.notes
      .filter(n => n.assetId === asset)
      .map(n => ({ type: 'receive' as const, amount: n.value, ledger: n.ledger }))
  }
}
