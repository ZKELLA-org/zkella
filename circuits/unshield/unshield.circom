pragma circom 2.0.0;

include "../common/commitment.circom";
include "../common/nullifier.circom";
include "../common/owner.circom";
include "../common/merkle.circom";
include "../common/range.circom";
include "../common/value_commit.circom";

// D = tree depth. The spent note's full `value` splits into a public payout
// (`pub_value`, leaves the shielded pool as real tokens) and a `change` note
// that stays shielded, owned by the same key (`pk`) as the spent note — so a
// user can unshield part of a note's value while the rest stays private
// instead of having to transfer() a split note first. `change` is
// range-checked nonnegative (Num2Bits fails on a field-wraparound negative if
// `pub_value > value`), so a prover cannot claim more than the note holds.
// The change note's own value stays hidden from the contract exactly like
// any other note's: only its commitment and a value-binding hash
// (`change_value_commit`) are public, not `change` itself.
template Unshield(D) {
    signal input value;
    signal input asset_id;
    signal input rho;
    signal input rcm;
    signal input nk;
    signal input path[D];
    signal input path_index[D];

    signal input anchor;
    signal input nullifier;
    signal input pub_value;
    signal input pub_asset_id;
    signal input recipient_hash;

    // Change note: same owner key as the spent note, fresh (rho, rcm).
    signal input change_rho;
    signal input change_rcm;
    signal input change_rcv;
    signal input change_commitment;
    signal input change_value_commit;

    // The note must commit to the owner key derived from `nk`; this is what
    // ties `nk` (and hence the nullifier) to the note being spent.
    component owner = OwnerKey();
    owner.nk <== nk;

    component cm = NoteCommitment();
    cm.value    <== value;
    cm.asset_id <== asset_id;
    cm.rho      <== rho;
    cm.rcm      <== rcm;
    cm.pk       <== owner.pk;

    component mp = MerkleProof(D);
    mp.leaf <== cm.cm;
    for (var i = 0; i < D; i++) {
        mp.path[i]  <== path[i];
        mp.index[i] <== path_index[i];
    }
    mp.root === anchor;

    component nf_c = Nullifier();
    nf_c.nk  <== nk;
    nf_c.rho <== rho;
    nf_c.nf  === nullifier;

    asset_id === pub_asset_id;

    // value = pub_value + change, change >= 0 (see the template comment above
    // for why Num2Bits on `change` is what actually enforces pub_value <= value).
    signal change;
    change <== value - pub_value;

    component value_range = Range64();
    value_range.value <== value;
    component pub_value_range = Range64();
    pub_value_range.value <== pub_value;
    component change_range = Range64();
    change_range.value <== change;

    component change_cm = NoteCommitment();
    change_cm.value    <== change;
    change_cm.asset_id <== asset_id;
    change_cm.rho      <== change_rho;
    change_cm.rcm      <== change_rcm;
    change_cm.pk       <== owner.pk;
    change_cm.cm === change_commitment;

    component change_cv = ValueCommit();
    change_cv.value <== change;
    change_cv.rcv   <== change_rcv;
    change_cv.cv === change_value_commit;

    // recipient_hash is a public binding — not used in constraints
    // but included as public input so the contract can verify destination
    signal recipient_hash_check;
    recipient_hash_check <== recipient_hash;
}

component main {public [anchor, nullifier, pub_value, pub_asset_id, recipient_hash, change_commitment, change_value_commit]}
  = Unshield(32);
