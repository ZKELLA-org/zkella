const circomlibjs = require("circomlibjs");
const fs = require("fs");

(async () => {
  const poseidon = await circomlibjs.buildPoseidon();
  const F = poseidon.F;

  function P2(a, b) {
    return F.toObject(poseidon([BigInt(a), BigInt(b)]));
  }

  const D = 32;
  const EMPTY_LEAF = P2(0n, 0n);
  const emptyRoots = [EMPTY_LEAF];
  for (let i = 1; i <= D; i++) {
    emptyRoots.push(P2(emptyRoots[i - 1], emptyRoots[i - 1]));
  }

  const DOMAIN_PK = 2258241487740017274987n;
  const ownerPk = nk => P2(nk, DOMAIN_PK);
  function noteCommitment(value, asset_id, rho, rcm, pk) {
    return P2(P2(P2(value, asset_id), P2(rho, rcm)), pk);
  }
  function nullifierOf(nk, rho) {
    return P2(nk, rho);
  }

  // Partial withdrawal: the spent note is worth 250000, only 100000 leaves the
  // pool publicly, and a fresh 150000-value change note (same owner key) stays
  // shielded — this is the case the change-note feature exists for.
  const value = 250000n;
  const pubValue = 100000n;
  const change = value - pubValue;
  const asset_id = 98765n;
  const rho = 2222n;
  const rcm = 3333n;
  const nk = 4444n;
  const changeRho = 5555n;
  const changeRcm = 6666n;
  const changeRcv = 7777n;

  const pk = ownerPk(nk);
  const leaf = noteCommitment(value, asset_id, rho, rcm, pk);
  const nf = nullifierOf(nk, rho);
  const changeCommitment = noteCommitment(change, asset_id, changeRho, changeRcm, pk);
  const changeValueCommit = P2(change, changeRcv);

  // Single leaf inserted at index 0 of an otherwise-empty depth-32 tree:
  // anchor is simply the fully-empty-subtree chain seeded with this one real leaf.
  let node = leaf;
  for (let lvl = 0; lvl < D; lvl++) {
    node = P2(node, emptyRoots[lvl]); // leaf/current is always the left child
  }
  const anchor = node;

  const path = [];
  const index = [];
  for (let lvl = 0; lvl < D; lvl++) {
    path.push(emptyRoots[lvl]);
    index.push(0);
  }

  // Independent verification (mirrors the circuit's own MerkleProof logic).
  function verifyPath(l, p, idx, root) {
    let cur = l;
    for (let i = 0; i < p.length; i++) cur = idx[i] === 0 ? P2(cur, p[i]) : P2(p[i], cur);
    return cur === root;
  }
  if (!verifyPath(leaf, path, index, anchor)) throw new Error("path does not reconstruct anchor");

  // recipient_hash is unconstrained by this circuit (see unshield.circom's
  // own comment) — any value is fine at the circuit-witness level.
  const recipient_hash = 42n;

  const input = {
    value: String(value),
    asset_id: String(asset_id),
    rho: String(rho),
    rcm: String(rcm),
    nk: String(nk),
    path: path.map(String),
    path_index: index.map(String),

    anchor: String(anchor),
    nullifier: String(nf),
    pub_value: String(pubValue),
    pub_asset_id: String(asset_id),
    recipient_hash: String(recipient_hash),

    change_rho: String(changeRho),
    change_rcm: String(changeRcm),
    change_rcv: String(changeRcv),
    change_commitment: String(changeCommitment),
    change_value_commit: String(changeValueCommit),
  };

  fs.writeFileSync(__dirname + "/input.json", JSON.stringify(input, null, 2));
  console.log("anchor               =", anchor.toString());
  console.log("nullifier            =", nf.toString());
  console.log("pub_value            =", pubValue.toString());
  console.log("pub_asset_id         =", asset_id.toString());
  console.log("recipient_hash       =", recipient_hash.toString());
  console.log("change (hidden)      =", change.toString());
  console.log("change_commitment    =", changeCommitment.toString());
  console.log("change_value_commit  =", changeValueCommit.toString());
  console.log("Wrote input.json");
})();
