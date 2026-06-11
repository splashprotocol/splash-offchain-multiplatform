# Preprod Batcher Indexer Auditor Flow

This flow demonstrates the Milestone 3 batcher execution indexer on preprod.

It starts `bloom-execution-indexer` from a human-readable datetime, waits until
the HTTP API is synced, verifies that batcher data is exposed, checks the
metrics invariant, and writes an auditor report.

Run from the repository root:

```bash
testing/preprod/batcher-indexer-flow/run-batcher-indexer-flow.sh
```

The script asks for:

- Cardano node socket path;
- start datetime, either `now` or ISO-8601 UTC like `2026-06-04T00:00:00Z`;
- Preprod Blockfrost project id when the start datetime is historical.

For non-interactive runs:

```bash
CARDANO_NODE_SOCKET_PATH=/path/to/node.socket \
BATCHER_INDEXER_FROM=2026-06-04T00:00:00Z \
BLOCKFROST_PROJECT_ID=<preprod-project-id> \
testing/preprod/batcher-indexer-flow/run-batcher-indexer-flow.sh
```

Every run creates a new state directory under `.run/runs/<run-id>`. Previous
generated run state is removed by default so restarts begin cleanly. Set
`KEEP_BATCHER_INDEXER_RUNS=1` to keep earlier generated state and logs for
debugging.

Reports are written to:

```text
testing/preprod/batcher-indexer-flow/.run/reports/
```

The report includes:

- resolved Cardano chain point;
- HTTP base URL;
- discovered batchers;
- per-batcher metrics queried through `/batchers/{pkh}/metrics`;
- log directory.

The flow succeeds only when at least one batcher has eligible orders and this
invariant holds:

```text
eligibleOrders = executedOrders + stillOpenEligibleOrders + missedEligibleOrders
```
