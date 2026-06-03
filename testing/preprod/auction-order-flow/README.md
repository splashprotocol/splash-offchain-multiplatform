# Preprod Auction Order Auditor Flow

This harness verifies that `bloom-cardano-agent` can execute the main Splash
preprod surfaces required by the milestone: AMM pool liquidity, limit orders,
and auction orders.

This README is the flow-specific auditor runbook. General documentation on
building and operating an off-chain service with the Bloom/Splash libraries is
in `docs/offchain-service-guide.md` and is also included in `milestone-2.md`,
section E.

## Auditor Command

Run from the repository root:

```bash
./testing/preprod/auction-order-flow/run-auction-flow.sh
```

The script is intentionally interactive. It prompts for missing local inputs,
prints a fresh wallet address, waits for funding, creates a fresh batcher,
starts the agent, deploys an AMM pool, publishes and confirms a limit order
using that pool's asset pair, publishes the auction order flow, verifies auction
execution, and writes a JSON report.

For this auditor flow the generated agent config sets `disableMempool=true`. The
proof is ledger-driven: all setup/order transactions are first confirmed on
preprod, then the agent observes them through chain sync and submits the
execution transaction. This avoids non-deterministic local mempool rollback
events in repeatable audit runs. Set `AGENT_DISABLE_MEMPOOL=0` only when
debugging mempool behavior itself.

## AMM Pool And Limit Order Demo Scripts

The supervised auditor command above is the primary end-to-end execution proof.
The lower-level AMM and limit-order scripts remain available for focused
debugging:

```bash
./testing/preprod/auction-order-flow/run-amm-limit-demo.sh
```

The lower-level wrapper does not start or fund `bloom-cardano-agent`; it is only
for script-level transaction checks. Use `run-auction-flow.sh` for auditor
acceptance evidence.

The lower-level scripts can also be run independently:

```bash
SUBMIT=1 deno run --allow-env --allow-read --allow-net \
  --config testing/preprod/auction-order-flow/deno.json \
  testing/preprod/auction-order-flow/deploy-amm-pool.ts

SUBMIT=1 deno run --allow-env --allow-read --allow-net \
  --config testing/preprod/auction-order-flow/deno.json \
  testing/preprod/auction-order-flow/create-limit-order.ts
```

Required shared inputs:

- `WALLET_SEED_FILE`
- `PROVIDER=blockfrost` with `BLOCKFROST_PROJECT_ID`, or `PROVIDER=maestro` with
  `MAESTRO_API_KEY`, or `PROVIDER=koios`
- `DEPLOYMENT_CONFIG`, defaulting to
  `bloom-cardano-agent/resources/preprod.deployment.json`

AMM pool defaults:

- If `POOL_X_*` and `POOL_Y_*` are absent, the script mints fresh demo pool
  assets under the wallet native policy.
- If `POOL_X_POLICY`/`POOL_X_NAME_HEX` and `POOL_Y_POLICY`/`POOL_Y_NAME_HEX` are
  set, those assets are used instead.

Limit order defaults:

- `LIMIT_INPUT_*` defaults to `AUCTION_QUOTE_*` when present.
- `LIMIT_OUTPUT_*` defaults to `AUCTION_BASE_*` when present.
- `LIMIT_TRADABLE_INPUT`, `LIMIT_BASE_PRICE_*`, `LIMIT_FEE`, and
  `LIMIT_LOVELACE_BUDGET` can be overridden through environment variables.
- The supervised AMM-limit phase creates a bid against the freshly deployed pool
  by default: `AMM_LIMIT_TRADABLE_INPUT=2000`,
  `AMM_LIMIT_MIN_MARGINAL_OUTPUT=900`, `AMM_LIMIT_BASE_PRICE_NUM=1`,
  `AMM_LIMIT_BASE_PRICE_DENOM=3`, and `AMM_LIMIT_LOVELACE_BUDGET=2100000`.

## What The Script Does

1. Creates an isolated run under `testing/preprod/auction-order-flow/.run`.
2. Generates a fresh wallet seed for the run.
3. Prompts for a preprod Blockfrost project id if `BLOCKFROST_PROJECT_ID` is not
   set.
