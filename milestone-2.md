# Proof of Achievement Report: Auction Order Support in Bloom/Splash Off-Chain Agent

Project Catalyst milestone reference: https://milestones.projectcatalyst.io/projects/1100283/milestones/2

This report describes the delivered auction order support for the Splash/Bloom Cardano execution engine and provides preprod transaction evidence showing that an auction order is created, matched, executed by `bloom-cardano-agent`, and verified on-chain.

No wallet seed phrases, signing keys, Blockfrost keys, node socket contents, or other secrets are included in this report.

## A. Output: Auction Order Support in the Off-Chain Execution Engine

Acceptance criteria: The Bloom/Splash execution engine can ingest auction order UTxOs, model them as timed taker orders, advance auction clocks from ledger time, match auction orders against compatible liquidity, and build valid execution transactions.

Evidence:

- Auction order domain implementation: https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/bloom-offchain-cardano/src/orders/auction.rs
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

Auditor reference branch:

- https://github.com/splashprotocol/splash-offchain-multiplatform/tree/bromel777/auction-orders-support

Auditor reference pull request:

- https://github.com/splashprotocol/splash-offchain-multiplatform/pull/256

## B. Output: Preprod Auction Order Auditor Flow

Acceptance criteria: Auditors can run one script from a fresh checkout, provide a Blockfrost preprod key and Cardano node socket path, fund a fresh generated wallet with tADA, and observe a full auction order flow on preprod.

Evidence:

- Auditor script: https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/auction-order-flow/run-auction-flow.sh
- Auditor README: https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/auction-order-flow/README.md
- Fresh preprod setup script: https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/auction-order-flow/setup-preprod-flow.ts
- Paired limit/auction order creation script: https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/auction-order-flow/create-order-pair.ts
- Execution verifier: https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/auction-order-flow/verify-auction-flow.ts
- Local wallet and agent config helpers:
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/auction-order-flow/wallet-info.ts
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/auction-order-flow/agent-info.ts
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/auction-order-flow/generate-agent-config.sh
  - https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/auction-order-flow/run-agent.sh

Auditor command:

```bash
./testing/preprod/auction-order-flow/run-auction-flow.sh
```

The script asks the auditor for:

- Preprod Blockfrost project id.
- Cardano node socket path. If a common socket path is detected, it is shown as the default and can be accepted with Enter.
- Funding confirmation after at least `500 tADA` is sent to the generated preprod wallet address.

On failure, the script kills the agent and removes the run-local RocksDB/state directory so a later run starts cleanly. Wallet seed files remain local under `.run/wallets` so leftover preprod funds can be recovered by the operator.

## C. Output: Tests and Local Verification

Acceptance criteria: The code and scripts are covered by local verification commands that auditors and reviewers can run before executing a live preprod flow.

Evidence:

Commands run successfully:

```bash
bash -n testing/preprod/auction-order-flow/run-auction-flow.sh
# exit code: 0, no output

deno check --config testing/preprod/auction-order-flow/deno.json \
  testing/preprod/auction-order-flow/create-order-pair.ts \
  testing/preprod/auction-order-flow/setup-preprod-flow.ts \
  testing/preprod/auction-order-flow/verify-auction-flow.ts \
  testing/preprod/auction-order-flow/wallet-info.ts
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

`cargo check -p bloom-cardano-agent` completed successfully. Existing repository warnings were emitted, but no build errors were reported.

## D. Output: Reproducible Preprod Execution Evidence

Acceptance criteria: The delivered auditor flow demonstrates a real preprod execution transaction that spends an auction order UTxO and is produced by `bloom-cardano-agent`.

Latest successful preprod auditor run:

- Run ID: `auditor-20260527-001856`
- Network: Cardano preprod
- Report file generated locally: `testing/preprod/auction-order-flow/.run/reports/auditor-20260527-001856.json`
- Final report status: `ok`
- Generated wallet address: `addr_test1qpzn3896cvp8nklf0xqd2v53az7zsmqe3k5fjw472zqwmglgcx60fyhevv9fyta4umtdjsqmxwtm0wm67hzhc8d4prmstkucy9`
- Agent funding address used by the report: `addr_test1qqz8nghr3dy00gh645h2t2a3kpsmtt5j7jgqfvsmgjsgvmh43fqc6fwscrcvv3qqx3kh3s7u8ggsfgzdjvh5rjp342ast3aymt`
- Agent funding amount: `50,000,000` lovelace
- Funding / mint transaction: `7c38987158e3667ff1bd35724aa162aa46f35f5b7011b238e82f074e13444984`
- Paired order transaction: `e0e71a2aa1a30cae126cdb7bfb3cfa1c6b458ed23c2d59133907fe00fbdb3407`
- Counter limit order ref: `e0e71a2aa1a30cae126cdb7bfb3cfa1c6b458ed23c2d59133907fe00fbdb3407#0`
- Auction order ref: `e0e71a2aa1a30cae126cdb7bfb3cfa1c6b458ed23c2d59133907fe00fbdb3407#1`
- Agent execution transaction: `fdae75c77dd175e384ed6a83de5311173d6f6cbfd643dbd1caf6f069a6d00ec4`

