# Proof of Achievement Report: AMM, Limit Order, and Auction Order Support in Bloom/Splash Off-Chain Agent

Project Catalyst milestone reference:
https://milestones.projectcatalyst.io/projects/1100283/milestones/2

This report describes the delivered Bloom/Splash off-chain service support for
AMM pools, limit orders, and auction orders. It also includes documentation on
building off-chain services with the library and an example real off-chain bot
integration. The preprod evidence shows that a limit order and auction order are
created, matched, executed by `bloom-cardano-agent`, and verified on-chain.

No wallet seed phrases, signing keys, Blockfrost keys, node socket contents, or
other secrets are included in this report.

Milestone requirement map:

- **Implementation supporting AMM pool, limit order, and auction order**:
  covered in section A with direct code links for AMM pool models/creation,
  limit order implementation/sending, and auction order
  implementation/execution.
- **Documentation on building off-chain services with the library**: covered in
  section E, including architecture, initialization, event/order handling,
  configuration, and a small end-to-end service example.
- **Example real off-chain bot integration**: covered by the
  `bloom-cardano-agent` executable links in section A and the live preprod
  execution evidence in sections B-D.

## A. Output: AMM Pool, Limit Order, and Auction Order Support

Acceptance criteria: The Bloom/Splash execution engine supports AMM pool
liquidity, limit order takers, and auction order takers. The service can ingest
pool/order UTxOs, classify them into domain entities, match compatible
liquidity, and build valid execution transactions.

Evidence:

- AMM pool support:
  - Generic pool classification for the execution engine:
    https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain-cardano/src/pools/classified.rs
  - Constant-function AMM pool ledger model and datum parsing:
    https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/spectrum-offchain-cardano/src/data/cfmm_pool.rs
  - AMM pool math used by pool transitions:
    https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/spectrum-offchain-cardano/src/pool_math/cfmm_math.rs
  - Preprod auditor AMM pool deployment script:
    https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/deploy-amm-pool.ts
  - Real pool-creation integration script:
    https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/splash-testing-cardano/src/balancePool.ts
  - Additional real pool deployment examples:
    - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/splash-testing-cardano/src/royaltyPool/deployPool.ts
    - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/splash-testing-cardano/src/stablePool/stablePool.ts
- Limit order support:
  - Limit order domain implementation and execution behavior:
    https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain-cardano/src/orders/limit.rs
  - Preprod auditor standalone limit-order sender:
    https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/create-limit-order.ts
  - Limit order transaction builder / sender integration script:
    https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/splash-testing-cardano/src/limitOrder.ts
  - Preprod auditor flow limit-order sender used for the verified run:
    https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/create-counter-limit-order.ts
  - The verified preprod order transaction includes the counter limit order at
    output `#0`:
    https://preprod.cexplorer.io/tx/8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440
- Auction order domain implementation:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain-cardano/src/orders/auction.rs
- Auction order wiring in Cardano event handling and entity decoding:
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-cardano-agent/src/entity.rs
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain-cardano/src/event_sink/handler.rs
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain-cardano/src/event_sink/context.rs
- Auction order execution transition support:
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain-cardano/src/execution_engine/instances.rs
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain-cardano/src/execution_engine/execution_state.rs
- Timed order clock support in the generic execution engine:
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain/src/execution_engine/types.rs
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain/src/execution_engine/mod.rs
- Liquidity book support for timed auction order activation:
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain/src/execution_engine/liquidity_book/core.rs
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain/src/execution_engine/liquidity_book/market_taker.rs
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain/src/execution_engine/liquidity_book/state/mod.rs
- Real off-chain bot integration:
  - `bloom-cardano-agent` executable composition:
    https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-cardano-agent/src/main.rs
  - agent configuration model:
    https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-cardano-agent/src/config.rs
  - preprod agent config example:
    https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-cardano-agent/resources/preprod.config.json

Auditor reference branch:

- https://github.com/splashprotocol/splash-offchain-multiplatform/tree/bromel777/auction-orders-support

Auditor reference pull request:

- https://github.com/splashprotocol/splash-offchain-multiplatform/pull/256

## B. Output: Preprod AMM, Limit Order, and Auction Auditor Flow

