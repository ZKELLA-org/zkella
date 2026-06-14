pragma circom 2.0.0;

include "../common/commitment.circom";
include "../common/nullifier.circom";
include "../common/merkle.circom";
include "../common/range.circom";

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

    component cm = NoteCommitment();
    cm.value    <== value;
    cm.asset_id <== asset_id;
    cm.rho      <== rho;
    cm.rcm      <== rcm;

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

    value    === pub_value;
    asset_id === pub_asset_id;

    component range = Range64();
    range.value <== value;

    // recipient_hash is a public binding — not used in constraints
    // but included as public input so the contract can verify destination
    signal recipient_hash_check;
    recipient_hash_check <== recipient_hash;
}

component main {public [anchor, nullifier, pub_value, pub_asset_id, recipient_hash]}
  = Unshield(32);
