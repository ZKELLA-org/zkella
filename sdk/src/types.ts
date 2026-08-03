export interface Note {
  value:      bigint
  assetId:    string   // SEP-41 contract address
  rho:        Uint8Array
  rcm:        Uint8Array
  leafIndex:  number
  commitment: Uint8Array
}

export interface SpendingKey {
  raw:         Uint8Array
  nullifierKey: Uint8Array
  viewingKey:  Uint8Array
  transmissionKey: Uint8Array  // BN254 G1 point, compressed
}

export interface ViewingKey {
  raw:             Uint8Array
  transmissionKey: Uint8Array
  birthdayLedger:  number
}

export interface ShieldedAddress {
  diversifier: Uint8Array   // 11 bytes
  pkD:         Uint8Array   // BN254 G1 point, compressed
  toString():  string       // base58check encoded
}

export interface Proof {
  a: Uint8Array  // G1 point, 64 bytes
  b: Uint8Array  // G2 point, 128 bytes
  c: Uint8Array  // G1 point, 64 bytes
}

export interface TransferOptions {
  to:     string   // shielded address
  asset:  string   // SEP-41 contract
  amount: bigint
}

export interface SwapOptions {
  assetIn:        string
  assetOut:       string
  amountIn:       bigint
  maxSlippageBps: number
  expiryLedgers?: number
}

export interface WalletConfig {
  keys:         SpendingKey
  network:      'testnet' | 'mainnet'
  sorobanRpc:   string
  indexerUrl:   string
  tokenAddress:  string
  /**
   * Stellar account that signs and pays for every Soroban transaction this
   * wallet submits (`shield()`'s SEP-41 `from`, and the source account for
   * every contract invocation). Distinct from `keys` (the shielded-pool
   * spending key) — one is a Stellar account identity, the other is a
   * ZKELLA note-ownership identity; conflating them isn't required by the
   * protocol and isn't assumed here.
   */
  stellarSecret: string
  /** Compiled circuit artifacts — see `sdk/src/prover/*`. Each is required
   *  only for the corresponding `ZKELLAWallet` method. */
  shieldCircuit?:    { wasmPath: string; zkeyPath: string }
  transferCircuit?:  { wasmPath: string; zkeyPath: string }
  transfer4Circuit?: { wasmPath: string; zkeyPath: string }
  unshieldCircuit?:  { wasmPath: string; zkeyPath: string }
}

export interface ViewingKeyExport {
  version:          number
  network:          string
  viewing_key:      string
  transmission_key: string
  birthday_ledger:  number
}