Acceptance criteria: Auditors can run one script from a fresh checkout, provide
a Blockfrost preprod key and Cardano node socket path, fund a fresh generated
wallet with tADA, and observe a full preprod flow that deploys AMM liquidity,
publishes a limit order, publishes a matching auction/counter limit-order pair,
and verifies auction execution by `bloom-cardano-agent`.

Evidence:

- Auditor script:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/run-amm-limit-auction-flow.sh
- AMM pool and standalone limit-order demo wrapper:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/run-amm-limit-demo.sh
- Standalone AMM pool deployment script:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/deploy-amm-pool.ts
- Standalone limit-order deployment script:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/create-limit-order.ts
- Auditor README:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/README.md
- Fresh preprod setup script:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/setup-preprod-flow.ts
- Paired limit/auction order creation script:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/create-order-pair.ts
- Execution verifier:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/verify-auction-flow.ts
- Limit-order publication verifier:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/verify-limit-order-flow.ts
- Local wallet and agent config helpers:
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/wallet-info.ts
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/agent-info.ts
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/generate-agent-config.sh
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/run-agent.sh

Auditor command:

```bash
./testing/preprod/amm-limit-auction-flow/run-amm-limit-auction-flow.sh
```

Additional AMM pool and limit-order demo command:

```bash
./testing/preprod/amm-limit-auction-flow/run-amm-limit-demo.sh
```

The script asks the auditor for:

- Preprod Blockfrost project id.
- Cardano node socket path. If a common socket path is detected, it is shown as
  the default and can be accepted with Enter.
- Funding confirmation after at least `500 tADA` is sent to the generated
  preprod wallet address.

On failure, the script kills the agent and removes the run-local RocksDB/state
directory so a later run starts cleanly. Wallet seed files remain local under
`.run/wallets` so leftover preprod funds can be recovered by the operator.

For deterministic auditor evidence the generated agent config sets
`disableMempool=true`. The flow waits for setup/order transactions to be
confirmed on preprod, then the agent observes them through ledger chain sync and
submits the execution transaction. This avoids non-deterministic local mempool
rollback events during repeated audit runs.

## C. Output: Tests and Local Verification

Acceptance criteria: The code and scripts are covered by local verification
commands that auditors and reviewers can run before executing a live preprod
flow.

Evidence:

Commands run successfully:

```bash
bash -n testing/preprod/amm-limit-auction-flow/run-amm-limit-auction-flow.sh
# exit code: 0, no output

bash -n testing/preprod/amm-limit-auction-flow/run-amm-limit-demo.sh
# exit code: 0, no output

bash -n testing/preprod/amm-limit-auction-flow/generate-agent-config.sh
# exit code: 0, no output

deno check --config testing/preprod/amm-limit-auction-flow/deno.json \
  testing/preprod/amm-limit-auction-flow/deploy-amm-pool.ts \
  testing/preprod/amm-limit-auction-flow/create-limit-order.ts \
  testing/preprod/amm-limit-auction-flow/create-order-pair.ts \
  testing/preprod/amm-limit-auction-flow/src/submit.ts \
  testing/preprod/amm-limit-auction-flow/setup-preprod-flow.ts \
  testing/preprod/amm-limit-auction-flow/verify-limit-order-flow.ts \
  testing/preprod/amm-limit-auction-flow/verify-auction-flow.ts \
  testing/preprod/amm-limit-auction-flow/wallet-info.ts
# exit code: 0

cargo check -p bloom-cardano-agent
# exit code: 0, existing repository warnings only

cargo test -p bloom-offchain-cardano orders::auction::tests
# exit code: 0
```

Observed Rust unit test result:

```text
running 1 test
test orders::auction::tests::computes_decayed_price_for_span ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 41 filtered out
```

`cargo check -p bloom-cardano-agent` completed successfully. Existing repository
warnings were emitted, but no build errors were reported.

## D. Output: Reproducible Preprod Execution Evidence

Acceptance criteria: The delivered auditor flow demonstrates real preprod
transactions that create AMM liquidity, publish a limit order, publish a
matching auction/counter limit-order pair, and execute the auction order through
`bloom-cardano-agent`.

Latest successful preprod auditor run:

