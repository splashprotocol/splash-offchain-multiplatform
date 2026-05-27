# Preprod Auction Order Auditor Flow

This harness verifies that `bloom-cardano-agent` can execute a Splash auction
order against a matching counter limit order on Cardano preprod.

## Auditor Command

Run from the repository root:

```bash
./testing/preprod/auction-order-flow/run-auction-flow.sh
```

The script is intentionally interactive. It prompts for missing local inputs,
prints a fresh wallet address, waits for funding, runs the agent, publishes the
orders, verifies execution, and writes a JSON report.

## What The Script Does

1. Creates an isolated run under `testing/preprod/auction-order-flow/.run`.
2. Generates a fresh wallet seed for the run.
3. Prompts for a preprod Blockfrost project id if `BLOCKFROST_PROJECT_ID` is not set.
4. Prompts for a Cardano node socket, showing an auto-detected path as the default when available.
5. Prints the wallet address and asks the auditor to send at least `500 tADA`.
6. Mints a unique token pair for the run.
7. Funds the agent funding addresses.
8. Starts `bloom-cardano-agent` from a recent preprod block.
9. Publishes a matching counter limit order and auction order.
10. Waits until the auction order is spent by the agent execution transaction.
11. Writes a report to `.run/reports/<run-id>.json`.

On failure, the wrapper kills the agent and removes the run RocksDB/state
directory so the next run starts cleanly. Wallet seeds are kept under
`.run/wallets` so leftover preprod funds can be recovered.

## Inputs

The default interactive flow needs only:

- a preprod Blockfrost project id,
- a reachable Cardano node socket,
- at least `500 tADA` sent to the generated wallet address.

Common socket paths are auto-detected:

- `/Users/aleksandr/node-external/node.socket`
- `$PWD/node.socket`
- `$HOME/node-external/node.socket`
- `/data/cardano-node/ipc/node.socket`

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

The agent log for the run should contain `Successfully formed a batch` and
`AuctionOrder::exec(`.

## Local Checks

Run these before handing the script to auditors:

```bash
bash -n testing/preprod/auction-order-flow/run-auction-flow.sh
deno check --config testing/preprod/auction-order-flow/deno.json \
  testing/preprod/auction-order-flow/create-order-pair.ts \
  testing/preprod/auction-order-flow/setup-preprod-flow.ts \
  testing/preprod/auction-order-flow/verify-auction-flow.ts \
  testing/preprod/auction-order-flow/wallet-info.ts
cargo check -p bloom-cardano-agent
```

## Last Verified Preprod Run

- Run ID: `auditor-20260526-234158`
- Funding tx: `4f86a41c1cdceadf94aa344dd0754e56e17f41782dec0fd2f2965d5f08fda96a`
- Auction tx: `949ac2a90cbc834afbdeb1a795a5340f5fb3ba171e21713866787b2c9df99465`
- Auction ref: `949ac2a90cbc834afbdeb1a795a5340f5fb3ba171e21713866787b2c9df99465#1`
- Execution tx: `f9bba6f9e7e135fae2883580c36feb627264dc6858b86e0e96bf5b86a7425e36`
