#!/usr/bin/env python3
"""
Converts a snarkjs Groth16 verification_key.json + proof.json + public.json
into the wire format contracts/verifier expects:

  vk_bytes    = alpha_g1(64) || beta_g2(128) || gamma_g2(128) || delta_g2(128) || IC[0](64) || IC[1](64) || ...
  proof_bytes = A(64) || B(128) || C(64)

Host point encoding (soroban-sdk crypto::bn254, protocol 25+):
  G1 = 64 bytes:  be(X) || be(Y)
  G2 = 128 bytes: be(X) || be(Y), each coordinate an Fp2 = be(c1) || be(c0)
    (c0 = real part, c1 = imaginary part; snarkjs JSON stores each coordinate
    as [c0, c1], so this script swaps order when emitting bytes.)

Public inputs for the contract call are the field elements as 32-byte
little-endian (matching BytesN<32> used throughout ct20's Rust code); this
script emits both a big-endian (for reference) and little-endian hex per
input, since ct20 stores/compares note fields as little-endian.
"""
import json
import sys


def fe_to_be32(x: str) -> bytes:
    return int(x).to_bytes(32, "big")


def g1_to_bytes(p) -> bytes:
    x, y, _ = p
    return fe_to_be32(x) + fe_to_be32(y)


def g2_to_bytes(p) -> bytes:
    (x_c0, x_c1), (y_c0, y_c1), _ = p
    x_bytes = fe_to_be32(x_c1) + fe_to_be32(x_c0)
    y_bytes = fe_to_be32(y_c1) + fe_to_be32(y_c0)
    return x_bytes + y_bytes


def main():
    vk = json.load(open("verification_key.json"))
    proof = json.load(open("proof.json"))
    public = json.load(open("public.json"))

    assert vk["protocol"] == "groth16"
    assert vk["curve"] == "bn128"

    vk_bytes = (
        g1_to_bytes(vk["vk_alpha_1"])
        + g2_to_bytes(vk["vk_beta_2"])
        + g2_to_bytes(vk["vk_gamma_2"])
        + g2_to_bytes(vk["vk_delta_2"])
    )
    for ic_point in vk["IC"]:
        vk_bytes += g1_to_bytes(ic_point)

    proof_bytes = (
        g1_to_bytes(proof["pi_a"])
        + g2_to_bytes(proof["pi_b"])
        + g1_to_bytes(proof["pi_c"])
    )

    print("vk_len", len(vk_bytes), "expected 448 + 64*", len(vk["IC"]), "=", 448 + 64 * len(vk["IC"]))
    print("proof_len", len(proof_bytes), "expected 256")
    print()
    print("VK_HEX =", vk_bytes.hex())
    print()
    print("PROOF_HEX =", proof_bytes.hex())
    print()
    print("PUBLIC_INPUTS_BE_HEX (order: commitment, value_commit, pub_value, pub_asset_id):")
    for x in public:
        print(" ", fe_to_be32(x).hex())
    print()
    print("PUBLIC_INPUTS_LE_HEX (ct20's BytesN<32> convention, little-endian):")
    for x in public:
        print(" ", fe_to_be32(x)[::-1].hex())


if __name__ == "__main__":
    main()
