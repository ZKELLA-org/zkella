use soroban_sdk::{BytesN, Env, Vec};
use crate::poseidon::Poseidon2Hasher;
use crate::types::StorageKey;

pub const TREE_DEPTH: u32 = 32;
pub const MAX_LEAVES: u32 = u32::MAX; // 2^32 - 1 usable leaf slots

#[cfg(test)]
extern crate std;

#[cfg(test)]
std::thread_local! {
    /// Test-only capacity override, so a test can fill a tree to its (lowered) capacity with
    /// real inserts. Per-thread, so parallel tests cannot interfere. Production always uses
    /// `MAX_LEAVES`.
    pub static TEST_CAPACITY: core::cell::Cell<u32> = const { core::cell::Cell::new(u32::MAX) };
}

#[inline]
fn capacity() -> u32 {
    #[cfg(test)]
    {
        TEST_CAPACITY.with(|c| c.get())
    }
    #[cfg(not(test))]
    {
        MAX_LEAVES
    }
}

/// How many of the most recent roots `is_known_root` accepts, besides the
/// current one. The tree is shared across every asset this `ShieldedToken`
/// instance wraps, so *any* shield/transfer/unshield call — on any asset —
/// advances the root; without this window, a proof anchored to root R is
/// invalidated by unrelated concurrent activity elsewhere in the contract,
/// not just by a conflicting spend of the same note. A fixed-size window is
/// the standard mitigation (the same approach Tornado Cash and Zcash-style
/// pools use) — it doesn't remove the possibility of a proof going stale,
/// it just makes it require `ROOT_HISTORY_SIZE` intervening leaf-inserting calls
/// instead of exactly one.
pub const ROOT_HISTORY_SIZE: u32 = 32;

// Persistent storage TTL constants (Stellar ledger ≈ 5 s).
// Threshold: bump only when remaining TTL falls below this.
// Extend-to: keep alive for this many ledgers from now.
const PERSISTENT_TTL_THRESHOLD: u32 = 17_280 * 30;   // 30 days
const PERSISTENT_TTL_EXTEND_TO: u32 = 17_280 * 365;  // 1 year