- Run ID: `auditor-20260603-172625`
- Network: Cardano preprod
- Report file generated locally:
  `testing/preprod/amm-limit-auction-flow/.run/reports/auditor-20260603-172625.json`
- Final report status: `ok`
- Generated wallet address:
  `addr_test1qz73hwwm5ry5zckdj2zmwf5p0t0sydn0jgf6axv3dw9ncgf005pcv8djrx9nx2a9map76uvf5vea7t20pg2362kq5lcqzsjx4s`
- Agent funding address used by the report:
  `addr_test1qrhjakt8vtykk57h7sv9yvcms0yd60k9eulaxpfegpafxw2sjwuf4654k2qjnzrf8cmu8xwp3d8ujx6z69xjxtm6q6jqx36ehj`
- Agent funding amount: `50,000,000` lovelace
- Funding / mint transaction:
  `eecdb897a2cb1816a5ec51934c1aba27fb2db0139c2902b34d4eb25250cf00c3`
- AMM pool deployment transaction:
  `02b55c2ee92589f4510284cab33dbf96cfee107ec6193fe8f7e4dc97781d4703`
- AMM-pair limit order publication transaction:
  `55f5bf36091fccc7acf5584e27a58d5f5db793032337519849c05cb1b8e2cef4`
- Paired counter limit order and auction order transaction:
  `8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440`
- Counter limit order ref:
  `8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440#0`
- Auction order ref:
  `8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440#1`
- Agent execution transaction:
  `f1fcc5d146dbe6c9296c80818b156006261b8970e7df385e37000ca18b0d0a08`

Public explorer links:

