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
