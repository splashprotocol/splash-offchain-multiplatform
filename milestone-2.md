# Proof of Achievement Report: Milestone 2

Project Catalyst milestone reference:
https://milestones.projectcatalyst.io/projects/1100283/milestones/2

This proof covers the delivered Bloom/Splash off-chain service support for AMM
pools, limit orders, and auction orders. It also includes documentation on
building off-chain services with the library and an example real off-chain bot
integration. The preprod evidence shows that a limit order and auction order are
created, matched, executed by `bloom-cardano-agent`, and verified on-chain.

No wallet seed phrases, signing keys, Blockfrost keys, node socket contents, or
other secrets are included in this report.

Auditor reference branch:
https://github.com/splashprotocol/splash-offchain-multiplatform/tree/bromel777/auction-orders-support

Auditor reference pull request:
https://github.com/splashprotocol/splash-offchain-multiplatform/pull/256

## Milestone Output 1 — Implementation Supporting AMM Pool, Limit Order, and Auction Order

Description:

The Bloom/Splash execution engine supports AMM pool liquidity, limit order
takers, and auction order takers. The service can ingest pool/order UTxOs,
classify them into domain entities, match compatible liquidity, and build valid
execution transactions.

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
- Auction order support:
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

Acceptance Criteria Verification:

- Review the AMM evidence links and verify that pool classification, constant
  function pool parsing, pool math, and pool deployment scripts are present.
- Review the limit order evidence links and verify that the order domain logic,
  execution logic, and transaction builders are present.
- Review the auction order evidence links and verify that auction order domain
  parsing, entity wiring, event handling, timed activation, and execution
  transition support are present.
- Confirm that the verified preprod order transaction contains the counter limit
  order at output `#0`:
  https://preprod.cexplorer.io/tx/8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440

## Milestone Output 2 — Preprod AMM, Limit Order, and Auction Auditor Flow

Description:

Auditors can run one script from a fresh checkout, provide a Blockfrost preprod
key and Cardano node socket path, fund a fresh generated wallet with tADA, and
observe a full preprod flow that deploys AMM liquidity, publishes a limit order,
publishes a matching auction/counter limit-order pair, and verifies auction
execution by `bloom-cardano-agent`.

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

Acceptance Criteria Verification:

- From the repository root, run:

  ```bash
  ./testing/preprod/amm-limit-auction-flow/run-amm-limit-auction-flow.sh
  ```

- When prompted, provide:
  - a preprod Blockfrost project id;
  - a Cardano node socket path;
  - funding confirmation after sending at least `500 tADA` to the generated
    preprod wallet address.
- Verify that the script:
  - starts `bloom-cardano-agent` with fresh run-local state;
  - funds the agent funding addresses;
  - deploys AMM liquidity;
  - publishes a limit order;
  - publishes a matching auction/counter limit-order pair;
  - verifies that the auction order was spent by the agent execution
    transaction.
- Verify that on failure the script kills the agent and removes run-local
  RocksDB/state directories so later runs start cleanly.
- Verify that the generated agent config sets `disableMempool=true`, waits for
  setup/order transactions to be confirmed on preprod, and then uses ledger
  chain sync for deterministic audit behavior.
- Optional AMM pool and limit-order demo command:

  ```bash
  ./testing/preprod/amm-limit-auction-flow/run-amm-limit-demo.sh
  ```

## Milestone Output 3 — Tests and Local Verification

Description:

The code and scripts are covered by local verification commands that auditors
and reviewers can run before executing a live preprod flow.

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

Acceptance Criteria Verification:

- Run the shell syntax checks listed above and verify they exit with code `0`.
- Run the Deno type-check command listed above and verify it exits with code
  `0`.
- Run `cargo check -p bloom-cardano-agent` and verify there are no build errors.
  Existing repository warnings are acceptable.
- Run `cargo test -p bloom-offchain-cardano orders::auction::tests` and verify
  that `computes_decayed_price_for_span` passes.

## Milestone Output 4 — Reproducible Preprod Execution Evidence

Description:

The delivered auditor flow demonstrates real preprod transactions that create
AMM liquidity, publish a limit order, publish a matching auction/counter
limit-order pair, and execute the auction order through `bloom-cardano-agent`.

Evidence:

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
  https://preprod.cexplorer.io/tx/eecdb897a2cb1816a5ec51934c1aba27fb2db0139c2902b34d4eb25250cf00c3
- AMM pool deployment transaction:
  https://preprod.cexplorer.io/tx/02b55c2ee92589f4510284cab33dbf96cfee107ec6193fe8f7e4dc97781d4703
- AMM-pair limit order publication transaction:
  https://preprod.cexplorer.io/tx/55f5bf36091fccc7acf5584e27a58d5f5db793032337519849c05cb1b8e2cef4
- Paired counter limit order and auction order transaction:
  https://preprod.cexplorer.io/tx/8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440
