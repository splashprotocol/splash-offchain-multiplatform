# Mainnet deployment configuration

`mainnet.deployment.json` is the sole Bloom Mainnet script inventory. Existing
top-level entries retain their legacy hashes and reference UTxOs. The
`protectedRoyalty` object contains the 13 protected single and double royalty
scripts, and `protectedFeeSwitch` contains the two ordinary pool validators
and their two DAO staking policies. All 17 references were checked against
the published Mainnet script outputs in
`splash-core/docs/deployment/mainnet-reference-publication-2026-10-09.md`.

Start Bloom with the normal `--deployment-path` pointing to this JSON. A
separate `--protected-royalty-deployment-path` is not needed for Mainnet.
Royalty and FeeSwitch pool validators are selected by the payment script hash
of each pool UTxO. Old pools continue to use their original references; new
pools use the matching entries in the protected objects.

The ordinary DAO policies are recorded in the same JSON because new pool
datums can name their staking credentials. Bloom currently does not create
ordinary DAO action transactions; its pool order execution uses the pool
validator references. The double royalty withdrawal policy
`a062da72f9f3280fbed90b3e95ea943aabd7eff9c82343324a9726ad` is an
existing external dependency and is not among the 17 newly published scripts.

## Read-only verification on 2026-10-10

The local Mainnet node socket served both confirmed pool-creation blocks and
transactions: `7d8454f89d24de81cbeea0979384affddfe0cb8bf11850613cd1acb1f9c363ab`
for `pool`, and `73db4c6e5199221b67460cfe806ae33e04699d435dfae59ecebc3d44b420d5d6`
for `pool-bfee`, `royalty-pool`, and `double-royalty-pool`. The four transaction
outputs were decoded by Spectrum's `ConstFnPool::try_from_ledger` using this
Mainnet deployment JSON and routed to the four protected variants. The
captured node output CBOR is in
`spectrum-offchain-cardano/tests/fixtures/mainnet_protected_pool_outputs_20261010.json`;
the read-only ChainSync probe is
`cardano-chain-sync/examples/verify_mainnet_pool_block.rs`.

All 17 protected reference UTxOs were unspent and their reference-script
hashes matched this JSON when checked through Koios and Blockfrost. `cargo
check --offline -p bloom-cardano-agent -p spectrum-offchain-cardano` passed;
the protected and Mainnet routing tests passed. This does not test live order
execution, transaction construction, or DAO/royalty withdrawals. The entire
`spectrum-offchain-cardano` test suite still has 12 failures in legacy fixtures
and other unrelated cases; do not interpret the targeted pass as a clean full
suite.