- Funding / mint transaction:
  [eecdb897a2cb1816a5ec51934c1aba27fb2db0139c2902b34d4eb25250cf00c3](https://preprod.cexplorer.io/tx/eecdb897a2cb1816a5ec51934c1aba27fb2db0139c2902b34d4eb25250cf00c3).
  This confirms the generated run assets and agent funding outputs.
- AMM pool deployment transaction:
  [02b55c2ee92589f4510284cab33dbf96cfee107ec6193fe8f7e4dc97781d4703](https://preprod.cexplorer.io/tx/02b55c2ee92589f4510284cab33dbf96cfee107ec6193fe8f7e4dc97781d4703).
  This creates the classic AMM pool for the fresh run asset pair.
- AMM-pair limit order publication transaction:
  [55f5bf36091fccc7acf5584e27a58d5f5db793032337519849c05cb1b8e2cef4](https://preprod.cexplorer.io/tx/55f5bf36091fccc7acf5584e27a58d5f5db793032337519849c05cb1b8e2cef4).
  This publishes a limit order using the AMM pool asset pair. The auditor flow
  records this as `published_on_preprod`; the auction execution proof below also
  executes a counter limit order through the agent.
- Paired counter limit order and auction order transaction:
  [8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440](https://preprod.cexplorer.io/tx/8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440).
  This contains the counter limit order at output `#0` and auction order at
  output `#1`.
- Agent execution transaction:
  [f1fcc5d146dbe6c9296c80818b156006261b8970e7df385e37000ca18b0d0a08](https://preprod.cexplorer.io/tx/f1fcc5d146dbe6c9296c80818b156006261b8970e7df385e37000ca18b0d0a08).
  This is the transaction submitted by `bloom-cardano-agent` that spends auction
  order ref `8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440#1`
  and the matching counter limit order ref
  `8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440#0`.

Validator references used by the run:

- Auction validator hash:
  `fbc59e20bd55b1b521db154643fb359020bdfc8da40120946c89c08a`
- Auction validator reference:
  `7eccade32a21891469792da9b9ecbd9f0fed9835a591a109e63f6889d2118cf0#0`
- Limit order validator hash:
  `464eeee89f05aff787d40045af2a40a83fd96c513197d32fbc54ff02`
- Limit order validator reference:
  `77fc5b1688a79dd5b9bdba5981b7403b519690dda4f1471fffa78151bbefe0f5#0`
- Limit order witness validator hash:
  `96f5c1bee23481335ff4aece32fe1dfa1aa40a944a66d2d6edc9a9a5`
- Limit order witness validator reference:
  `77fc5b1688a79dd5b9bdba5981b7403b519690dda4f1471fffa78151bbefe0f5#1`

Generated test assets:

- Minting policy: `24c33fdd6e66e84023b6f01549aba36461a321a9ff67f2d68e04d535`
- AMM pool asset X name hex: `706f6f6c582d36303630332d313732363235`
- AMM pool asset Y name hex: `706f6f6c592d36303630332d313732363235`
- Auction base name hex: `61756374696f6e426173652d36303630332d313732363235`
- Auction quote name hex: `61756374696f6e51756f74652d36303630332d313732363235`
- Auction input amount: `1000` units of the generated base asset
- Counter order input amount: `2000` units of the generated quote asset
- Active auction price: `2/1`
- Counter order price: `1/2`
- Expected auction output: `2000` units of the generated quote asset
- Expected counter order output: `1000` units of the generated base asset

Verifier output from the run:

```json
{
  "status": "spent_by_agent_flow",
  "txHash": "8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440",
  "outputIndex": "1",
  "spendingTx": "f1fcc5d146dbe6c9296c80818b156006261b8970e7df385e37000ca18b0d0a08",
  "agentLog": "testing/preprod/amm-limit-auction-flow/.run/logs/auditor-20260603-172625/agent.log"
}
```

Generated JSON report from the run:

```json
{
  "status": "ok",
  "runId": "auditor-20260603-172625",
  "fundingTx": "eecdb897a2cb1816a5ec51934c1aba27fb2db0139c2902b34d4eb25250cf00c3",
  "ammPoolTx": "02b55c2ee92589f4510284cab33dbf96cfee107ec6193fe8f7e4dc97781d4703",
  "ammLimitTx": "55f5bf36091fccc7acf5584e27a58d5f5db793032337519849c05cb1b8e2cef4",
  "counterTx": "8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440",
  "auctionTx": "8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440",
  "auctionRef": "8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440#1",
  "executionTx": "f1fcc5d146dbe6c9296c80818b156006261b8970e7df385e37000ca18b0d0a08"
}
```

Agent log checkpoints from the run:

- The generated config contained `disableMempool: true`, so the proof used
  ledger-confirmed events for deterministic audit behavior.
- The AMM pool was observed from ledger and added to the active frontier.
- The AMM-pair limit order was observed from ledger and added to the active
  frontier.
- The liquidity book formed a batch containing the auction ask and counter limit
  bid.
- `AuctionOrder::exec(removed_input=1000, added_output=2000, consumed_budget=0, consumed_fee=0)`
  was executed.
- `LimitOrder::exec(removed_input=2000, added_output=1000, consumed_budget=358900, consumed_fee=500000)`
  was executed in the final fee-corrected transaction.
- The execution transaction
  `f1fcc5d146dbe6c9296c80818b156006261b8970e7df385e37000ca18b0d0a08` was
  accepted.
- The execution transaction was later confirmed and removed from pending
  transaction tracking.

## E. Output: Documentation for Building and Operating Off-Chain Services

Acceptance criteria: The repository includes Markdown documentation that
explains how to build an off-chain service with the Bloom/Splash libraries,
including minimal architecture, initialization, event/order handling,
configuration, and a small end-to-end example. It also includes auditor/operator
documentation for running the full preprod auditor flow and interpreting
success.

Evidence:

- General off-chain service guide:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/docs/offchain-service-guide.md
- Auditor flow README:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/README.md
- This Proof of Achievement report: `milestone-2.md` in the current branch.

### Building an Off-Chain Service with the Bloom/Splash Libraries

This guide explains how to assemble a Cardano off-chain service using the
Bloom/Splash off-chain libraries in this repository. It is intentionally
separate from the preprod auditor flow runbook: the runbook proves AMM pool,
limit-order, and auction-order behavior, while this section explains the
reusable service architecture and integration points.

The production example in this repository is `bloom-cardano-agent`. A smaller
service can reuse the same pieces and disable features it does not need.

#### Minimal Architecture

A Bloom/Splash off-chain service has six moving parts:

1. **Chain source**: reads ledger transactions from a Cardano node and turns
   them into typed transaction views. Code:
   https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/cardano-chain-sync/src/lib.rs
   and agent wiring in
   https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-cardano-agent/src/main.rs
2. **Mempool source**: optionally reads unconfirmed transactions so the service
   can react before ledger confirmation. Code:
   https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/cardano-mempool-sync/src/lib.rs
   and agent wiring in
   https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-cardano-agent/src/main.rs
3. **Event handlers**: classify transaction outputs and inputs into domain
   events such as pool updates, order creation, order elimination, and funding
   box changes. Code:
   https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain-cardano/src/event_sink/handler.rs
4. **Execution engine**: keeps per-pair liquidity books, matches compatible
   orders/pools, and produces execution recipes. Code:
   https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain/src/execution_engine/mod.rs
   and liquidity book implementation in
   https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain/src/execution_engine/liquidity_book/mod.rs
5. **Cardano interpreter**: converts recipes into Cardano transactions using
   deployed validator references, collateral, funding boxes, and operator
   credentials. Code:
   https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain-cardano/src/execution_engine/interpreter.rs
   and order/pool execution instances in
   https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain-cardano/src/execution_engine/instances.rs
6. **Submission and health services**: submit transactions, track confirmation,
   report execution status, and expose a health endpoint. Code:
   https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/spectrum-offchain-cardano/src/tx_submission.rs
   and
   https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain/src/health.rs

In `bloom-cardano-agent`, these pieces are wired in
`bloom-cardano-agent/src/main.rs`.

#### Crate Roles

- `bloom-offchain`: generic execution engine, temporal liquidity book, state
  index, backlog, matching, and execution stream orchestration.
- `bloom-offchain-cardano`: Cardano-specific order/pool decoding, event
  handlers, recipe interpretation, validation rules, and transaction building.
- `spectrum-offchain-cardano`: Cardano data types, validator deployment models,
  credentials, transaction submission, and node helpers.
- `cardano-chain-sync` and `cardano-mempool-sync`: node-to-client ledger and
  mempool streams.
- `bloom-cardano-agent`: executable composition of the above libraries.

#### Initialization Flow

A service should initialize components in this order:

1. **Load configuration**:
   - service config, for example
     `bloom-cardano-agent/resources/preprod.config.json`;
   - validator deployment config, for example
     `bloom-cardano-agent/resources/preprod.deployment.json`;
   - validation rules, for example
     `bloom-cardano-agent/resources/validation-rules.json.template`;
   - logging config, for example `bloom-cardano-agent/resources/log4rs.yaml`.
2. **Validate configuration** with the local integrity checks. The agent uses
   `CheckIntegrity` to reject malformed partitioning, fee, and graduation
   settings before connecting to the node.
3. **Create explorer access** through `cardano_explorer::AnyExplorer`. The
   explorer is used to pull validator reference UTxOs and collateral/funding
   state.
4. **Pull deployed validators** with `ProtocolDeployment::unsafe_pull`.
5. **Build optional order registries**, such as `AuctionOrderRegistry`, from
   configured validator references and cost limits.
6. **Open local state**:
   - chain-sync RocksDB cache;
   - optional graduation RocksDB state;
   - in-memory entity/order/funding indexes.
7. **Connect node clients**:
   - `ChainSyncClient` for ledger events;
   - `LocalTxMonitorClient` for mempool events;
   - local transaction submission client.
8. **Derive operator credentials**, collateral address, and funding addresses
   from the operator signing key.
9. **Create event channels and handlers** for pair updates, specialized order
   updates, and funding events.
10. **Create execution contexts** and start one or more execution streams.

The agent uses four execution streams in production. A minimal service can use
one partition and one execution stream.

#### Configuration Checklist

The minimal config fields are:

- `chainSync.startingPoint`: ledger point where the service starts scanning.
- `chainSync.replayFromPoint`: optional earlier point for replaying indexed
  events.
- `chainSync.disableRollbacksUntil`: rollback guard point.
- `chainSync.dbPath`: local RocksDB path for the ledger cache.
- `node.path`: Cardano node socket path.
- `node.magic`: network magic, for example `1` for preprod.
- `networkId`: Cardano network id, for example `0` for testnet.
- `operatorKey`: operator signing key material or configured key source.
- `explorer`: Blockfrost or Maestro configuration, depending on the build and
  deployment environment.
- `execution.executionCap`: soft and hard execution-unit caps for batches.
- `execution.o2oAllowed`: whether order-to-order matching is enabled.
- `partitioning`: total partitions and assigned partitions for this service
  instance.
- `daoConfig`, `royaltyWithdraw`, and other protocol-specific contexts required
  by enabled validators.
- `healthListenAddr`: optional HTTP health endpoint.

Auction order support additionally requires:

```json
{
  "auctionOrders": [
    {
      "validator": {
        "hash": "<auction-validator-hash>",
        "referenceUtxo": {
          "txHash": "<reference-script-tx>",
          "outputIndex": 0
        },
        "cost": {
          "mem": 210000,
          "steps": 80000000
        },
        "marginalCost": {
          "mem": 210000,
          "steps": 80000000
        }
      },
      "maxCostPerExStep": {
        "asset": "lovelace",
        "quantity": 1
      },
      "minMarginalOutput": {
        "asset": "out",
        "quantity": 1
      }
    }
  ]
}
```

Use the deployed values for the target network. Do not hardcode private keys,
API keys, or local node socket paths into committed config.

#### Event and Order Handling

The service receives `TxViewMut` values from ledger and mempool streams. Event
handlers inspect each transaction and produce typed domain events:

- **Pair updates** for evolving entities such as pools.
- **Specialized order updates** for atomic order entities such as limit,
  instant, grid, and auction orders.
- **Funding events** for operator funding boxes used by transaction building.

The main handlers are:

- `PairUpdateHandler`: classifies pool and evolving entity transitions.
- `SpecializedHandler`: classifies order creation and elimination.
- `FundingEventHandler`: tracks funding boxes belonging to operator-derived
  funding addresses.

Order support is usually added in four places:

1. **Domain decoder**: parse the on-chain datum/redeemer into a Rust order type.
   Auction orders are implemented in
   `bloom-offchain-cardano/src/orders/auction.rs`.
2. **Entity wiring**: include the order in the agent entity enum so ledger and
   mempool handlers can see it. See `bloom-cardano-agent/src/entity.rs`.
3. **Event handling context**: provide validator hashes, deployment references,
   and validation rules through `HandlerContext`.
4. **Execution instances**: implement how the order executes and how the Cardano
   interpreter spends it. See
   `bloom-offchain-cardano/src/execution_engine/instances.rs`.

Auction orders are timed taker orders. The liquidity book advances clocks from
ledger time, activates the order only in its valid time span, computes the
current auction price, and matches it against compatible counter liquidity.

#### Execution Engine Flow

The execution stream consumes pair/order/funding events and performs this loop:

1. Resolve the latest known state of every affected entity through the state
   index.
2. Update the per-pair `TLB` liquidity book.
3. Select candidate takers and makers for the pair.
4. Form a `MatchmakingRecipe` when prices, time bounds, and execution caps allow
   a valid batch.
5. Convert the recipe into a Cardano transaction blueprint.
6. Pull collateral and funding boxes.
7. Balance fees and execution budgets.
8. Submit the transaction through the local submission agent.
9. Track mempool acceptance and ledger confirmation.
10. Commit or roll back predicted state depending on confirmation.

The generic stream entry point is `execution_part_stream` in
`bloom-offchain/src/execution_engine/mod.rs`.

#### Minimal End-to-End Example

This example describes the smallest practical service that follows the same
architecture as `bloom-cardano-agent`.

1. Create a config file for preprod:

```json
{
  "eventFeedBufferSize": 1024,
  "chainSync": {
    "startingPoint": { "Specific": [64919047, "<block-hash>"] },
    "replayFromPoint": { "Specific": [64919047, "<block-hash>"] },
    "disableRollbacksUntil": 64919047,
    "dbPath": "state/preprod-chain-sync.rocksdb"
  },
  "node": {
    "path": "/path/to/node.socket",
    "magic": 1
  },
  "txSubmissionBufferSize": 64,
  "backlogCapacity": 128,
  "networkId": 0,
  "eventCacheTtl": { "secs": 120, "nanos": 0 },
  "operatorKey": "<operator-signing-key>",
  "takeResidualFee": false,
  "explorer": {
    "blockfrostKeyPath": "secrets/preprod.blockfrost.key"
  },
  "execution": {
    "executionCap": {
      "soft": { "mem": 5000000, "steps": 4000000000 },
      "hard": { "mem": 14000000, "steps": 10000000000 }
    },
    "o2oAllowed": true
  },
  "eventFeedBufferingDuration": { "secs": 0, "nanos": 50000 },
  "partitioning": {
    "numPartitionsTotal": 1,
    "assignedPartitions": [0]
  },
  "daoConfig": {
    "publicKeys": [],
    "threshold": 0,
    "exFee": 1500000
  },
  "royaltyWithdraw": {
    "transactionFee": 1500000
  },
  "reportingEndpoint": "127.0.0.1:9021",
  "healthListenAddr": "127.0.0.1:9024",
  "auctionOrders": []
}
```

2. Provide deployment and validation files:

```text
bloom-cardano-agent/resources/preprod.deployment.json
bloom-cardano-agent/resources/validation-rules.json.template
```

3. Build and run the agent:

```bash
cargo run -p bloom-cardano-agent -- \
  --config-path path/to/preprod.config.json \
  --deployment-path bloom-cardano-agent/resources/preprod.deployment.json \
  --validation-rules-path bloom-cardano-agent/resources/validation-rules.json.template \
  --log4rs-path bloom-cardano-agent/resources/log4rs.yaml
```

4. Fund the derived operator funding addresses and collateral address.

5. Publish supported order or pool UTxOs on-chain.

6. Watch logs for:

```text
Successfully formed a batch
Finished Tx: <transaction-hash>
Tx <transaction-hash> was accepted
Removed confirmed Tx <transaction-hash>: true
```

7. Query the health endpoint if configured:

```bash
curl http://127.0.0.1:9024/health
```

The preprod auditor flow in
`testing/preprod/amm-limit-auction-flow/run-amm-limit-auction-flow.sh` is a
concrete example of steps 1-6. It generates a wallet, asks for external inputs,
funds the agent, deploys AMM liquidity, publishes a limit order, publishes a
matching auction/counter limit-order pair, waits for execution, and writes a
JSON report.

#### Operational Notes

- Start chain sync far enough before the order UTxOs so the service sees
  validator references, funding boxes, and orders.
- Keep RocksDB paths unique per run or clean them before replaying the same
  scenario.
- Keep operator keys, API keys, and wallet seed files outside committed docs.
- Prefer a health endpoint in auditor and production deployments.
- Use mempool support for faster reaction; disable it only when a deployment
  intentionally wants ledger-confirmed events only.
- When adding a new order family, add a small unit test for order math and a
  live-network runbook or integration flow showing that the order is observed,
  matched, submitted, and confirmed.

The preprod auditor flow README documents:

- The one-command auditor entry point.
- Required inputs: Blockfrost preprod key, Cardano node socket, and tADA
  funding.
- What the script does at each stage.
- Expected success output.
- Local verification commands.
- Last verified preprod run and transaction hashes.

## Summary

The milestone delivery includes the auction order implementation and wiring
referenced above, plus a demonstrated preprod execution path, a reproducible
auditor flow, local verification commands, Markdown documentation, and concrete
preprod transaction evidence.

The latest completed preprod run demonstrates that:

1. Fresh test assets were minted on preprod.
2. A classic AMM pool was deployed for the generated run asset pair.
3. A limit order using that AMM pool asset pair was published on preprod.
4. A matching counter limit order and auction order were published in one
   transaction.
5. `bloom-cardano-agent` observed the pool and orders from ledger sync.
6. The liquidity book formed a valid auction/counter limit-order batch.
7. The auction order and counter limit-order execution transitions ran.
8. The agent submitted execution transaction
   `f1fcc5d146dbe6c9296c80818b156006261b8970e7df385e37000ca18b0d0a08`.
9. The verifier confirmed that the auction order UTxO was spent by the agent
   flow.

Sensitive-information confirmation: this Proof of Achievement contains public
repository paths, public preprod transaction hashes, public preprod addresses,
and public asset ids. It does not include wallet seeds, private keys, API keys,
local socket paths, or local state paths.