4. Prompts for a Cardano node socket, showing an auto-detected path as the
   default when available.
5. Prints the wallet address and asks the auditor to send at least `500 tADA`.
6. Mints a unique token pair for the run.
7. Funds the agent funding addresses.
8. Starts `bloom-cardano-agent` from a recent preprod block and waits until it
   reaches tip. The generated config disables mempool processing by default.
9. Deploys a classic AMM pool with fresh demo assets and waits for it to be
   indexed.
10. Publishes a limit order using that AMM pool's asset pair and waits for it to
    be indexed.
11. Publishes a matching counter limit order and auction order.
12. Waits until the auction order is spent by the agent execution transaction.
13. Writes a report to `.run/reports/<run-id>.json`.

On failure, the wrapper kills the agent and removes the run RocksDB/state
directory so the next run starts cleanly. Fresh setup creates a new funding
wallet seed on each start; set `REUSE_RUN_WALLET=1` only for local debugging
when intentionally reusing a funded test wallet.

## Inputs

The default interactive flow needs only:

- a preprod Blockfrost project id,
- a reachable Cardano node socket,
- at least `500 tADA` sent to the generated wallet address.

Common socket paths are auto-detected:

- `$PWD/node.socket`
- `$CARDANO_NODE_SOCKET_PATH`
- `$CARDANO_NODE_SOCKET`
- `$HOME/.cardano-node/node.socket`
- `/data/cardano-node/ipc/node.socket`
- `/var/lib/cardano-node/node.socket`

The script always asks for the socket path in interactive mode. Press Enter to
accept the detected default, or type another path.

Optional environment overrides:

```bash
BLOCKFROST_PROJECT_ID=preprod... \
NODE_SOCKET=/path/to/node.socket \
./testing/preprod/auction-order-flow/run-auction-flow.sh
```

The wrapper uses the already deployed preprod auction validator reference from
`env.example` by default. It does not require an Aiken blueprint.

## Expected Success

The final output should contain:

```json
{
  "status": "ok",
  "executionTx": "<preprod transaction hash>"
}
```

The verifier also prints:

```json
{
  "status": "spent_by_agent_flow",
  "spendingTx": "<preprod transaction hash>"
}
```

The agent log for the run should contain `Successfully formed a batch`,
`LimitOrder::exec(`, and `AuctionOrder::exec(` for the auction execution path.

## Local Checks

Run these before handing the script to auditors:

```bash
bash -n testing/preprod/auction-order-flow/run-auction-flow.sh
bash -n testing/preprod/auction-order-flow/run-amm-limit-demo.sh
deno check --config testing/preprod/auction-order-flow/deno.json \
  testing/preprod/auction-order-flow/deploy-amm-pool.ts \
  testing/preprod/auction-order-flow/create-limit-order.ts \
  testing/preprod/auction-order-flow/create-order-pair.ts \
  testing/preprod/auction-order-flow/setup-preprod-flow.ts \
  testing/preprod/auction-order-flow/verify-limit-order-flow.ts \
  testing/preprod/auction-order-flow/verify-auction-flow.ts \
  testing/preprod/auction-order-flow/wallet-info.ts
cargo check -p bloom-cardano-agent
```

## Last Verified Preprod Run

- Run ID: `auditor-20260603-172625`
- Funding / mint tx:
  `eecdb897a2cb1816a5ec51934c1aba27fb2db0139c2902b34d4eb25250cf00c3`
- AMM pool tx:
  `02b55c2ee92589f4510284cab33dbf96cfee107ec6193fe8f7e4dc97781d4703`
- AMM-pair limit order tx:
  `55f5bf36091fccc7acf5584e27a58d5f5db793032337519849c05cb1b8e2cef4`
- Auction + counter limit order tx:
  `8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440`
- Counter limit order ref:
  `8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440#0`
- Auction ref:
  `8e300ad3a876277be43a120d505aa960b5d9df349da24a147bb9135893192440#1`
- Execution tx:
  `f1fcc5d146dbe6c9296c80818b156006261b8970e7df385e37000ca18b0d0a08`
