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

### Supplemental pre-submission verification rerun

After the milestone baseline run above, the integrated flow was rerun again on
June 11, 2026 as a fresh pre-submission verification pass:

- run id:
  `amm-limit-indexer-20260611-091343`
- run root:
  `testing/preprod/amm-limit-indexer-flow/.run/runs/amm-limit-indexer-20260611-091343`

This rerun is supplementary evidence. It demonstrates that the current branch
still executes the AMM proof flow on preprod. The authoritative milestone
acceptance baseline remains the fully completed successful run
`amm-limit-indexer-20260610-093703` described above.

Confirmed setup and pool transactions from the June 11 rerun:

- setup tx:
  `7eccade32a21891469792da9b9ecbd9f0fed9835a591a109e63f6889d2118cf0`
- funding / mint tx:
  `5d60cf33bd88d91b9e4f6163a0f0df20fc232096cf9ac8dfde971bd1be3c9401`
- royalty pool deploy tx:
  `af26448de9e041fe887f981b2a5cebadde157a084407ca7a3670cf3ecd13e2b7`

Good-order publication and execution transactions recorded for this rerun:

| Order | Create tx | Execution tx |
| --- | --- | --- |
| `good-01` | `efe67ca6f617384d58acc3987e2cf72f648495105a5037a5024e6515c56ca742` | `b410390bf5fb7f16f371d014b3908e15898fbde5a7e806c24f82a71e7b5d7901` |
| `good-02` | `cc7b5b5257220cd2e0994e312c26ef6b1d079e8722017af9ba63544f2be95c4b` | `6f81f7834f340558a522831f096ef8e01bbffdb1b5fa977d7be2c0dab66fd8f2` |
| `good-03` | `e00294515fe790a94ab83baede482064746efb097d4b56dba0a73deca13ca42b` | `19e47686fb42ef5560a903f36bda5ca3ac52567f3f021e1e80514d613b20547c` |
| `good-04` | `833b8f44fa3b822e47ec6ff2d48eb29010e1b1591b977c4e9ef54ab863ac8d08` | `016c188f8caefbb48ce7e7cc9bb20bf52b38e169e6973b2f1fabbf57c6967869` |
| `good-05` | `1bd370c42b47741272ff4d5edbd3798cf4d5c91f4b85718a13f45e9c366760fa` | `c3f85087f61d58a53340e33a3e1b86f514eb1a568db97a7f8dfcda178ec4141e` |
| `good-06` | `717ab574dcce5f1e42f78d25c3e7139236ac745ec220b691b08913c2625a788e` | `65851b881df782a0b5f1e2548b986a25d7002cd1c0bf0db7f7e479817f1cb867` |
| `good-07` | `3a004aa912bef813d0042634ab48e43ce463a9ea2bac7bd28c437f159a1a1d49` | `1b54ccce85ec755bb6e7666f8ba087eb51ce0ec6414fde07daeff6f8d035362a` |
| `good-08` | `ab7c414576be04c1af1ecac04ad2864d5000b649fd41d7112acf70a6c492074f` | `17e6f547d059f94fd4e30a3285926db4ac2d72154f873839340a7bca6381cfd0` |
| `good-09` | `0fef95ffb28015147e2d85731d8cf20a90f5c83659f767179eaef042c93c0174` | `f6a5a14be5e5c3b1a93a014831fff2c4c560163df57de48657d1e6b1dbdef2de` |
| `good-10` | `2dadee425f024432226aa53c14e98c8b7928432dbf1efac506d2e1af1ec68d27` | `201abe8fca420f7468a56e4b0e6581e22f5d94aedd9633c6aeb5a1afe6ae6a97` |

Bad-order transaction recorded for this rerun:

- bad-order create tx:
  `32a9622b35f72d717bb1d461107ece552d732b6a3020e60da52fc4ba2d475d71`
- bad-order final verifier state:
  `still_open_after_window`
- observation window:
  `180` seconds

Relevant local artifact paths for this rerun:

- good-order artifacts:
  `testing/preprod/amm-limit-indexer-flow/.run/runs/amm-limit-indexer-20260611-091343/logs/orders/good/`
- bad-order artifacts:
  `testing/preprod/amm-limit-indexer-flow/.run/runs/amm-limit-indexer-20260611-091343/logs/orders/bad/`

Verifier file format note:

- `good-01-verify.json`, `good-05-verify.json`, `good-06-verify.json`,
  `good-08-verify.json`, `good-09-verify.json`, and `bad-verify.json` contain
  intermediate polling snapshots appended as newline-delimited JSON before the
  final result.
- for auditor interpretation, the final JSON object in each verifier file is the
  authoritative terminal status.

At the time this report was refreshed, this June 11 rerun had already proven:

- fresh setup and funding on preprod;
- royalty pool deployment and parsing by `bloom-cardano-agent`;
- `10` separate good AMM limit-order executions;
- `1` separate bad AMM limit order remaining open for the full observation
  window.

The nested batcher-indexer report for this supplementary rerun is not used as
the milestone acceptance baseline in this document. The completed milestone
baseline remains `amm-limit-indexer-20260610-093703`.

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
