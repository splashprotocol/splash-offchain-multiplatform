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

The general off-chain service guide documents:

- The minimal service architecture: chain source, mempool source, event handlers, execution engine, Cardano interpreter, submission, and health services.
- Initialization order for config, deployed validators, explorer access, local state, node clients, event channels, contexts, and execution streams.
- Configuration fields required to run a service and optional auction-order registry configuration.
- Event and order handling responsibilities, including where to add a new order family.
- Execution-engine data flow from ledger/mempool events to matched recipes, transaction building, submission, and confirmation.
- A small end-to-end preprod service example with config shape, command line, funding requirements, expected logs, and health endpoint.

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