/// Root of an all-empty subtree at each level: `EMPTY_ROOTS[0]` is the empty
/// leaf `Poseidon2(0, 0)` (matches circomlibjs `buildPoseidon()([0n, 0n])`),
/// and `EMPTY_ROOTS[i] = Poseidon2(EMPTY_ROOTS[i-1], EMPTY_ROOTS[i-1])`.
///
/// Precomputed rather than hashed on demand. `insert` used to advance a running
/// empty root with one extra Poseidon hash per level, i.e. 32 of the 64 hashes
/// in every insert existed only to recompute these constants. They depend on
/// nothing but the hash function, so they are fixed here (little-endian
/// bytes); the test `empty_roots_table_matches_poseidon_chain` recomputes the
/// whole chain in pure Rust and requires an exact match.
const EMPTY_ROOTS: [[u8; 32]; 33] = [
    [0x64, 0x48, 0xb6, 0x46, 0x84, 0xee, 0x39, 0xa8, 0x23, 0xd5, 0xfe, 0x5f, 0xd5, 0x24, 0x31, 0xdc, 0x81, 0xe4, 0x81, 0x7b, 0xf2, 0xc3, 0xea, 0x3c, 0xab, 0x9e, 0x23, 0x9e, 0xfb, 0xf5, 0x98, 0x20],
    [0xe1, 0xf1, 0xb1, 0x60, 0x44, 0x77, 0xa4, 0x67, 0xf0, 0x8d, 0xc6, 0x9d, 0xcb, 0x44, 0x1a, 0x26, 0xec, 0xa7, 0x84, 0xf5, 0x6f, 0x1a, 0x30, 0xdf, 0x63, 0x22, 0xb1, 0xcd, 0x3d, 0x67, 0x69, 0x10],
    [0x38, 0xd2, 0x56, 0xb8, 0xb2, 0x7e, 0xd5, 0x28, 0xd5, 0x1d, 0x37, 0x50, 0xea, 0x6e, 0x7c, 0x46, 0x06, 0x21, 0xf7, 0x50, 0x8d, 0x75, 0x3d, 0x2e, 0xaf, 0xe2, 0x7e, 0x53, 0x31, 0x33, 0xf4, 0x18],
    [0x2a, 0x95, 0xbc, 0x9d, 0x55, 0x97, 0xac, 0xca, 0x65, 0x82, 0x56, 0x1a, 0x57, 0x28, 0xb7, 0xf1, 0x45, 0x23, 0xa5, 0x3b, 0xe9, 0xff, 0x20, 0x63, 0xd3, 0xb0, 0x17, 0xcb, 0x37, 0xd8, 0xf9, 0x07],
    [0x55, 0x3f, 0x18, 0x39, 0x16, 0xec, 0x5c, 0x7b, 0x4d, 0xad, 0xb2, 0x94, 0x8c, 0xc5, 0x99, 0xa6, 0x07, 0x29, 0xf3, 0x5d, 0x4c, 0x1f, 0x63, 0xc9, 0xf5, 0xb3, 0x46, 0x87, 0x5e, 0xcf, 0x94, 0x2b],
    [0x78, 0x9d, 0xa0, 0x2e, 0xa3, 0xdd, 0x11, 0x1d, 0x61, 0x53, 0xb9, 0x51, 0x69, 0x1e, 0xd7, 0xfe, 0xbc, 0xe1, 0xa9, 0xcc, 0x22, 0x7d, 0xea, 0x46, 0x96, 0x45, 0x66, 0xa6, 0xc5, 0x93, 0xee, 0x2d],
    [0x9d, 0x34, 0x87, 0x3c, 0xbe, 0xaa, 0xa4, 0xa8, 0x7f, 0xac, 0xb5, 0x8c, 0xa8, 0x15, 0x05, 0x8b, 0x7b, 0x59, 0x39, 0xb6, 0x1e, 0x60, 0xcf, 0x82, 0xe9, 0x84, 0x2b, 0xa2, 0xe5, 0x95, 0x82, 0x07],
    [0x61, 0xcc, 0xf3, 0x99, 0x3a, 0xbe, 0x4c, 0x44, 0x1a, 0x21, 0x41, 0x4a, 0x27, 0x2e, 0x6b, 0x61, 0x2a, 0x47, 0x64, 0x45, 0x86, 0xec, 0x1b, 0x50, 0xa6, 0x27, 0x60, 0x8f, 0xf1, 0xe5, 0xa5, 0x2f],
    [0x47, 0xd7, 0xfc, 0x14, 0xa6, 0x56, 0x21, 0x3e, 0xab, 0x28, 0xe2, 0xe3, 0xcc, 0x7a, 0x5e, 0xe4, 0x66, 0x1f, 0x94, 0x9e, 0x38, 0x80, 0xb7, 0xec, 0x21, 0xfd, 0xd8, 0xd0, 0x76, 0x43, 0x88, 0x0e],
    [0xf2, 0x0a, 0x19, 0xda, 0xe5, 0x75, 0x61, 0xde, 0x33, 0x35, 0x71, 0x57, 0xf9, 0x92, 0x58, 0xf9, 0x69, 0xb4, 0x2e, 0xa5, 0xd1, 0x7a, 0x71, 0x28, 0x1e, 0x4f, 0x49, 0x72, 0xda, 0x01, 0x72, 0x1b],
    [0x36, 0x76, 0x7d, 0xce, 0xfa, 0x6b, 0xbc, 0xbe, 0xb5, 0x08, 0x08, 0x65, 0xe4, 0xe1, 0xe6, 0xa6, 0x19, 0x98, 0x24, 0x01, 0xb2, 0xc0, 0x00, 0x52, 0x38, 0x36, 0x5e, 0x72, 0x22, 0x88, 0x8d, 0x1f],
    [0x5a, 0xf8, 0xb5, 0x71, 0x04, 0x9a, 0x87, 0xd0, 0xa8, 0x88, 0xcf, 0x2a, 0xa1, 0xb0, 0x62, 0x61, 0xfb, 0xfc, 0x8c, 0xba, 0x89, 0x15, 0x70, 0xb9, 0xaf, 0x4b, 0x91, 0x6c, 0xf6, 0x82, 0x5d, 0x2c],
    [0xd0, 0xbf, 0xbf, 0xe0, 0x70, 0xf2, 0x58, 0x64, 0x64, 0xf4, 0x13, 0xa1, 0xaa, 0xc4, 0xf5, 0x4e, 0x13, 0xa1, 0x3f, 0xdf, 0x5a, 0x7f, 0x95, 0x20, 0xb8, 0x0b, 0x94, 0xa0, 0x48, 0x41, 0xc5, 0x14],
    [0x0c, 0xe8, 0xeb, 0xf4, 0x4b, 0x8e, 0x11, 0x16, 0xd4, 0x89, 0xad, 0x8c, 0x58, 0x25, 0xbe, 0x11, 0xaf, 0xb9, 0xd8, 0x44, 0xee, 0xc0, 0x10, 0x1e, 0x96, 0x6f, 0x98, 0x2f, 0xb1, 0x33, 0x0d, 0x19],
    [0x92, 0x6c, 0xe0, 0x25, 0x93, 0x64, 0xb3, 0xa5, 0x0a, 0x51, 0xaf, 0x96, 0x65, 0xae, 0x67, 0x11, 0xed, 0x73, 0xad, 0x14, 0x49, 0x35, 0x17, 0xac, 0x52, 0x41, 0x70, 0xce, 0xa9, 0x8a, 0xf9, 0x22],
    [0x23, 0x73, 0xba, 0x8b, 0xd3, 0x53, 0xb7, 0xf8, 0xee, 0xcc, 0x6e, 0xc6, 0x29, 0x6f, 0x52, 0x5a, 0x57, 0x6a, 0xbf, 0x72, 0x8d, 0x22, 0x6f, 0x9f, 0x0b, 0x88, 0xe5, 0x6c, 0x9b, 0x7c, 0x7c, 0x2a],
    [0x92, 0xb9, 0x36, 0x3f, 0x64, 0xdd, 0x75, 0x4d, 0x95, 0x8b, 0x98, 0xc2, 0xc9, 0x43, 0x00, 0x47, 0xfc, 0x3f, 0x46, 0x4d, 0xc1, 0xf9, 0x7a, 0xc6, 0xc1, 0x8e, 0x69, 0x58, 0xe5, 0x86, 0x81, 0x2e],
    [0x0f, 0xf1, 0x1f, 0x1c, 0x9d, 0x24, 0x46, 0x35, 0x27, 0x92, 0x73, 0x64, 0xad, 0x6e, 0xef, 0x8a, 0x94, 0xae, 0x0d, 0x05, 0xcf, 0xc8, 0xe2, 0x49, 0xab, 0x4e, 0x9a, 0x1e, 0x57, 0xc5, 0x57, 0x0f],
    [0xca, 0x2c, 0xf7, 0x34, 0x61, 0xe3, 0x9c, 0x3c, 0xe4, 0x46, 0x7d, 0x69, 0x10, 0xe3, 0x78, 0xfe, 0x1c, 0x0e, 0x80, 0x88, 0x43, 0x3d, 0xf6, 0xd5, 0x4a, 0x55, 0xfb, 0xb5, 0x67, 0xee, 0x30, 0x18],
    [0x3e, 0x1f, 0x19, 0x22, 0xdf, 0xb6, 0x71, 0xd3, 0xf9, 0x12, 0xf7, 0xea, 0x46, 0x1e, 0x0a, 0x88, 0xee, 0x84, 0x8f, 0xdd, 0xe1, 0x2b, 0x6c, 0x18, 0xab, 0x1a, 0xd2, 0xc5, 0x6a, 0xe7, 0x34, 0x21],
    [0xb1, 0xa5, 0x91, 0xdb, 0xf3, 0x8d, 0x8f, 0x8f, 0xa8, 0x3a, 0xee, 0x58, 0xc9, 0xd8, 0x51, 0xc0, 0xb0, 0x59, 0x38, 0xf3, 0x66, 0xd8, 0xeb, 0xfe, 0x4f, 0xbc, 0x4e, 0x84, 0xec, 0x90, 0xdf, 0x19],
    [0x2b, 0xe5, 0xef, 0x22, 0xf7, 0x05, 0x8c, 0x64, 0xb4, 0x12, 0x49, 0xef, 0x93, 0x0e, 0xaf, 0x74, 0x2d, 0x85, 0x84, 0xfd, 0xae, 0x69, 0x1e, 0x98, 0x87, 0x07, 0x5c, 0x6b, 0xa6, 0xa2, 0xcc, 0x18],
    [0x8d, 0x53, 0xd2, 0x49, 0x45, 0x64, 0x05, 0xdf, 0xfa, 0x83, 0xad, 0xef, 0xf2, 0x38, 0x83, 0x62, 0x3a, 0x47, 0x4f, 0xd5, 0xd2, 0x04, 0x13, 0x4d, 0x1b, 0x0d, 0x23, 0x15, 0x94, 0x90, 0x88, 0x23],
    [0x40, 0xd5, 0x9e, 0x52, 0xe4, 0x73, 0xe6, 0x96, 0x1d, 0x0b, 0x8d, 0x9c, 0x2c, 0xaf, 0xa2, 0x66, 0xe8, 0x4d, 0x29, 0xb5, 0x43, 0xf5, 0xe8, 0xe9, 0xc0, 0x6c, 0x7b, 0xa9, 0xb4, 0x1f, 0x17, 0x27],
    [0x21, 0xae, 0xe6, 0xdd, 0x96, 0x96, 0xd5, 0xf8, 0xe5, 0x83, 0x25, 0x39, 0xb9, 0x30, 0xb2, 0xdc, 0x28, 0x0d, 0xfc, 0x74, 0xbc, 0xa0, 0x11, 0x57, 0xfd, 0x29, 0xf6, 0x40, 0x05, 0x65, 0xf6, 0x2f],
    [0x18, 0x15, 0x61, 0xce, 0x30, 0x26, 0x93, 0x69, 0x56, 0xd7, 0xad, 0xf6, 0x68, 0x51, 0xad, 0xe0, 0xa2, 0x78, 0x77, 0x27, 0xf5, 0xf7, 0x02, 0x59, 0xe9, 0x91, 0xd4, 0x43, 0xf1, 0x58, 0x0c, 0x12],
    [0x95, 0x37, 0x48, 0x79, 0xd2, 0x65, 0xd6, 0xa2, 0x1d, 0xa2, 0x65, 0xa5, 0xa0, 0x95, 0xc4, 0x1e, 0x07, 0x03, 0xdb, 0xe5, 0xd5, 0x53, 0xf8, 0x7b, 0xb0, 0x21, 0x3f, 0x0d, 0xb7, 0xfe, 0x21, 0x1f],
    [0xd2, 0x72, 0x8a, 0xe1, 0xdd, 0xa1, 0xb1, 0x8b, 0x96, 0x9e, 0x8a, 0x06, 0x68, 0xe7, 0x26, 0xa8, 0x23, 0x86, 0x6a, 0xf6, 0xc0, 0x8c, 0x63, 0x4c, 0xe1, 0x35, 0x13, 0xa7, 0x5f, 0x90, 0xbe, 0x24],
    [0x6d, 0xc2, 0xda, 0x53, 0xe5, 0x74, 0x4c, 0x74, 0x28, 0xc3, 0x65, 0x1d, 0x82, 0xf3, 0x7e, 0x59, 0xcd, 0xd4, 0x57, 0xad, 0xde, 0xea, 0x0c, 0xc5, 0x91, 0x74, 0xd1, 0x2e, 0xb6, 0x66, 0x86, 0x0f],
    [0xef, 0x59, 0x19, 0x0e, 0x23, 0x2a, 0x1a, 0x3d, 0xb4, 0x8c, 0xe0, 0x6a, 0x3f, 0x7a, 0x7a, 0x4e, 0x59, 0x41, 0x1c, 0x1a, 0x4a, 0x3f, 0x41, 0x34, 0xb0, 0x98, 0x2d, 0xf5, 0x6b, 0xd4, 0x18, 0x09],
    [0xf2, 0x5f, 0x5c, 0x37, 0xad, 0x13, 0x85, 0x12, 0x65, 0x5a, 0xfc, 0x0a, 0x0d, 0xf9, 0x26, 0x2e, 0xfa, 0x4d, 0x40, 0x5e, 0x64, 0x17, 0x69, 0xe7, 0xcd, 0x9e, 0x47, 0x4c, 0x1b, 0xb0, 0xbe, 0x1b],
    [0xd9, 0xea, 0x34, 0x97, 0x4c, 0x18, 0x8e, 0xac, 0xd5, 0x19, 0xb1, 0x2a, 0x92, 0xb9, 0x60, 0xd5, 0x1e, 0x55, 0xf5, 0xdf, 0x61, 0x6c, 0x7a, 0xa1, 0x42, 0x7e, 0x25, 0x8e, 0xc5, 0xa1, 0x68, 0x2f],
    [0x11, 0x1d, 0xd8, 0xb5, 0x5a, 0x1d, 0x28, 0xa2, 0x35, 0x22, 0x21, 0xb7, 0x7e, 0x44, 0x28, 0xeb, 0x45, 0xed, 0x85, 0x3d, 0xad, 0xe8, 0x42, 0x48, 0xaf, 0xe4, 0x05, 0xdb, 0xf8, 0xd2, 0x02, 0x11],
];

