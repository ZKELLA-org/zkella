# SDK release decisions

- **Package:** `@zkella/sdk`, published with public access (`publishConfig`). It ships only `dist/`; circuit artifacts are not bundled, and callers pass their paths.
- **Version:** the first publish is `0.1.0`, not `1.0.0`. The API is not stable yet: the wallet's own flows are proven live, but several wrappers were only just wired and have one live run each. `1.0.0` is published once those wrappers have a second live run and the public surface stops changing.
- **Network:** `sdk/src/config/testnet.ts` holds the Testnet addresses, kept in sync with `deployments.json` by a test. Mainnet configuration is added with the mainnet deployment.
- **Publishing:** requires the npm login of the publishing account. Each version is permanent, so the first publish is made only after the team confirms the account and the version.
