use soroban_sdk::{BytesN, Env, Vec};
use crate::poseidon::Poseidon2Hasher;
use crate::types::StorageKey;

pub const TREE_DEPTH: u32 = 32;
pub const MAX_LEAVES: u32 = u32::MAX; // 2^32 - 1 usable leaf slots

/// How many of the most recent roots `is_known_root` accepts, besides the
/// current one. The tree is shared across every asset this `ShieldedToken`
/// instance wraps, so *any* shield/transfer/unshield call — on any asset —
/// advances the root; without this window, a proof anchored to root R is
/// invalidated by unrelated concurrent activity elsewhere in the contract,
/// not just by a conflicting spend of the same note. A fixed-size window is
/// the standard mitigation (the same approach Tornado Cash and Zcash-style
/// pools use) — it doesn't remove the possibility of a proof going stale,
/// it just makes it require `ROOT_HISTORY_SIZE` intervening insertions
/// instead of exactly one.
pub const ROOT_HISTORY_SIZE: u32 = 32;

// Persistent storage TTL constants (Stellar ledger ≈ 5 s).
// Threshold: bump only when remaining TTL falls below this.
// Extend-to: keep alive for this many ledgers from now.
const PERSISTENT_TTL_THRESHOLD: u32 = 17_280 * 30;   // 30 days
const PERSISTENT_TTL_EXTEND_TO: u32 = 17_280 * 365;  // 1 year

/// The empty leaf value: Poseidon2(0, 0).
/// Matches circomlibjs buildPoseidon()([0n, 0n]) — verified by poseidon2_zero_zero_matches_circomlibjs test.
/// hex (little-endian bytes): 6448b64684ee39a823d5fe5fd52431dc81e4817bf2c3ea3cab9e239efbf59820
const EMPTY_LEAF: [u8; 32] = [
    0x64, 0x48, 0xb6, 0x46, 0x84, 0xee, 0x39, 0xa8,
    0x23, 0xd5, 0xfe, 0x5f, 0xd5, 0x24, 0x31, 0xdc,
    0x81, 0xe4, 0x81, 0x7b, 0xf2, 0xc3, 0xea, 0x3c,
    0xab, 0x9e, 0x23, 0x9e, 0xfb, 0xf5, 0x98, 0x20,
];

/// Pre-computed empty subtree roots at each level.
/// empty_roots[0] = EMPTY_LEAF
/// empty_roots[i] = Poseidon2(empty_roots[i-1], empty_roots[i-1])
///
/// Standalone/one-shot use only (e.g. `root()` before any leaf has ever been
/// inserted). `insert`/`get_path` below track this incrementally instead of
/// calling this per level — see the comment on `running_empty` there for why.
fn empty_subtree_root(hasher: &mut Poseidon2Hasher, level: u32) -> [u8; 32] {
    let mut current = EMPTY_LEAF;
    for _ in 0..level {
        current = hasher.hash(&current, &current);
    }
    current
}

/// Insert a new leaf into the incremental Merkle tree.
/// Returns the leaf index assigned.
/// Caller must have already verified the commitment is not a duplicate.
pub fn insert(env: &Env, commitment: BytesN<32>, hasher: &mut Poseidon2Hasher) -> u32 {
    let index: u32 = env
        .storage()
        .instance()
        .get(&StorageKey::NextLeafIndex)
        .unwrap_or(0);

    assert!(index < MAX_LEAVES, "merkle tree full");

    let cm_bytes: [u8; 32] = commitment.clone().into();

    // Store the leaf at level 0
    let leaf_key = StorageKey::MerkleNode(0, index);
    env.storage().persistent().set(&leaf_key, &commitment);
    env.storage().persistent().extend_ttl(&leaf_key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL_EXTEND_TO);

    // Walk up the tree, recomputing ancestor nodes
    let mut current: [u8; 32] = cm_bytes;
    let mut node_index = index;

    // Tracks empty_subtree_root(level) incrementally instead of recomputing
    // it from EMPTY_LEAF on every iteration. Recomputing from scratch turns a
    // fresh-tree insert into O(depth^2) hashes (0+1+2+...+31 = 496 just for
    // empty-subtree lookups); tracking it here makes it O(depth) — one extra
    // hash per level, 32 total, regardless of how many siblings are empty.
    let mut running_empty: [u8; 32] = EMPTY_LEAF; // == empty_subtree_root(0)

    for level in 0..TREE_DEPTH {
        let sibling_index = if node_index % 2 == 0 {
            node_index + 1  // left child — sibling is right (may be empty)
        } else {
            node_index - 1  // right child — sibling is left (already stored)
        };

        let sibling: [u8; 32] = env
            .storage()
            .persistent()
            .get::<_, BytesN<32>>(&StorageKey::MerkleNode(level, sibling_index))
            .map(|b| b.into())
            .unwrap_or(running_empty);

        let parent = if node_index % 2 == 0 {
            hasher.hash(&current, &sibling)
        } else {
            hasher.hash(&sibling, &current)
        };

        let parent_index = node_index / 2;
        let parent_level = level + 1;
        let parent_key   = StorageKey::MerkleNode(parent_level, parent_index);

        env.storage().persistent().set(&parent_key, &BytesN::from_array(env, &parent));
        env.storage().persistent().extend_ttl(&parent_key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL_EXTEND_TO);

        current    = parent;
        node_index = parent_index;
        running_empty = hasher.hash(&running_empty, &running_empty); // advance to empty_subtree_root(level+1)
    }

    // Update root and leaf counter in instance storage (bumped by caller via shield())
    let new_root = BytesN::from_array(env, &current);
    env.storage()
        .instance()
        .set(&StorageKey::MerkleRoot, &new_root);
    env.storage()
        .instance()
        .set(&StorageKey::NextLeafIndex, &(index + 1));

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

    index
}

/// Return the current Merkle root.
pub fn root(env: &Env, hasher: &mut Poseidon2Hasher) -> BytesN<32> {
    env.storage()
        .instance()
        .get(&StorageKey::MerkleRoot)
        .unwrap_or_else(|| {
            let empty_root = empty_subtree_root(hasher, TREE_DEPTH);
            BytesN::from_array(env, &empty_root)
        })
}

/// Returns true if `candidate` is the current root, or was the current root
/// at some point within the last `ROOT_HISTORY_SIZE` insertions (on any
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
pub fn get_path(env: &Env, leaf_index: u32, hasher: &mut Poseidon2Hasher) -> Vec<BytesN<32>> {
    let mut path  = Vec::new(env);
    let mut index = leaf_index;

    // Same incremental tracking as `insert` — see its comment on `running_empty`.
    let mut running_empty: [u8; 32] = EMPTY_LEAF;

    for level in 0..TREE_DEPTH {
        let sibling_index = if index % 2 == 0 { index + 1 } else { index - 1 };

        let sibling: [u8; 32] = env
            .storage()
            .persistent()
            .get::<_, BytesN<32>>(&StorageKey::MerkleNode(level, sibling_index))
            .map(|b| b.into())
            .unwrap_or(running_empty);

        path.push_back(BytesN::from_array(env, &sibling));
        index /= 2;
        running_empty = hasher.hash(&running_empty, &running_empty);
    }

    path
}

/// Return the direction bits for `leaf_index` (false = left, true = right).
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