fn empty_subtree_root(level: u32) -> [u8; 32] {
    EMPTY_ROOTS[level as usize]
}

/// Returns true if the tree has reached `MAX_LEAVES` capacity and cannot
/// accept another insert. Callers (shield/transfer/unshield) check this
/// *before* calling `insert` so a full tree fails gracefully via
/// `Error::MerkleTreeFull` rather than the panic `insert` itself still
/// asserts as a defensive, should-never-happen invariant check.
pub fn is_full(env: &Env) -> bool {
    let index: u32 = env
        .storage()
        .instance()
        .get(&StorageKey::NextLeafIndex)
        .unwrap_or(0);
    index >= capacity()
}

/// Largest batch `insert_many` accepts (transfer4 inserts 4 notes; shield_batch at most
/// `MAX_SHIELD_BATCH`). A static array bound: it must stay >= both.
pub const MAX_INSERT_BATCH: usize = 8;

/// Number of leaves the tree can still accept, as a plain check the callers
/// use before touching any state (so a full tree fails gracefully).
pub fn has_capacity(env: &Env, count: u32) -> bool {
    let index: u32 = env
        .storage()
        .instance()
        .get(&StorageKey::NextLeafIndex)
        .unwrap_or(0);
    (index as u64) + (count as u64) <= capacity() as u64
}

