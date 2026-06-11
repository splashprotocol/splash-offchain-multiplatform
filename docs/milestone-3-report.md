# Milestone 3 Completion Report

## Scope

Milestone 3 requested two outputs:

1. Order Steering implementation in library components.
2. Off-chain indexer service for on-chain data relevant to execution assessment.

This report covers both implementation and auditor evidence.

## Repository and Revision Context

- GitHub repository:
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform`
- working branch used for this report:
  `bromel777/batcher-indexer-app`
- audited source revision for the milestone-3 implementation and rerun
  instructions:
  `2c6904237cb229cb787fee41ccbd10fcb434990d`
- branch source browser root:
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform/tree/bromel777/batcher-indexer-app`
- immutable source browser root for the audited revision:
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform/tree/2c6904237cb229cb787fee41ccbd10fcb434990d`

At the time this report was generated, the milestone evidence was validated from
the local branch and local preprod runs listed below. A GitHub pull request URL
is therefore not embedded here. Auditors should use the repository URL above
plus the branch or commit listed here when cross-checking the implementation.

## Delivered Outputs

### 1. Order Steering in library components

Order Steering is implemented for limit orders by carrying a permitted executors
list directly in the order datum and enforcing it in off-chain execution logic.

Key implementation points:

- Limit-order datum includes `permitted_executors`:
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform/blob/bromel777/batcher-indexer-app/bloom-offchain-cardano/src/orders/limit.rs#L268`
- Parsed observations preserve the executor allowlist and derive
  `requires_executor_sig` from it:
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform/blob/bromel777/batcher-indexer-app/bloom-offchain-cardano/src/orders/limit.rs#L284`
- Execution eligibility is checked against the operator credential:
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform/blob/bromel777/batcher-indexer-app/bloom-offchain-cardano/src/orders/limit.rs#L341`
- Ledger decoding rejects orders for operators not present in the allowlist:
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform/blob/bromel777/batcher-indexer-app/bloom-offchain-cardano/src/orders/limit.rs#L505`
- When the order is steered, the execution engine adds the operator as a
  required signer:
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform/blob/bromel777/batcher-indexer-app/bloom-offchain-cardano/src/execution_engine/instances.rs#L166`

Operational meaning:

- If `permitted_executors` is empty, the order is permissionless.
- If `permitted_executors` contains one or more operator key hashes, only those
  operators may execute the order.
- This is the on-chain steering primitive. Users choose a preferred batcher set
  by placing that set into the order datum.

### 2. Off-chain execution-assessment indexer

The repository now contains a batcher execution indexer flow and an integrated
auditor harness.

Primary components:

- Indexer flow:
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform/blob/bromel777/batcher-indexer-app/testing/preprod/batcher-indexer-flow/run-batcher-indexer-flow.sh`
- Indexer README:
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform/blob/bromel777/batcher-indexer-app/testing/preprod/batcher-indexer-flow/README.md`
- AMM + indexer auditor flow:
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform/blob/bromel777/batcher-indexer-app/testing/preprod/amm-limit-indexer-flow/run-amm-limit-indexer-flow.sh`
- AMM + indexer README:
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform/blob/bromel777/batcher-indexer-app/testing/preprod/amm-limit-indexer-flow/README.md`

What the indexer proves:

- batcher discovery through observed execution signers;
- synced HTTP API availability;
- per-batcher execution metrics from on-chain activity;
- execution quality information that can be used by users to choose a better
  batcher before steering future orders to that executor.

In other words:

- steering is expressed on-chain via `permitted_executors`;
- batcher choice can be informed off-chain via indexer metrics such as capture
  rate and response time.

## Acceptance Criteria Mapping

### Bot implementation for Order Steering support

Satisfied by:

- limit-order datum support for `permitted_executors`;
- off-chain filtering of executable orders by operator credential;
- execution transaction signer enforcement for steered orders.

### Off-chain indexer's ability to index on-chain data for execution assessment

Satisfied by:

- synced indexer HTTP flow;
- batcher discovery through `/batchers`;
- execution metrics through `/batchers/{pkh}/metrics`;
- automated auditor harness that creates fresh orders, waits for execution, and
  validates indexer results against those fresh orders.

## Auditor Evidence

### Evidence baseline run

