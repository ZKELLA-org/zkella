# Security Policy

ZKELLA is confidential-finance infrastructure for Stellar/Soroban. The contracts, circuits, and SDK in this repository handle proof verification, note commitments, and real SEP-41 token custody — treat anything that could affect fund safety, proof soundness, or private-note confidentiality as a security issue.

## Current status

This is a **PoC implementation**, not an audited, production release. It has been through an internal senior-review pass (see `docs/TESTNET_DEPLOYMENT.md` for what's been fixed and validated on live Stellar Testnet), but **no external, independent security audit has been performed yet**. Every trusted-setup ceremony behind the Groth16 verifying keys in this repository so far is a local, single-contributor development ceremony — not suitable for any deployment holding real user funds. Do not use any contract address in this repository to custody real value.

## Reporting a vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

If you find a security issue — a soundness gap in a circuit, an authorization bypass in a contract, a way to double-spend, forge a proof, or drain escrowed funds, a key-derivation or encryption weakness in the SDK, or anything else that could compromise funds, proofs, or private data — report it privately:

- Open a [GitHub private security advisory](https://github.com/ZKELLA-org/zkella/security/advisories/new) for this repository — this is the preferred channel, since it's private by default and notifies maintainers directly.

Please include:
- The component affected (e.g. `contracts/swap`, `circuits/unshield`, `sdk/src/prover`)
- Whether the issue is exploitable against the current live Testnet deployment (see `docs/TESTNET_DEPLOYMENT.md` for current addresses) or only in theory
- Steps to reproduce, or a proof-of-concept if you have one

## What to expect

- We will acknowledge a report as soon as reasonably possible.
- We will work with you to understand impact and severity before any public disclosure.
- Once a fix is available and deployed (or the report is confirmed out of scope), we're happy to credit the reporter in the fix's changelog/commit, unless you prefer to stay anonymous.

## Scope

In scope: `contracts/`, `circuits/`, `sdk/`, `indexer/`.

Out of scope: third-party dependencies (report upstream), the Stellar network itself, and anything requiring physical or social-engineering access.
