const circomlibjs = require("circomlibjs");
const fs = require("fs");

(async () => {
  const poseidon = await circomlibjs.buildPoseidon();
  const F = poseidon.F;

  function P2(a, b) {
    return F.toObject(poseidon([BigInt(a), BigInt(b)]));
  }

  // Real Stellar Testnet native-XLM SAC contract address
  // CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC, its 32-byte
  // contract hash read little-endian (ZKELLA's BytesN<32> convention) —
  // independently verified to equal ct20::address_to_field_bytes' output for
  // this exact address, reversed+reparsed the same way verifier::verify does.
  const asset_id = 44239764132731213050593584193610954592770732183378613204087781673509415785175n;

  const value = 10000000n; // 1 XLM in stroops, well above MIN_SHIELD_AMOUNT=1000
  const rho = 823746192837465192837465n;
  const rcm = 918273645192837465918273n;
  const rcv = 102938475610293847561029n;

  const commitment = P2(P2(value, asset_id), P2(rho, rcm));
  const value_commit = P2(value, rcv);

  const input = {
    value: String(value),
    asset_id: String(asset_id),
    rho: String(rho),
    rcm: String(rcm),
    rcv: String(rcv),
    commitment: String(commitment),
    value_commit: String(value_commit),
    pub_value: String(value),
    pub_asset_id: String(asset_id),
  };

  fs.writeFileSync(__dirname + "/input_testnet.json", JSON.stringify(input, null, 2));
  console.log("value          =", value.toString());
  console.log("asset_id       =", asset_id.toString());
  console.log("commitment     =", commitment.toString());
  console.log("value_commit   =", value_commit.toString());
  console.log("rho            =", rho.toString());
  console.log("rcm            =", rcm.toString());
  console.log("Wrote input_testnet.json");
})();