The milestone evidence in this report is based on the latest fully completed
successful integrated auditor run:

- run id: `amm-limit-indexer-20260610-093703`
- top-level generated report path:
  `testing/preprod/amm-limit-indexer-flow/.run/reports/amm-limit-indexer-20260610-093703.json`
- indexer generated report path:
  `testing/preprod/batcher-indexer-flow/.run/reports/amm-limit-indexer-20260610-093703-indexer.json`

### What that run demonstrated

- fresh wallet funding:
  `62b7839c32f7f0014bbfe925e9c262341b816e59e0cabdc0754a8b8a87313775`
- fresh royalty-pool deployment:
  `5f476de826e34fea6be69a38887c0aa67113c414ff51934161cbdbd4d6b5731b`
- `10` executable AMM limit orders in separate transactions;
- `10` corresponding execution transactions;
- `1` deliberately non-executable AMM limit order that remained open for the
  full observation window.

Later reruns may exist in `.run/runs/` as soak or retry attempts. They are not
used as milestone evidence unless they also complete and emit matching reports.

### Verified results from the successful run

From the top-level run report:

- `10` good order transaction hashes are recorded.
- `10` good execution transaction hashes are recorded.
- bad order tx:
  `24124d10d18cd63b42139cbb62b46dfe271ca4788aafab7c8e0e443dd2c7802b`
- bad order final status:
  `still_open_after_window`
- observation window:
  `180` seconds

From the indexer report:

- `status = ok`
- `completeness = synced`
- `healthStatus = 200`
- `fromMs = 1781084493000`
- `toMs = 1781087353443`
- resolved chain point:
  - `slot = 125401287`
  - `hash = 9e8ddaa87ebaa5a33fc453b14ae7a30f22eb45e558fd9910866b316a6bfd1bf8`
  - `provider = blockfrost`
  - `blockHeight = 4806555`
- fast-forwarded script activity:
  - address: `addr_test1wpryamhgnuz6lau86sqytte2gz5rlktv2yce05e0h3207qst4n9nh`
  - tx: `6675f5b61ae951a4f4793dea0f216259548bf2f675b5a17739b5c395c15dd83a`
- batcher:
  `cc7dbe5cc9cfa8046adc7bbbe8316cd598d55ef90dda77c5bc1eb6fa`
- `eligibleOrders = 10`
- `executedOrders = 10`
- `stillOpenEligibleOrders = 0`
- `missedEligibleOrders = 0`
- `captureRate = 1.0000`
- `medianResponseMs = 18000`
- `p95ResponseMs = 48000`
- `ambiguousExecutions = 0`
- `unknownExecutions = 0`
- eligible input volume:
  `20000` units of
  `79886abfd815c1c9444145232169acfcca1d3a56f169ac01c59e273c.poolY-60610-093703`
- executed input volume:
  `20000` units of
  `79886abfd815c1c9444145232169acfcca1d3a56f169ac01c59e273c.poolY-60610-093703`
- executed output volume:
  `0` units of
  `79886abfd815c1c9444145232169acfcca1d3a56f169ac01c59e273c.poolX-60610-093703`

These values satisfy the milestone execution-assessment invariant:

```text
eligibleOrders = executedOrders + stillOpenEligibleOrders + missedEligibleOrders
```

For this run:

```text
10 = 10 + 0 + 0
```

## How auditors can run it locally

### Preconditions

- repository checkout;
- checkout branch `bromel777/batcher-indexer-app`;
- checkout the audited revision used by this report:

  ```bash
  git checkout 2c6904237cb229cb787fee41ccbd10fcb434990d
  ```

- preprod Cardano node socket;
- Blockfrost preprod project id;
- `500 tADA` available to fund the temporary wallet printed by the harness;
- build toolchain for the Rust and Deno components already used by this repo.
- required command-line tools available in the local environment:
  `cargo`, `deno`, `curl`, `jq`, `lsof`, `python3`, `perl`, `pkill`
- for the standalone batcher indexer flow only:
  - historical datetime mode works with Blockfrost and does not require
    `cardano-cli`
  - `BATCHER_INDEXER_FROM=now` currently requires a working `cardano-cli`
    because the script queries the local node tip directly

### Integrated AMM + indexer proof

Run:

