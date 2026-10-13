/**
 * Stellar Testnet configuration for the ZKELLA stack. Addresses are the
 * `testnet_final` record in deployments.json; keep the two in sync.
 */
export const TESTNET_CONTRACTS = {
  verifier:     'CBICEUJRZ54VBBT7DHFVDUBWNZ64VTD43YLME7W2I5BE5VFEP2D43U7V',
  governance:   'CCQ2JTVUS2OKHZFFTTX7C7NQ65XFDDNFE66ZJX5PX5K4PLXVDB23NRYP',
  token:        'CDOHQBBDCNY6VVFBOOYDIIVQMA2U2XOT7ZP5AZWHUQKKFN6D4Z63WODP',
  swap:         'CDLETDJQX7P5PAQDSIW62PMUTQ7JKECZLLT4E76PEPZ4EVDEA24OIPRK',
  compliance:   'CBNJVPZQA4BLT2MUBWTK3I4ISWJ5JTWTXY456LYG2OR4HKJCGT2D2LJD',
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
