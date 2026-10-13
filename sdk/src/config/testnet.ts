/**
 * Stellar Testnet configuration for the ZKELLA stack. Addresses are the
 * `testnet_final` record in deployments.json; keep the two in sync.
 */
export const TESTNET_CONTRACTS = {
  verifier:     'CC2LQPXH3L5YKRP7YJ6UIC57AOGJXBQN4DEKNRU4Y32ABXJZOENCDAX3',
  governance:   'CDTJLTBEKBXRJJKHVI32A5UMBB4UC6VBMDOF7WR43H2SKCCRAVRJWY5Q',
  token:        'CA5TFEVODC25SSEZII2XHB2XMCKFNXNLXRNFTKWPKMT5PCWYZUMLPRUZ',
  swap:         'CBN7JJEPAEA5NCKOECPPGHETAK4CCCUFOBUJCDZ7K7HPIV7Y6ILOC524',
  compliance:   'CDP5SRSUFDVEYHUCUX53SM4PZVTOIHDZR3Z5C7G4TFFKAQSLX64FOZVJ',
  viewingKeys:  'CDT776JLXU5GWRIY6WXLZGVKZ5V4TG32HAITNPFZVX5UCJSMMFHNMEEE',
} as const

export const TESTNET_SOROBAN_RPC = 'https://soroban-testnet.stellar.org'

/**
 * The indexer's own documented default (indexer/README.md,
 * docker-compose.yml's `INDEXER_HTTP_PORT`) for a locally-run instance
 * pointed at `TESTNET_CONTRACTS.token`. The indexer is self-hosted
 * infrastructure (see docs/RUNBOOK.md §1), not a permanently-running
 * public service this SDK reaches by default — callers running their own
 * instance on this port get drift coverage against `deployments.json` for
 * free; anyone pointed elsewhere should override `indexerUrl` explicitly
 * wherever this package's wallet/indexer clients take it. See
 * docs/TRANCHE2_DELIVERABLES.md §4.1 for a live run against this port,
 * backed by a real PostgreSQL server, recorded with sync log and
 * `/health` output.
 */
export const TESTNET_INDEXER_URL = 'http://localhost:8787'
