# SDK release decisions

- **Package:** `@zkella/sdk`, published with public access (`publishConfig`). It ships only `dist/`; circuit artifacts are not bundled, and callers pass their paths.
- **Version:** the first publish is `0.1.0`, not `1.0.0`. The API is not stable yet: the wallet's own flows are proven live, but several wrappers were only just wired and have one live run each. `1.0.0` is published once those wrappers have a second live run and the public surface stops changing.
- **Network:** `sdk/src/config/testnet.ts` holds the Testnet addresses, kept in sync with `deployments.json` by a test. Mainnet configuration is added with the mainnet deployment.
- **Publishing:** requires the npm login of the publishing account. Each version is permanent, so the first publish is made only after the team confirms the account and the version.

## Known audit findings in the published tree

A fresh install of `@zkella/sdk@0.1.1` reports 4 high and 2 moderate findings through its dependencies (unchanged from `0.1.0` — the dependency set itself hasn't moved, only metadata and version). Decision: no new version is published for these, because the fix cannot be made from the SDK's manifest.

- `bfj`, `jsonpath`, `underscore`: reached only through the `snarkjs` command-line bundle (`build/cli.cjs`). The SDK imports the `snarkjs` library and never calls the CLI.
- `ws`, `ethers`, `@ethersproject/*`: reached through `circomlibjs`, which imports neither the ethers provider nor websockets in the code the SDK uses.
- npm ignores `overrides` in a published package, and direct dependencies on patched versions don't replace the nested copies (`jsonpath` and `ethers` keep their own).

The repository's own tree is remediated (see `docs/SECURITY_TOOLING_REPORT.md`). The complete fix for consumers means replacing `circomlibjs` and `snarkjs` with maintained alternatives; that is planned for the release that stabilizes the API.
