# ZKELLA Indexer (reference implementation)

Persists `ct20`'s `("zkella","note")` and `("zkella","nf")` contract events
past Stellar RPC's own short retention window, and serves them over HTTP in
exactly the shape `sdk/src/indexer/client.ts`'s `IndexerClient` expects.
`merkle_root`/`merkle_path` are proxied live to `ct20` itself rather than
duplicated — the contract is already the source of truth for current tree
state.

## Running

```sh
CT20_CONTRACT_ID=C...        \
SOROBAN_RPC_URL=https://soroban-testnet.stellar.org \
ZKELLA_NETWORK=testnet       \
INDEXER_START_LEDGER=<ct20's deploy ledger> \
npm run indexer
```

Uses Node's built-in `node:sqlite` (experimental, Node 22.5+) for storage —
no external database dependency — and runs directly via
`node --experimental-strip-types`, no build step required.

Optional env vars: `INDEXER_DB_PATH` (default `./indexer.db`),
`INDEXER_HTTP_PORT` (default `8787`), `INDEXER_POLL_MS` (default `5000`).

## Status

Validated against live Stellar Testnet: a real `shield()` transaction's
`("zkella","note")` event was correctly synced, persisted, and served back
through every HTTP endpoint, including `merkle_root`/`merkle_path` proxying
to the real deployed `ct20` contract. Not yet covered: horizontal scaling,
multiple independent operators, and an operational runbook (see
`docs/POC_IMPLEMENTATION.md` for the full status) — this is a correct,
working reference implementation of the API surface, not a production
deployment.
