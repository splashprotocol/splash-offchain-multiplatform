# Building an Off-Chain Service with the Bloom/Splash Libraries

This guide explains how to assemble a Cardano off-chain service using the
Bloom/Splash off-chain libraries in this repository. It is intentionally
separate from the auction-order preprod runbook: the runbook proves one flow,
while this document explains the reusable service architecture and integration
points.

The production example in this repository is `bloom-cardano-agent`. A smaller
service can reuse the same pieces and disable features it does not need.

## Minimal Architecture

A Bloom/Splash off-chain service has six moving parts:

1. **Chain source**: reads ledger transactions from a Cardano node and turns
   them into typed transaction views.
2. **Mempool source**: optionally reads unconfirmed transactions so the service
   can react before ledger confirmation.
3. **Event handlers**: classify transaction outputs and inputs into domain
   events such as pool updates, order creation, order elimination, and funding
   box changes.
4. **Execution engine**: keeps per-pair liquidity books, matches compatible
   orders/pools, and produces execution recipes.
5. **Cardano interpreter**: converts recipes into Cardano transactions using
   deployed validator references, collateral, funding boxes, and operator
   credentials.
6. **Submission and health services**: submit transactions, track confirmation,
   report execution status, and expose a health endpoint.

In `bloom-cardano-agent`, these pieces are wired in
`bloom-cardano-agent/src/main.rs`.

## Crate Roles

- `bloom-offchain`: generic execution engine, temporal liquidity book, state
  index, backlog, matching, and execution stream orchestration.
- `bloom-offchain-cardano`: Cardano-specific order/pool decoding, event
  handlers, recipe interpretation, validation rules, and transaction building.
- `spectrum-offchain-cardano`: Cardano data types, validator deployment models,
  credentials, transaction submission, and node helpers.
- `cardano-chain-sync` and `cardano-mempool-sync`: node-to-client ledger and
  mempool streams.
- `bloom-cardano-agent`: executable composition of the above libraries.

## Initialization Flow

A service should initialize components in this order:

1. **Load configuration**:
   - service config, for example `bloom-cardano-agent/resources/preprod.config.json`;
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

## Configuration Checklist

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

## Event and Order Handling

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
4. **Execution instances**: implement how the order executes and how the
   Cardano interpreter spends it. See
   `bloom-offchain-cardano/src/execution_engine/instances.rs`.

Auction orders are timed taker orders. The liquidity book advances clocks from
ledger time, activates the order only in its valid time span, computes the
current auction price, and matches it against compatible counter liquidity.

## Execution Engine Flow

The execution stream consumes pair/order/funding events and performs this loop:

1. Resolve the latest known state of every affected entity through the state
   index.
2. Update the per-pair `TLB` liquidity book.
3. Select candidate takers and makers for the pair.
4. Form a `MatchmakingRecipe` when prices, time bounds, and execution caps
   allow a valid batch.
5. Convert the recipe into a Cardano transaction blueprint.
6. Pull collateral and funding boxes.
7. Balance fees and execution budgets.
8. Submit the transaction through the local submission agent.
9. Track mempool acceptance and ledger confirmation.
10. Commit or roll back predicted state depending on confirmation.

The generic stream entry point is `execution_part_stream` in
`bloom-offchain/src/execution_engine/mod.rs`.

## Minimal End-to-End Example

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

The preprod auction-order auditor flow in
`testing/preprod/auction-order-flow/run-auction-flow.sh` is a concrete example
of steps 1-6 for one order family. It generates a wallet, asks for external
inputs, funds the agent, publishes a matching auction/limit order pair, waits
for execution, and writes a JSON report.

## Operational Notes

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
