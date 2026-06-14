pragma circom 2.0.0;

include "../common/commitment.circom";
include "../common/range.circom";
include "../common/value_commit.circom";

template Shield() {
    signal input value;
    signal input asset_id;
    signal input rho;
    signal input rcm;
    signal input rcv;

    signal input commitment;
    signal input value_commit;
    signal input pub_value;
    signal input pub_asset_id;

    component cm_check = NoteCommitment();
    cm_check.value    <== value;
    cm_check.asset_id <== asset_id;
    cm_check.rho      <== rho;
    cm_check.rcm      <== rcm;
    cm_check.cm       === commitment;

    component cv_check = ValueCommit();
    cv_check.value <== value;
    cv_check.rcv   <== rcv;
    cv_check.cv    === value_commit;

    value    === pub_value;
    asset_id === pub_asset_id;

    component range = Range64();
    range.value <== value;
}

component main {public [commitment, value_commit, pub_value, pub_asset_id]}
  = Shield();