```bash
bash testing/preprod/amm-limit-indexer-flow/run-amm-limit-indexer-flow.sh
```

The script prompts for:

- Cardano node socket path;
- Blockfrost preprod project id;
- funding transfer confirmation after it prints a fresh wallet address.
- depending on local Deno permission cache, an FFI permission prompt may appear;
  allow it for the run.

Committed helper scripts used by this integrated flow live under:

- `https://github.com/splashprotocol/spectrum-offchain-multiplatform/tree/bromel777/batcher-indexer-app/testing/preprod/amm-limit-auction-flow`

In particular, the integrated wrapper calls the checked-in royalty pool helper:

- `https://github.com/splashprotocol/spectrum-offchain-multiplatform/blob/bromel777/batcher-indexer-app/testing/preprod/amm-limit-auction-flow/deploy-royalty-pool.ts`

Auditors do not need to create or patch any local helper scripts. The rerun is
expected to work from the committed branch contents plus local runtime inputs
only.

What it does:

1. creates fresh generated state;
2. starts a fresh Bloom agent;
3. deploys a fresh royalty AMM pool;
4. submits `10` executable AMM limit orders in separate transactions;
5. submits `1` non-executable AMM limit order;
6. verifies the `10` good orders are executed;
7. verifies the bad order remains open;
8. runs the batcher indexer over that exact execution window;
9. writes JSON reports for auditors.

Auditor rerun checklist:

1. open the repository on branch `bromel777/batcher-indexer-app`;
2. run the integrated harness command above from repository root;
3. enter the local preprod node socket path;
4. enter the Blockfrost preprod project id;
5. wait for the script to print a temporary wallet address;
6. fund that wallet with at least `500 tADA`;
7. confirm the transfer by pressing `Enter`;
8. wait for the script to finish all `10` good orders, `1` bad order, and the
   nested indexer run;
9. open the generated top-level and indexer JSON reports;
10. verify that:
    - there are `10` good order transaction hashes;
    - there are `10` good execution transaction hashes;
    - the bad order ends in `still_open_after_window`;
    - indexer metrics show `eligibleOrders = 10` and `executedOrders = 10`.

Outputs:

- top-level reports:
  `testing/preprod/amm-limit-indexer-flow/.run/reports/`
- indexer reports:
  `testing/preprod/batcher-indexer-flow/.run/reports/`

### Standalone batcher indexer flow

Run:

```bash
bash testing/preprod/batcher-indexer-flow/run-batcher-indexer-flow.sh
```

This standalone flow is useful when auditors already know the historical
execution window they want to inspect.

Recommended auditor mode for the standalone flow:

1. choose a historical ISO-8601 UTC start datetime rather than `now`;
2. provide the preprod node socket path;
3. provide the Blockfrost preprod project id when prompted;
4. wait for the script to produce the report under
   `testing/preprod/batcher-indexer-flow/.run/reports/`;
5. verify that the report status is `ok`, completeness is `synced`, and the
   per-batcher metric invariant holds.

## Repository evidence

Implementation and proof material are present in this repository:

- order steering logic in:
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform/blob/bromel777/batcher-indexer-app/bloom-offchain-cardano/src/orders/limit.rs#L268`
- execution signer enforcement in:
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform/blob/bromel777/batcher-indexer-app/bloom-offchain-cardano/src/execution_engine/instances.rs#L166`
- batcher indexer harness:
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform/tree/bromel777/batcher-indexer-app/testing/preprod/batcher-indexer-flow`
- integrated AMM auditor harness:
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform/tree/bromel777/batcher-indexer-app/testing/preprod/amm-limit-indexer-flow`
- successful milestone generated evidence paths:
  `testing/preprod/amm-limit-indexer-flow/.run/reports/amm-limit-indexer-20260610-093703.json`
  and
  `testing/preprod/batcher-indexer-flow/.run/reports/amm-limit-indexer-20260610-093703-indexer.json`

## Conclusion

Milestone 3 is complete.

- Order Steering is implemented through the on-chain `permitted_executors` list
  in limit-order datum and enforced by off-chain execution logic.
- The batcher execution indexer is implemented and demonstrated on fresh preprod
  activity.
- The repository contains code, documentation, reproducible scripts, and JSON
  auditor evidence for independent verification.
