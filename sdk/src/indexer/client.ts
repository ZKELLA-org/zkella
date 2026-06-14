export interface IndexerNote {
  leafIndex:     number
  commitment:    string  // hex
  encryptedNote: string  // hex
  ledger:        number
}

export interface MerklePath {
  path:  string[]  // hex[32]
  index: number[]  // 0|1[32]
  root:  string    // hex
}

export class IndexerClient {
  constructor(private readonly baseUrl: string) {}

  async getNotes(fromLedger: number, limit = 500): Promise<{
    notes: IndexerNote[]
    nextLedger: number
  }> {
    const res = await fetch(
      `${this.baseUrl}/notes?from_ledger=${fromLedger}&limit=${limit}`
    )
    if (!res.ok) throw new Error(`indexer error: ${res.status}`)
    return res.json()
  }

  async getMerklePath(leafIndex: number): Promise<MerklePath> {
    const res = await fetch(`${this.baseUrl}/merkle/path/${leafIndex}`)
    if (!res.ok) throw new Error(`indexer error: ${res.status}`)
    return res.json()
  }

  async getMerkleRoot(): Promise<{ root: string; leafCount: number }> {
    const res = await fetch(`${this.baseUrl}/merkle/root`)
    if (!res.ok) throw new Error(`indexer error: ${res.status}`)
    return res.json()
  }

  async batchCheckNullifiers(
    nullifiers: string[]
  ): Promise<Record<string, boolean>> {
    const res = await fetch(`${this.baseUrl}/nullifiers/batch`, {
      method:  'POST',
      headers: { 'Content-Type': 'application/json' },
      body:    JSON.stringify({ nullifiers }),
    })
    if (!res.ok) throw new Error(`indexer error: ${res.status}`)
    const data = await res.json()
    return data.spent
  }

  /**
   * Look up the leaf index for a specific commitment hash.
   * Race-free alternative to reading leafCount - 1 after submission,
   * since a commitment uniquely identifies exactly one leaf.
   */
  async getLeafByCommitment(commitmentHex: string): Promise<{ leafIndex: number }> {
    const res = await fetch(`${this.baseUrl}/commitment/${commitmentHex}`)
    if (!res.ok) throw new Error(`indexer error: ${res.status}`)
    return res.json()
  }

  async health(): Promise<{ syncedLedger: number; tipLedger: number; lag: number }> {
    const res = await fetch(`${this.baseUrl}/health`)
    if (!res.ok) throw new Error(`indexer error: ${res.status}`)
    return res.json()
  }
}
