# Protected royalty Mainnet deployment template

`mainnet.protected-royalty.deployment.template.json` is a separate deployment
profile for the protected royalty scripts. It starts with the existing Mainnet
deployment and updates the 13 royalty script entries listed in
`splash-testing-cardano/scripts/mainnet-royalty-f853526/script-hashes.txt`.
The old `mainnet.deployment.json` remains the legacy profile so both generations
can be routed by their pool script hashes.

Each new `referenceUtxo` is `"TBD"` until that script has been published and
its output verified on Mainnet. **This template cannot be loaded by Bloom**:
replace every `TBD` with the actual `{ "txHash": "...", "outputIndex": n }`
before passing a copy to `--protected-royalty-deployment-path`.

The published references are filled in
`mainnet.protected-royalty.deployment.json`. Load it alongside the unchanged
`mainnet.deployment.json` with `--protected-royalty-deployment-path`. The four
new ordinary pool and DAO policy references are recorded in
`mainnet.new-ordinary-pool-reference-utxos.json`; that inventory is not a Bloom
deployment profile. Ordinary pool runtime routing needs its own additive wiring
before those four references can be used by Bloom.

`royaltyPoolWithdrawContractV2` is still the existing external policy
`a062da72f9f3280fbed90b3e95ea943aabd7eff9c82343324a9726ad`.
It is not part of the 13-script release and retains its existing reference.
Verify that policy's deployed bytes, hash, and reference UTxO before enabling
protected V2 withdrawal; the 13-script artifact check does not cover it.
The legacy `royaltyPool` and `royaltyPoolWithdrawContract` entries also retain
their existing hashes and references; protected V1 routing uses
`royaltyPoolLedgerFixed` and `royaltyPoolWithdrawContractLedgerFixed`.

The script artifact directory's README records the provenance and remaining
Mainnet release checks. Its `deploymentApproved` flag is currently `false`.
