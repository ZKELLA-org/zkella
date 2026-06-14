use soroban_sdk::{BytesN, Env, Vec};
use crate::poseidon::poseidon2_bytes;
use crate::types::StorageKey;

pub const TREE_DEPTH: u32 = 32;
pub const MAX_LEAVES: u32 = u32::MAX; // 2^32 - 1 usable leaf slots

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
fn empty_subtree_root(level: u32) -> [u8; 32] {
    let mut current = EMPTY_LEAF;
    for _ in 0..level {
        current = poseidon2_bytes(&current, &current);
    }
    current
}

/// Insert a new leaf into the incremental Merkle tree.
/// Returns the leaf index assigned.
/// Caller must have already verified the commitment is not a duplicate.
pub fn insert(env: &Env, commitment: BytesN<32>) -> u32 {
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
            .unwrap_or_else(|| empty_subtree_root(level));

        let parent = if node_index % 2 == 0 {
            poseidon2_bytes(&current, &sibling)
        } else {
            poseidon2_bytes(&sibling, &current)
        };

        let parent_index = node_index / 2;
        let parent_level = level + 1;
        let parent_key   = StorageKey::MerkleNode(parent_level, parent_index);

        env.storage().persistent().set(&parent_key, &BytesN::from_array(env, &parent));
        env.storage().persistent().extend_ttl(&parent_key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL_EXTEND_TO);

        current    = parent;
        node_index = parent_index;
    }

    // Update root and leaf counter in instance storage (bumped by caller via shield())
    env.storage()
        .instance()
        .set(&StorageKey::MerkleRoot, &BytesN::from_array(env, &current));
    env.storage()
        .instance()
        .set(&StorageKey::NextLeafIndex, &(index + 1));

    index
}

/// Return the current Merkle root.
pub fn root(env: &Env) -> BytesN<32> {
    env.storage()
        .instance()
        .get(&StorageKey::MerkleRoot)
        .unwrap_or_else(|| {
            let empty_root = empty_subtree_root(TREE_DEPTH);
            BytesN::from_array(env, &empty_root)
        })
}

/// Return the Merkle authentication path for `leaf_index`.
/// Returns a Vec of sibling nodes from leaf level to root.
pub fn get_path(env: &Env, leaf_index: u32) -> Vec<BytesN<32>> {
    let mut path  = Vec::new(env);
    let mut index = leaf_index;

    for level in 0..TREE_DEPTH {
        let sibling_index = if index % 2 == 0 { index + 1 } else { index - 1 };

        let sibling: [u8; 32] = env
            .storage()
            .persistent()
            .get::<_, BytesN<32>>(&StorageKey::MerkleNode(level, sibling_index))
            .map(|b| b.into())
            .unwrap_or_else(|| empty_subtree_root(level));

        path.push_back(BytesN::from_array(env, &sibling));
        index /= 2;
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
            poseidon2_bytes(&current, sibling)
        } else {
            poseidon2_bytes(sibling, &current)
        };
        idx /= 2;
    }
    &current == root
}
