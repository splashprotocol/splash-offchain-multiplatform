# Preprod Auction Order Flow

This harness tests Splash auction orders through `bloom-cardano-agent`.

Default behavior is dry-run. No preprod transaction is submitted unless
`SUBMIT=1`.

## Modes

- `observe`: do not create an auction; run the agent against
  `EXISTING_AUCTION_TX_HASH#EXISTING_AUCTION_OUTPUT_INDEX`.
- `submit`: create a paired counter limit order, create a token-token auction
  UTxO, run the agent, and verify the auction is spent.

## Commands

```bash
cp testing/preprod/auction-order-flow/env.example testing/preprod/auction-order-flow/.env
$EDITOR testing/preprod/auction-order-flow/.env

testing/preprod/auction-order-flow/run-flow.sh observe
SUBMIT=1 testing/preprod/auction-order-flow/run-flow.sh submit
```

## Auditor Flow

For a self-contained preprod run, run the wrapper from the repository root:

```bash
testing/preprod/auction-order-flow/run-auditor-flow.sh
```

The script defaults to Blockfrost, fresh setup with the already deployed
validator references, paired order publication in one transaction, and starting
`bloom-cardano-agent` before publishing orders. It
creates an isolated run env/state/log directory under `.run`, generates a fresh
funding wallet seed for that run, prints the wallet address and the 500 tADA
default minimum funding requirement, pauses for funding, mints a unique token
pair, publishes the paired limit order and auction order, verifies execution,
and writes a JSON report. On failure it kills the agent and removes the run
RocksDB/state directory so the next run starts cleanly. Generated wallet seeds
are kept under `.run/wallets` so any leftover preprod funds can still be
recovered.

The wrapper auto-detects common local paths:

- node socket: `/Users/aleksandr/node-external/node.socket`,
  `$PWD/node.socket`, `$HOME/node-external/node.socket`,
  `/data/cardano-node/ipc/node.socket`
If the node socket is not found, the script prompts for it. It also prompts for
the preprod Blockfrost key if `BLOCKFROST_PROJECT_ID` is not already set.

Optional developer override:

```bash
BLOCKFROST_PROJECT_ID=preprod... \
testing/preprod/auction-order-flow/run-auditor-flow.sh \
  --node-socket /path/to/node.socket
```

## Safety

Use a dedicated wallet and tiny token-token amounts. ADA-side auctions are
rejected because the current auction integration intentionally does not support
ADA-side auction accounting.

## Local Verification

Run these before any preprod submission:

```bash
bash -n testing/preprod/auction-order-flow/*.sh
cd testing/preprod/auction-order-flow && deno task check
cargo check --package bloom-cardano-agent
cargo test --package bloom-offchain-cardano orders::auction::tests
git diff --check
```

For live preprod:

```bash
SUBMIT=1 testing/preprod/auction-order-flow/run-flow.sh submit
```

Success criteria:

- creator prints submitted auction tx hash
- agent starts from isolated `.run/state`
- verifier reports `status: "spent_by_agent_flow"`
- agent log contains an auction order execution transition
