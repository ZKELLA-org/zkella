const circomlibjs = require("circomlibjs");
const fs = require("fs");

(async () => {
  const poseidon = await circomlibjs.buildPoseidon();
  const F = poseidon.F;
  function P2(a, b) {
    return F.toObject(poseidon([BigInt(a), BigInt(b)]));
  }

  const asset_in = 111n;
  const asset_out = 222n;
  const amount_in = 1000000n;
  const max_slippage_bps = 250n; // 2.5%
  const intent_nonce = 999n;

  // Correct derivation: min_amount_out = floor(amount_in * (10000 - bps) / 10000)
  const scaled = amount_in * (10000n - max_slippage_bps);
  const min_amount_out = scaled / 10000n; // BigInt division floors automatically
  const amount_out = min_amount_out + 5000n; // executed price beats the floor

  const packed = amount_in * (2n ** 32n) + max_slippage_bps;
  const h1 = P2(asset_in, asset_out);
  const h2 = P2(packed, intent_nonce);
  const intent_commitment = P2(h1, h2);

  const input = {
    intent_nonce: String(intent_nonce),
    amount_in: String(amount_in),
    max_slippage_bps: String(max_slippage_bps),
    intent_commitment: String(intent_commitment),
    asset_in: String(asset_in),
    asset_out: String(asset_out),
    amount_out: String(amount_out),
    min_amount_out: String(min_amount_out),
  };

  fs.writeFileSync(__dirname + "/input.json", JSON.stringify(input, null, 2));
  console.log("min_amount_out (correctly derived) =", min_amount_out.toString());
  console.log("amount_out =", amount_out.toString());
  console.log("intent_commitment =", intent_commitment.toString());
  console.log("Wrote input.json");

  // Adversarial witness: same commitment, but min_amount_out forged to 0 —
  // this is exactly the attack Vuln 1 described. Must FAIL witness generation
  // now that min_amount_out is bound to amount_in/max_slippage_bps.
  const forged = { ...input, min_amount_out: "0" };
  fs.writeFileSync(__dirname + "/input_forged.json", JSON.stringify(forged, null, 2));
  console.log("Wrote input_forged.json (min_amount_out forged to 0 — must fail)");
})();
