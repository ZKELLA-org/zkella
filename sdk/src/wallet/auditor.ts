import { IndexerClient } from '../indexer/client'
import { ViewingKeyExport, Note } from '../types'

export class ZKELLAAuditor {
  private indexer: IndexerClient
  private vkExport: ViewingKeyExport
  private notes: Note[] = []

  constructor(config: { viewingKeyExport: ViewingKeyExport; indexerUrl: string }) {
    this.vkExport = config.viewingKeyExport
    this.indexer  = new IndexerClient(config.indexerUrl)
  }

  async sync(): Promise<void> {
    let cursor = this.vkExport.birthday_ledger

    while (true) {
      const { notes, nextLedger } = await this.indexer.getNotes(cursor)
      if (notes.length === 0) break

      for (const raw of notes) {
        const plaintext = this.tryDecryptWithViewingKey(raw.encryptedNote)
        if (!plaintext) continue
        this.notes.push(plaintext)
      }
      cursor = nextLedger
    }
  }

  async transactionHistory(asset: string): Promise<Array<{
    type:   'receive' | 'spend'
    amount: bigint
    ledger: number
  }>> {
    return this.notes
      .filter(n => n.assetId === asset)
      .map(n => ({ type: 'receive' as const, amount: n.value, ledger: n.leafIndex }))
  }

  private tryDecryptWithViewingKey(_encryptedNote: string): Note | null {
    // Decrypt using viewing key — M2
    return null
  }
}