/// Index the next inserted leaf will get.
pub fn next_leaf_index(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&StorageKey::NextLeafIndex)
        .unwrap_or(0)
}

/// Insert one leaf. Returns its index. See `insert_many`.
pub fn insert(env: &Env, commitment: BytesN<32>, hasher: &mut Poseidon2Hasher) -> u32 {
    insert_many(env, &[commitment], hasher)
}

/// Insert `commitments` as consecutive leaves and return the index of the first.
///
/// Produces exactly the stored tree that inserting the leaves one at a time
/// would (same leaf indices, same node values at every level, same final
/// root), but each level is computed once for the whole batch: consecutive
/// leaves share almost every ancestor above the first couple of levels, so a
/// 4-leaf batch needs about 36 hashes instead of 4 x 32 = 128. It also never
/// reads a right sibling of the last node at a level, since leaves are only
/// ever appended and everything to the right of the last leaf is empty.
///
/// Only the final root is appended to the root history (one entry per call,
/// not one per leaf): intermediate roots exist only inside this call and no
/// transaction can be anchored to them. The caller must have checked
/// `has_capacity`; the assert here is a defensive invariant only.
pub fn insert_many(env: &Env, commitments: &[BytesN<32>], hasher: &mut Poseidon2Hasher) -> u32 {
    let n = commitments.len();
    assert!(n >= 1 && n <= MAX_INSERT_BATCH, "bad merkle batch size");
    let first: u32 = next_leaf_index(env);
    assert!((first as u64) + (n as u64) <= capacity() as u64, "merkle tree full");

    // Level 0: store the new leaves.
    let mut vals: [[u8; 32]; MAX_INSERT_BATCH] = [[0u8; 32]; MAX_INSERT_BATCH];
    for (i, cm) in commitments.iter().enumerate() {
        let key = StorageKey::MerkleNode(0, first + i as u32);
        env.storage().persistent().set(&key, cm);
        env.storage().persistent().extend_ttl(&key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL_EXTEND_TO);
        vals[i] = cm.clone().into();
    }
    let mut lo = first;
    let mut hi = first + (n as u32 - 1);

    for level in 0..TREE_DEPTH {
        let new_lo = lo / 2;
        let new_hi = hi / 2;
        let mut next: [[u8; 32]; MAX_INSERT_BATCH] = [[0u8; 32]; MAX_INSERT_BATCH];
        for p in new_lo..=new_hi {
            let left_idx = 2 * p;
            let right_idx = left_idx + 1;
            let left: [u8; 32] = if left_idx >= lo {
                vals[(left_idx - lo) as usize]
            } else {
                // Only the leftmost parent can have a left child outside the batch: that
                // child is already in the tree.
                env.storage()
                    .persistent()
                    .get::<_, BytesN<32>>(&StorageKey::MerkleNode(level, left_idx))
                    .map(|b| b.into())
                    .unwrap_or(empty_subtree_root(level))
            };
            let right: [u8; 32] = if right_idx <= hi {
                vals[(right_idx - lo) as usize]
            } else {
                // Right of the last node at this level: empty (leaves are only appended).
                empty_subtree_root(level)
            };
            let parent = hasher.hash(&left, &right);
            let key = StorageKey::MerkleNode(level + 1, p);
            env.storage().persistent().set(&key, &BytesN::from_array(env, &parent));
            env.storage().persistent().extend_ttl(&key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL_EXTEND_TO);
            next[(p - new_lo) as usize] = parent;
        }
        vals = next;
        lo = new_lo;
        hi = new_hi;
    }

    // After 32 levels a single node remains: the new root.
    let new_root = BytesN::from_array(env, &vals[0]);
    env.storage().instance().set(&StorageKey::MerkleRoot, &new_root);
    env.storage().instance().set(&StorageKey::NextLeafIndex, &(first + n as u32));

    let mut history: Vec<BytesN<32>> = env
        .storage()
        .instance()
        .get(&StorageKey::RootHistory)
        .unwrap_or_else(|| Vec::new(env));
    history.push_back(new_root);
    if history.len() > ROOT_HISTORY_SIZE {
        history.pop_front();
    }
    env.storage().instance().set(&StorageKey::RootHistory, &history);

    first
}