Public explorer links:

- Funding / mint transaction: [7c38987158e3667ff1bd35724aa162aa46f35f5b7011b238e82f074e13444984](https://preprod.cexplorer.io/tx/7c38987158e3667ff1bd35724aa162aa46f35f5b7011b238e82f074e13444984). This confirms creation of the generated run assets and funding outputs used by the agent.
- Paired counter limit order and auction order transaction: [e0e71a2aa1a30cae126cdb7bfb3cfa1c6b458ed23c2d59133907fe00fbdb3407](https://preprod.cexplorer.io/tx/e0e71a2aa1a30cae126cdb7bfb3cfa1c6b458ed23c2d59133907fe00fbdb3407). This contains the counter limit order at output `#0` and auction order at output `#1`.
- Agent execution transaction: [fdae75c77dd175e384ed6a83de5311173d6f6cbfd643dbd1caf6f069a6d00ec4](https://preprod.cexplorer.io/tx/fdae75c77dd175e384ed6a83de5311173d6f6cbfd643dbd1caf6f069a6d00ec4). This is the transaction submitted by the agent flow that spends auction order ref `e0e71a2aa1a30cae126cdb7bfb3cfa1c6b458ed23c2d59133907fe00fbdb3407#1`.

Validator references used by the run:

- Auction validator hash: `fbc59e20bd55b1b521db154643fb359020bdfc8da40120946c89c08a`
- Auction validator reference: `7eccade32a21891469792da9b9ecbd9f0fed9835a591a109e63f6889d2118cf0#0`
- Limit order validator hash: `464eeee89f05aff787d40045af2a40a83fd96c513197d32fbc54ff02`
- Limit order validator reference: `77fc5b1688a79dd5b9bdba5981b7403b519690dda4f1471fffa78151bbefe0f5#0`
- Limit order witness validator hash: `96f5c1bee23481335ff4aece32fe1dfa1aa40a944a66d2d6edc9a9a5`
- Limit order witness validator reference: `77fc5b1688a79dd5b9bdba5981b7403b519690dda4f1471fffa78151bbefe0f5#1`

Generated test assets:

- Base unit: `72689a5a6a6507e64431146ea7899314a0293a782a23631d95088b4b61756374696f6e426173652d36303532372d303031383536`
- Quote unit: `72689a5a6a6507e64431146ea7899314a0293a782a23631d95088b4b61756374696f6e51756f74652d36303532372d303031383536`
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
  "txHash": "e0e71a2aa1a30cae126cdb7bfb3cfa1c6b458ed23c2d59133907fe00fbdb3407",
  "outputIndex": "1",
  "spendingTx": "fdae75c77dd175e384ed6a83de5311173d6f6cbfd643dbd1caf6f069a6d00ec4",
  "agentLog": "testing/preprod/auction-order-flow/.run/logs/auditor-20260527-001856/agent.log"
}
```

Agent log checkpoints from the run:

- The liquidity book formed a batch containing the auction ask and counter limit bid.
- `AuctionOrder::exec(removed_input=1000, added_output=2000, consumed_budget=0, consumed_fee=0)` was executed.
- The execution transaction `fdae75c77dd175e384ed6a83de5311173d6f6cbfd643dbd1caf6f069a6d00ec4` was accepted.
- The execution transaction was later confirmed and removed from pending transaction tracking.

## E. Output: Documentation for Building and Operating Off-Chain Services

Acceptance criteria: The repository includes Markdown documentation that explains how to build an off-chain service with the Bloom/Splash libraries, including minimal architecture, initialization, event/order handling, configuration, and a small end-to-end example. It also includes auditor/operator documentation for running the auction order test flow and interpreting success.

Evidence:

- General off-chain service guide: https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/docs/offchain-service-guide.md
- Auditor flow README: https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/testing/preprod/auction-order-flow/README.md
- Resolution plan and implementation notes: https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/auction-orders-support/docs/plans/2026-05-26-auction-order-preprod-resolution.md
- This Proof of Achievement report: `milestone-2.md` in the current branch.

### Building an Off-Chain Service with the Bloom/Splash Libraries

This guide explains how to assemble a Cardano off-chain service using the Bloom/Splash off-chain libraries in this repository. It is intentionally separate from the auction-order preprod runbook: the runbook proves one flow, while this section explains the reusable service architecture and integration points.

The production example in this repository is `bloom-cardano-agent`. A smaller service can reuse the same pieces and disable features it does not need.

#### Minimal Architecture

A Bloom/Splash off-chain service has six moving parts:

1. **Chain source**: reads ledger transactions from a Cardano node and turns them into typed transaction views.
2. **Mempool source**: optionally reads unconfirmed transactions so the service can react before ledger confirmation.
3. **Event handlers**: classify transaction outputs and inputs into domain events such as pool updates, order creation, order elimination, and funding box changes.
4. **Execution engine**: keeps per-pair liquidity books, matches compatible orders/pools, and produces execution recipes.
5. **Cardano interpreter**: converts recipes into Cardano transactions using deployed validator references, collateral, funding boxes, and operator credentials.
6. **Submission and health services**: submit transactions, track confirmation, report execution status, and expose a health endpoint.

In `bloom-cardano-agent`, these pieces are wired in `bloom-cardano-agent/src/main.rs`.

#### Crate Roles

- `bloom-offchain`: generic execution engine, temporal liquidity book, state index, backlog, matching, and execution stream orchestration.
- `bloom-offchain-cardano`: Cardano-specific order/pool decoding, event handlers, recipe interpretation, validation rules, and transaction building.
- `spectrum-offchain-cardano`: Cardano data types, validator deployment models, credentials, transaction submission, and node helpers.
- `cardano-chain-sync` and `cardano-mempool-sync`: node-to-client ledger and mempool streams.
- `bloom-cardano-agent`: executable composition of the above libraries.

#### Initialization Flow

A service should initialize components in this order:

1. **Load configuration**:
   - service config, for example `bloom-cardano-agent/resources/preprod.config.json`;
   - validator deployment config, for example `bloom-cardano-agent/resources/preprod.deployment.json`;
   - validation rules, for example `bloom-cardano-agent/resources/validation-rules.json.template`;
   - logging config, for example `bloom-cardano-agent/resources/log4rs.yaml`.
2. **Validate configuration** with the local integrity checks. The agent uses `CheckIntegrity` to reject malformed partitioning, fee, and graduation settings before connecting to the node.
3. **Create explorer access** through `cardano_explorer::AnyExplorer`. The explorer is used to pull validator reference UTxOs and collateral/funding state.
4. **Pull deployed validators** with `ProtocolDeployment::unsafe_pull`.
5. **Build optional order registries**, such as `AuctionOrderRegistry`, from configured validator references and cost limits.
6. **Open local state**:
   - chain-sync RocksDB cache;
   - optional graduation RocksDB state;
   - in-memory entity/order/funding indexes.
7. **Connect node clients**:
   - `ChainSyncClient` for ledger events;
   - `LocalTxMonitorClient` for mempool events;
   - local transaction submission client.
8. **Derive operator credentials**, collateral address, and funding addresses from the operator signing key.
9. **Create event channels and handlers** for pair updates, specialized order updates, and funding events.
10. **Create execution contexts** and start one or more execution streams.

The agent uses four execution streams in production. A minimal service can use one partition and one execution stream.

#### Configuration Checklist

The minimal config fields are:

- `chainSync.startingPoint`: ledger point where the service starts scanning.
- `chainSync.replayFromPoint`: optional earlier point for replaying indexed events.
- `chainSync.disableRollbacksUntil`: rollback guard point.
- `chainSync.dbPath`: local RocksDB path for the ledger cache.
- `node.path`: Cardano node socket path.
- `node.magic`: network magic, for example `1` for preprod.
- `networkId`: Cardano network id, for example `0` for testnet.
- `operatorKey`: operator signing key material or configured key source.
- `explorer`: Blockfrost or Maestro configuration, depending on the build and deployment environment.
- `execution.executionCap`: soft and hard execution-unit caps for batches.
- `execution.o2oAllowed`: whether order-to-order matching is enabled.
- `partitioning`: total partitions and assigned partitions for this service instance.
- `daoConfig`, `royaltyWithdraw`, and other protocol-specific contexts required by enabled validators.
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

Use the deployed values for the target network. Do not hardcode private keys, API keys, or local node socket paths into committed config.

#### Event and Order Handling

The service receives `TxViewMut` values from ledger and mempool streams. Event handlers inspect each transaction and produce typed domain events:

- **Pair updates** for evolving entities such as pools.
- **Specialized order updates** for atomic order entities such as limit, instant, grid, and auction orders.
- **Funding events** for operator funding boxes used by transaction building.

The main handlers are:

- `PairUpdateHandler`: classifies pool and evolving entity transitions.
- `SpecializedHandler`: classifies order creation and elimination.
- `FundingEventHandler`: tracks funding boxes belonging to operator-derived funding addresses.

Order support is usually added in four places:

1. **Domain decoder**: parse the on-chain datum/redeemer into a Rust order type. Auction orders are implemented in `bloom-offchain-cardano/src/orders/auction.rs`.
2. **Entity wiring**: include the order in the agent entity enum so ledger and mempool handlers can see it. See `bloom-cardano-agent/src/entity.rs`.
3. **Event handling context**: provide validator hashes, deployment references, and validation rules through `HandlerContext`.
4. **Execution instances**: implement how the order executes and how the Cardano interpreter spends it. See `bloom-offchain-cardano/src/execution_engine/instances.rs`.

Auction orders are timed taker orders. The liquidity book advances clocks from ledger time, activates the order only in its valid time span, computes the current auction price, and matches it against compatible counter liquidity.

#### Execution Engine Flow

The execution stream consumes pair/order/funding events and performs this loop:

1. Resolve the latest known state of every affected entity through the state index.
2. Update the per-pair `TLB` liquidity book.
3. Select candidate takers and makers for the pair.
4. Form a `MatchmakingRecipe` when prices, time bounds, and execution caps allow a valid batch.
5. Convert the recipe into a Cardano transaction blueprint.
6. Pull collateral and funding boxes.
7. Balance fees and execution budgets.
8. Submit the transaction through the local submission agent.
9. Track mempool acceptance and ledger confirmation.
10. Commit or roll back predicted state depending on confirmation.

The generic stream entry point is `execution_part_stream` in `bloom-offchain/src/execution_engine/mod.rs`.

#### Minimal End-to-End Example

This example describes the smallest practical service that follows the same architecture as `bloom-cardano-agent`.

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

The preprod auction-order auditor flow in `testing/preprod/auction-order-flow/run-auction-flow.sh` is a concrete example of steps 1-6 for one order family. It generates a wallet, asks for external inputs, funds the agent, publishes a matching auction/limit order pair, waits for execution, and writes a JSON report.

#### Operational Notes

- Start chain sync far enough before the order UTxOs so the service sees validator references, funding boxes, and orders.
- Keep RocksDB paths unique per run or clean them before replaying the same scenario.
- Keep operator keys, API keys, and wallet seed files outside committed docs.
- Prefer a health endpoint in auditor and production deployments.
- Use mempool support for faster reaction; disable it only when a deployment intentionally wants ledger-confirmed events only.
- When adding a new order family, add a small unit test for order math and a live-network runbook or integration flow showing that the order is observed, matched, submitted, and confirmed.

The auction flow README documents:

- The one-command auditor entry point.
- Required inputs: Blockfrost preprod key, Cardano node socket, and tADA funding.
- What the script does at each stage.
- Expected success output.
- Local verification commands.
- Last verified preprod run and transaction hashes.

## Summary

The milestone delivery includes the auction order implementation and wiring referenced above, plus a demonstrated preprod execution path, a reproducible auditor flow, local verification commands, Markdown documentation, and concrete preprod transaction evidence.

The latest completed preprod run demonstrates that:

1. A fresh test token pair was minted on preprod.
2. A matching counter limit order and auction order were published in one transaction.
3. `bloom-cardano-agent` observed both orders.
4. The liquidity book formed a valid batch.
5. The auction order execution transition ran.
6. The agent submitted execution transaction `fdae75c77dd175e384ed6a83de5311173d6f6cbfd643dbd1caf6f069a6d00ec4`.
7. The verifier confirmed that the auction order UTxO was spent by the agent flow.

Sensitive-information confirmation: this Proof of Achievement contains public repository paths, public preprod transaction hashes, public preprod addresses, and public asset ids. It does not include wallet seeds, private keys, API keys, local socket paths, or local state paths.
