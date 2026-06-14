use soroban_sdk::{contracttype, contracterror, Address, Bytes, BytesN};

// ── Storage keys ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    // Instance storage (cheap, bumped on every shield call)
    Admin,
    PendingAdmin,   // Set by transfer_admin(); cleared by accept_admin()
    MerkleRoot,
    NextLeafIndex,
    Paused,
    VerifyingKey,
    ShieldedSupply(Address),
    // Persistent storage (long-lived, pays rent; TTL bumped on every write)
    MerkleLeaf(u32),
    MerkleNode(u32, u32),      // (level, index) — level 0 = leaf, level 32 = root
    Nullifier(BytesN<32>),
    CommitmentSeen(BytesN<32>), // Set to true when a commitment is inserted (prevents replay)
}

// ── Public inputs passed alongside proofs ────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub struct ShieldPublicInputs {
    pub commitment:   BytesN<32>,
    pub value_commit: BytesN<32>,
    pub pub_value:    i128,
    pub pub_asset_id: Address,
}

#[contracttype]
#[derive(Clone)]
pub struct TransferPublicInputs {
    pub anchor:            BytesN<32>,
    pub nullifiers:        soroban_sdk::Vec<BytesN<32>>,
    pub out_commitments:   soroban_sdk::Vec<BytesN<32>>,
    pub in_value_commits:  soroban_sdk::Vec<BytesN<32>>,
    pub out_value_commits: soroban_sdk::Vec<BytesN<32>>,
    pub fee:               i128,
    pub asset_id:          Address,
}

#[contracttype]
#[derive(Clone)]
pub struct UnshieldPublicInputs {
    pub anchor:         BytesN<32>,
    pub nullifier:      BytesN<32>,
    pub pub_value:      i128,
    pub pub_asset_id:   Address,
    pub recipient_hash: BytesN<32>,
}

// ── Emitted events ────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub struct NoteCommitmentEvent {
    pub leaf_index:     u32,
    pub commitment:     BytesN<32>,
    pub encrypted_note: Bytes,
}

#[contracttype]
#[derive(Clone)]
pub struct NullifierEvent {
    pub nullifier: BytesN<32>,
}

#[contracttype]
#[derive(Clone)]
pub struct UnshieldEvent {
    pub to:     Address,
    pub amount: i128,
    pub asset:  Address,
}

// `from` intentionally omitted: publishing the shielder's public address
// would deanonymize who deposited, breaking the protocol's privacy guarantee.
// The SAC transfer event already records the token movement on-chain.
#[contracttype]
#[derive(Clone)]
pub struct ShieldEvent {
    pub leaf_index: u32,
    pub asset:      Address,
    pub commitment: BytesN<32>,
}

// ── Contract error codes ──────────────────────────────────────────────────────

#[contracterror]
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized   = 1,
    NotInitialized       = 2,
    Paused               = 3,
    InvalidProof         = 4,
    InvalidAnchor        = 5,
    NullifierSpent       = 6,
    CommitmentMismatch   = 7,
    AssetMismatch        = 8,
    AmountMismatch       = 9,
    Unauthorized         = 10,
    MerkleTreeFull       = 11,
    NotImplemented       = 12, // stub functions not yet available (M2)
    InvalidNote          = 13, // encrypted_note has wrong length or format
    DuplicateCommitment  = 14, // same commitment submitted twice
}
