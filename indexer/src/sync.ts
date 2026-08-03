// Polls Stellar RPC's `getEvents` for `ct20`'s `("zkella","note")` and
// `("zkella","nf")` contract events and persists them — the whole reason
// this service exists: Stellar RPC only retains events for a short window
// (a few days), so anything a wallet needs to recover note history or
// nullifier-spent status beyond that window has to live somewhere durable.

import { SorobanRpc, scValToNative, xdr } from '@stellar/stellar-sdk'
import { IndexerDb } from './db.ts'

function toHex(buf: Uint8Array): string {
  return Array.from(buf).map(b => b.toString(16).padStart(2, '0')).join('')
}

const TOPIC_ZKELLA = xdr.ScVal.scvSymbol('zkella').toXDR('base64')
const TOPIC_NOTE   = xdr.ScVal.scvSymbol('note').toXDR('base64')
const TOPIC_NF     = xdr.ScVal.scvSymbol('nf').toXDR('base64')

export interface SyncConfig {
  rpcUrl:      string
  ct20Address: string
  db:          IndexerDb
  startLedger: number
  pollIntervalMs: number
}

export class Syncer {
  private server: SorobanRpc.Server
  private stopped = false
  private config: SyncConfig

  constructor(config: SyncConfig) {
    this.config = config
    this.server = new SorobanRpc.Server(config.rpcUrl)
  }

  stop(): void {
    this.stopped = true
  }

  /** Runs forever (until `stop()`), polling for new events and persisting them. */
  async run(): Promise<void> {
    while (!this.stopped) {
      try {
        await this.syncOnce()
      } catch (err) {
        // A single failed poll shouldn't kill the service — log and retry
        // next tick. Real deployments should alert on repeated failures
        // (see docs/POC_IMPLEMENTATION.md's operational-readiness notes);
        // this reference implementation just keeps trying.
        console.error('[indexer] sync error:', err)
      }
      await sleep(this.config.pollIntervalMs)
    }
  }

  private async syncOnce(): Promise<void> {
    let cursor = this.config.db.getLastSyncedLedger(this.config.startLedger)

    // Drain every page of events starting at `cursor` before sleeping again
    // — `getEvents` pages by event count (`limit`), not ledger range, so a
    // burst of activity needs multiple calls to fully catch up.
    for (;;) {
      const response = await this.server.getEvents({
        startLedger: cursor,
        filters: [
          { type: 'contract', contractIds: [this.config.ct20Address], topics: [[TOPIC_ZKELLA, TOPIC_NOTE]] },
          { type: 'contract', contractIds: [this.config.ct20Address], topics: [[TOPIC_ZKELLA, TOPIC_NF]] },
        ],
        limit: 1000,
      })

      for (const event of response.events) {
        const topic1 = event.topic[1] ? scValToNative(event.topic[1]) : undefined
        const value = scValToNative(event.value) as Record<string, unknown>

        if (topic1 === 'note') {
          const leafIndex     = Number(value.leaf_index)
          const commitment    = toHex(value.commitment as Uint8Array)
          const encryptedNote = toHex(value.encrypted_note as Uint8Array)
          this.config.db.upsertNote({ leafIndex, commitment, encryptedNote, ledger: event.ledger })
        } else if (topic1 === 'nf') {
          const nullifier = toHex(value.nullifier as Uint8Array)
          this.config.db.markNullifierSpent(nullifier, event.ledger)
        }
      }

      // response.latestLedger is the RPC node's own current tip — once our
      // cursor reaches it, we're caught up for this tick.
      if (response.events.length === 0) {
        cursor = response.latestLedger + 1
        this.config.db.setLastSyncedLedger(cursor)
        break
      }

      const lastEventLedger = response.events[response.events.length - 1].ledger
      cursor = lastEventLedger + 1
      this.config.db.setLastSyncedLedger(cursor)

      if (response.events.length < 1000 && cursor > response.latestLedger) break
    }
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms))
}
