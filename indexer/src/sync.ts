// Polls Stellar RPC's `getEvents` for `token`'s `("zkella","note")` and
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
  tokenAddress: string
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
    const ledgerCursor = this.config.db.getLastSyncedLedger(this.config.startLedger)
    const filters = [
      { type: 'contract' as const, contractIds: [this.config.tokenAddress], topics: [[TOPIC_ZKELLA, TOPIC_NOTE]] },
      { type: 'contract' as const, contractIds: [this.config.tokenAddress], topics: [[TOPIC_ZKELLA, TOPIC_NF]] },
    ]
    // `getEvents`'s own cursor, distinct from the ledger-number cursor
    // persisted to the db — see below for why both are needed.
    let pagingToken: string | undefined

    // Drain every page of events starting at `ledgerCursor` before sleeping
    // again — `getEvents` pages by event count (`limit`), not ledger range,
    // so a burst of activity needs multiple calls to fully catch up.
    for (;;) {
      // Once this loop has paged at least once, resume via the last event's
      // own `pagingToken` rather than recomputing `startLedger =
      // lastEventLedger + 1`. Using a ledger-number cursor for *every* page
      // (the original approach here) silently skips any events still
      // remaining in that same ledger once a single ledger has more
      // matching events than `limit` — unlikely at today's usage (ZKELLA
      // emits at most a handful of these events per transaction), but a
      // real, permanent, silent data-loss bug under sustained high load,
      // since a skipped ledger's remaining events are never fetched again.
      const response = pagingToken !== undefined
        ? await this.server.getEvents({ cursor: pagingToken, filters, limit: 1000 })
        : await this.server.getEvents({ startLedger: ledgerCursor, filters, limit: 1000 })

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
        this.config.db.setLastSyncedLedger(response.latestLedger + 1)
        break
      }

      const lastEvent = response.events[response.events.length - 1]
      pagingToken = lastEvent.pagingToken
      // The db's persisted cursor stays ledger-based (not the RPC paging
      // token) — safe across a restart because re-fetching from the start
      // of a ledger already partially processed just re-upserts the same
      // rows (`upsertNote`/`markNullifierSpent` are idempotent).
      this.config.db.setLastSyncedLedger(lastEvent.ledger + 1)

      if (response.events.length < 1000) break
    }
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms))
}