/// Return the current Merkle root.
pub fn root(env: &Env, _hasher: &mut Poseidon2Hasher) -> BytesN<32> {
    env.storage()
        .instance()
        .get(&StorageKey::MerkleRoot)
        .unwrap_or_else(|| {
            let empty_root = empty_subtree_root(TREE_DEPTH);
            BytesN::from_array(env, &empty_root)
        })
}

/// Returns true if `candidate` is the current root, or was the current root
/// at some point within the last `ROOT_HISTORY_SIZE` leaf-inserting calls (on any
/// asset this contract instance wraps — see `ROOT_HISTORY_SIZE`'s doc
/// comment). Before the tree's first insertion, `history` is empty and only
/// the freshly-computed empty-tree root (from `root()`) is accepted.
pub fn is_known_root(env: &Env, candidate: &BytesN<32>, hasher: &mut Poseidon2Hasher) -> bool {
    if *candidate == root(env, hasher) {
        return true;
    }
    let history: Vec<BytesN<32>> = env
        .storage()
        .instance()
        .get(&StorageKey::RootHistory)
        .unwrap_or_else(|| Vec::new(env));
    history.contains(candidate)
}

/// Return the Merkle authentication path for `leaf_index`.
/// Returns a Vec of sibling nodes from leaf level to root.
pub fn get_path(env: &Env, leaf_index: u32, _hasher: &mut Poseidon2Hasher) -> Vec<BytesN<32>> {
    let mut path  = Vec::new(env);
    let mut index = leaf_index;

    for level in 0..TREE_DEPTH {
        let sibling_index = if index % 2 == 0 { index + 1 } else { index - 1 };

        let sibling: [u8; 32] = env
            .storage()
            .persistent()
            .get::<_, BytesN<32>>(&StorageKey::MerkleNode(level, sibling_index))
            .map(|b| b.into())
            .unwrap_or(empty_subtree_root(level));

        path.push_back(BytesN::from_array(env, &sibling));
        index /= 2;
    }

    path
}