- Agent execution transaction:
  https://preprod.cexplorer.io/tx/f1fcc5d146dbe6c9296c80818b156006261b8970e7df385e37000ca18b0d0a08

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

Acceptance Criteria Verification:

- Open the public explorer links and verify the funding/mint, AMM pool
  deployment, limit order publication, paired counter/auction order, and agent
  execution transactions are present on preprod.
- Verify that the paired order transaction contains the counter limit order at
  output `#0` and auction order at output `#1`.
- Verify that the agent execution transaction spends auction order ref
  `8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440#1` and the
  matching counter limit order ref
  `8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440#0`.
- Verify that the generated JSON report status is `ok`.
- Verify that the verifier status is `spent_by_agent_flow` and the spending
  transaction is
  `f1fcc5d146dbe6c9296c80818b156006261b8970e7df385e37000ca18b0d0a08`.

## Milestone Output 5 — Documentation on Building and Operating Off-Chain Services

Description:

The repository includes Markdown documentation that explains how to build an
off-chain service with the Bloom/Splash libraries, including minimal
architecture, initialization, event/order handling, configuration, and a small
end-to-end example. It also includes auditor/operator documentation for running
the full preprod auditor flow and interpreting success.

Evidence:

- General off-chain service guide:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/docs/offchain-service-guide.md
- Auditor flow README:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/README.md
- This Proof of Achievement report: `milestone-2.md` in the current branch.

Acceptance Criteria Verification:

- Open the general off-chain service guide and verify it is separate from the
  preprod auditor flow runbook.
- Verify that the guide explains the minimal architecture:
  - chain source;
  - mempool source;
  - event handlers;
  - execution engine;
  - Cardano interpreter;
  - submission and health services.
- Verify that the guide explains crate roles for:
  - `bloom-offchain`;
  - `bloom-offchain-cardano`;
  - `spectrum-offchain-cardano`;
  - `cardano-chain-sync`;
  - `cardano-mempool-sync`;
  - `bloom-cardano-agent`.
- Verify that the guide explains initialization:
  - loading service config, deployment config, validation rules, and logging
    config;
  - validating configuration;
  - creating explorer access;
  - pulling deployed validators;
  - building auction order registries;
  - opening local state;
  - connecting node clients;
  - deriving operator credentials;
  - creating event channels and handlers;
  - creating execution contexts.
- Verify that the guide includes a configuration checklist with the required
  chain sync, node, network, operator, explorer, execution, partitioning, and
  health fields.
- Verify that auction order support configuration is shown with an
  `auctionOrders` JSON example.
- Verify that event and order handling are documented for pair updates,
  specialized order updates, funding events, order decoding, entity wiring,
  handler context, and execution instances.
- Verify that the small end-to-end example shows:
  - creating a preprod config;
  - providing deployment and validation files;
  - running `bloom-cardano-agent`;
  - funding operator addresses;
  - publishing supported order or pool UTxOs;
  - watching logs for batch formation, transaction creation, acceptance, and
    confirmation;
  - querying the health endpoint.
- Verify that the auditor flow README documents the one-command entry point,
  required inputs, expected prompts, expected success output, local verification
  commands, and the last verified preprod run.

## Milestone Output 6 — Example Real Off-Chain Bot Integration

Description:

The production example in this repository is `bloom-cardano-agent`, an
executable composition of the Bloom/Splash off-chain libraries. A smaller
service can reuse the same pieces and disable features it does not need.

Evidence:

- `bloom-cardano-agent` executable composition:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-cardano-agent/src/main.rs
- Agent configuration model:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-cardano-agent/src/config.rs
- Preprod agent config example:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-cardano-agent/resources/preprod.config.json
- Full preprod auditor flow that runs the agent and verifies execution:
  https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/amm-limit-auction-flow/run-amm-limit-auction-flow.sh

Acceptance Criteria Verification:

- Review `bloom-cardano-agent/src/main.rs` and verify it wires together chain
  sync, mempool sync, event handlers, execution streams, transaction
  interpretation/submission, and health reporting.
- Review `bloom-cardano-agent/src/config.rs` and verify the executable is
  configured through typed service configuration rather than hardcoded runtime
  values.
- Review `bloom-cardano-agent/resources/preprod.config.json` as a concrete
  preprod configuration example.
- Run the full preprod auditor flow and verify that the real bot observes the
  AMM pool and orders, forms a valid auction/counter limit-order batch, submits
  the execution transaction, and confirms it on preprod.

## Summary

This milestone delivery includes:

1. AMM pool support.
2. Limit order support.
3. Auction order implementation and wiring.
4. A reproducible preprod auditor flow.
5. Local verification commands.
6. Markdown documentation for building off-chain services with the library.
7. A real off-chain bot integration through `bloom-cardano-agent`.
8. Concrete preprod transaction evidence.

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
