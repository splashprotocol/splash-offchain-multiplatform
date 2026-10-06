# Mainnet royalty script artifact set

Network: **Mainnet**. Source release candidate: splash-core `f8535269d7373a6ea5ddaefe4ed39c215395c409`.

This directory contains 13 raw Plutus V2 CBOR scripts, `script-hashes.txt`,
the public production DAO administrator parameters, and `release-manifest.json`
with independently checked Plutus V2 hashes and SHA-256 sums. Its
`artifactReadiness` is `mainnet-ready`: these are the designated Mainnet
artifacts. `deploymentApproved` remains `false` until the release checks below
are complete.

The DAO V1 policy hashes in this set are:

- Single royalty: `76847f451fb7c4e1d4b93284bb5ec32439770053d86804b7b687c60f`
- Double royalty: `45613319ca4393e41499ffe47cd055718ddf2a1a1fe10c8fd879d0ee`

Do not combine this set with `preprod-royalty-*` directories. The existing
`deploy-preprod-royalty-reference-scripts.ts` is hardcoded to the Preprod
network and is not the Mainnet publisher. Mainnet publication still requires
review of the remote source-build provenance, validator behavior checks against
these exact bytes, and a separately configured Mainnet deployment flow.

The source bundle and validation tool are in splash-core:
`artifacts/mainnet-royalty-f853526/raw-cbor` and
`scripts/verify_mainnet_royalty_artifacts.py`.
For the latest handoff and remaining checks, read
`splash-core/docs/deployment/mainnet-royalty-current-status.md`.