/// Return the direction bits for `leaf_index` (false = left, true = right).
#[cfg(test)]
pub fn get_path_indices(leaf_index: u32) -> [bool; 32] {
    let mut bits  = [false; 32];
    let mut index = leaf_index;
    for b in bits.iter_mut() {
        *b = (index % 2) == 1;
        index /= 2;
    }
    bits
}

/// Verify a Merkle path against a given root. Used in tests.
#[cfg(test)]
pub fn verify_path(
    leaf:  &[u8; 32],
    path:  &[[u8; 32]; 32],
    index: u32,
    root:  &[u8; 32],
) -> bool {
    let mut current = *leaf;
    let mut idx = index;
    for sibling in path.iter() {
        current = if idx % 2 == 0 {
            crate::poseidon::poseidon2_bytes(&current, sibling)
        } else {
            crate::poseidon::poseidon2_bytes(sibling, &current)
        };
        idx /= 2;
    }
    &current == root
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::poseidon::poseidon2_bytes;
    use std::vec::Vec as StdVec;

    /// The precomputed table must equal the Poseidon2 chain it replaces, at
    /// every level, computed independently here in pure Rust.
    #[test]
    fn empty_roots_table_matches_poseidon_chain() {
        let mut current = poseidon2_bytes(&[0u8; 32], &[0u8; 32]);
        for level in 0..=TREE_DEPTH as usize {
            assert_eq!(EMPTY_ROOTS[level], current, "EMPTY_ROOTS[{level}] is wrong");
            current = poseidon2_bytes(&current, &current);
        }
    }

    /// Every level of the tree over `leaves`, in pure Rust, independent of the
    /// contract's incremental algorithm.
    fn levels_of(leaves: &StdVec<[u8; 32]>) -> StdVec<StdVec<[u8; 32]>> {
        let mut levels = std::vec![leaves.clone()];
        for l in 0..TREE_DEPTH as usize {
            let cur = &levels[l];
            let mut next = StdVec::new();
            let mut i = 0;
            while i < cur.len() {
                let right = if i + 1 < cur.len() { cur[i + 1] } else { EMPTY_ROOTS[l] };
                next.push(poseidon2_bytes(&cur[i], &right));
                i += 2;
            }
            levels.push(next);
        }
        levels
    }

    fn leaf(i: u32) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[..4].copy_from_slice(&i.to_le_bytes());
        poseidon2_bytes(&b, &b)
    }

    /// Asserts the contract's stored tree (every node at every level) and root equal the
    /// independent tree over `leaves`.
    fn assert_tree_matches(env: &Env, contract: &soroban_sdk::Address, leaves: &StdVec<[u8; 32]>) {
        let levels = levels_of(leaves);
        env.as_contract(contract, || {
            let root: BytesN<32> = env.storage().instance().get(&StorageKey::MerkleRoot).unwrap();
            assert_eq!(root.to_array(), levels[TREE_DEPTH as usize][0], "root differs from the independent tree");
            for (l, nodes) in levels.iter().enumerate().take(TREE_DEPTH as usize) {
                for (i, expected) in nodes.iter().enumerate() {
                    let stored: BytesN<32> = env
                        .storage()
                        .persistent()
                        .get(&StorageKey::MerkleNode(l as u32, i as u32))
                        .unwrap_or_else(|| panic!("node ({l},{i}) missing"));
                    assert_eq!(stored.to_array(), *expected, "node ({l},{i}) differs");
                }
            }
            let next: u32 = env.storage().instance().get(&StorageKey::NextLeafIndex).unwrap();
            assert_eq!(next as usize, leaves.len());
        });
    }

    /// Batched insertion must store exactly what sequential insertion would, for
    /// every batch size and alignment (crossing power-of-two boundaries).
    #[test]
    fn insert_many_matches_an_independent_tree_for_every_batch_size_and_alignment() {
        let env = Env::default();
        let contract = env.register(crate::ShieldedToken, ());
        let sizes = [1usize, 3, 2, 8, 1, 5, 4, 7, 6, 1, 1, 2, 8, 3, 4, 5, 1, 8, 2, 6, 3, 7, 4, 1, 8, 5, 2, 3, 6, 4];
        let mut leaves: StdVec<[u8; 32]> = StdVec::new();
        let mut next = 0u32;
        for size in sizes {
            let batch: StdVec<[u8; 32]> = (0..size).map(|k| leaf(next + k as u32)).collect();
            next += size as u32;
            env.as_contract(&contract, || {
                let mut hasher = Poseidon2Hasher::new(&env);
                let cms: StdVec<BytesN<32>> = batch.iter().map(|b| BytesN::from_array(&env, b)).collect();
                let first = insert_many(&env, &cms, &mut hasher);
                assert_eq!(first as usize, leaves.len(), "first index of the batch");
            });
            leaves.extend(batch);
            assert_tree_matches(&env, &contract, &leaves);
        }
    }

    /// The single-leaf path (`insert`) agrees with the independent tree too, and
    /// batches and single inserts can be mixed freely.
    #[test]
    fn single_and_batched_inserts_can_be_mixed() {
        let env = Env::default();
        let contract = env.register(crate::ShieldedToken, ());
        let mut leaves: StdVec<[u8; 32]> = StdVec::new();
        for round in 0..6u32 {
            env.as_contract(&contract, || {
                let mut hasher = Poseidon2Hasher::new(&env);
                let l = leaf(1000 + round);
                let idx = insert(&env, BytesN::from_array(&env, &l), &mut hasher);
                assert_eq!(idx as usize, leaves.len());
                leaves.push(l);
                let pair = [leaf(2000 + round), leaf(3000 + round)];
                insert_many(&env, &[BytesN::from_array(&env, &pair[0]), BytesN::from_array(&env, &pair[1])], &mut hasher);
                leaves.extend(pair);
            });
        }
        assert_tree_matches(&env, &contract, &leaves);
    }

    /// A batch whose end would pass capacity is refused by `has_capacity`, and one
    /// that exactly fills the tree is allowed.
    #[test]
    fn has_capacity_boundaries() {
        let env = Env::default();
        let contract = env.register(crate::ShieldedToken, ());
        env.as_contract(&contract, || {
            env.storage().instance().set(&StorageKey::NextLeafIndex, &(MAX_LEAVES - 4));
            assert!(has_capacity(&env, 4));
            assert!(!has_capacity(&env, 5));
            env.storage().instance().set(&StorageKey::NextLeafIndex, &MAX_LEAVES);
            assert!(!has_capacity(&env, 1));
            assert!(has_capacity(&env, 0));
        });
    }
}
