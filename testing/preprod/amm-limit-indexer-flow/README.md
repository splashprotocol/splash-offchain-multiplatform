# AMM Limit + Indexer Auditor Flow

This harness runs a fresh preprod proof for:

- fresh Bloom agent startup;
- fresh AMM pool deployment;
- 10 executable AMM limit orders in separate transactions;
- 1 non-executable AMM limit order in a separate transaction;
- batcher indexer verification against that fresh execution window.

Run:

```bash
bash testing/preprod/amm-limit-indexer-flow/run-amm-limit-indexer-flow.sh
```

The script prompts for:

- Cardano node socket path;
- Blockfrost preprod project id when needed.

It creates fresh generated state on every run and removes prior generated state by default.
