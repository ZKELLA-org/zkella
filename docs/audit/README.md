# Audit readiness evidence

Navigation for the preparation work tracked in `docs/EXTERNAL_AUDIT_PREPARATION.md`. Everything in this directory is produced evidence (STRIDE model, data flows, artifact provenance, scan results), not the audit itself — no external review has happened yet.

- **`THREAT_MODEL.md`** — the STRIDE threat model, following the [Stellar template](https://developers.stellar.org/docs/build/security-docs/threat-modeling/STRIDE-template). Every issue cites exact file:line evidence checked on 2026-11-13, not assumed.
- **`DATA_FLOWS.md`** — the data-flow diagram and trust-boundary list `THREAT_MODEL.md`'s table maps against, with explicit state stores per entity.
- **`artifacts.json`** — real sha256 provenance for every committed circuit trusted-setup artifact (zkeys, witness WASM, verifying keys). One finding closed in this pass: a locally-cached Powers-of-Tau file (`.ptau/pot12_final.ptau`) was a corrupted S3 error page, not a real ceremony file, which would have silently broken a from-scratch rebuild of the `shield`/`compliance` circuits — fixed by regenerating it locally and confirmed with two real rebuilds through `circuits/build.sh`.
- **`findings.json`** — machine-readable register of scan/fuzz/clippy findings.

Dated snapshots: this directory reflects the repository as of the commit each file states at its top. Re-run the underlying checks (see each file's own methodology notes) before citing any of this against a later commit.
