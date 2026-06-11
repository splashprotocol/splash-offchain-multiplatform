# Bloom Execution Indexer

Ledger-only service for batcher execution assessment. It is separate from
`bloom-cardano-agent` and reuses the existing Cardano chain-sync, transaction
view, and limit-order parsing code.

## Scope

The first version indexes confirmed ledger data only. It does not subscribe to
mempool events and does not expose pending-order metrics.

The service discovers batchers from chain data:

- public key hashes listed in limit-order `permitted_executors`;
- public key hashes that sign confirmed transactions spending those steered
  limit orders.

## Metrics Model

For a batcher `B`, `GET /batchers/{pkh}/metrics` builds the creation-time cohort
of observed limit orders where `B` is present in `permitted_executors`.

For each order in the cohort:

- `executedOrders`: the spend transaction has exactly one signer that matches
  the order's permitted executor set, and that signer is `B`;
- `stillOpenEligibleOrders`: the order output has not been observed as spent;
- `missedEligibleOrders`: the order was spent by another executor, spent with
  ambiguous executor attribution, spent with unknown attribution, or otherwise
  spent without a single matching executor signer.

The accounting invariant is:

```text
eligibleOrders = executedOrders + stillOpenEligibleOrders + missedEligibleOrders
```

Response-time metrics use confirmed ledger timestamps:

```text
responseMs = execution_time_ms - order_creation_time_ms
```

`medianResponseMs` and `p95ResponseMs` are computed over orders attributed to
the requested batcher.

## API

```text
GET /health
GET /batchers
GET /batchers/{pkh}
GET /batchers/{pkh}/metrics?fromMs=&toMs=&pair=
```

`fromMs` and `toMs` are Unix milliseconds and filter orders by creation time.
`pair` is the canonical pair string returned by indexed order records, for
example `Native/<policy.asset-name-hex>`.

## Configuration

Copy `resources/preprod.config.json.template` and replace:

- `chainSync.startingPoint`: the point to scan from;
- `chainSync.disableRollbacksUntil`: the rollback floor for the chosen point;
- `chainSync.dbPath`: chain-sync cache RocksDB path;
- `node.path`: local Cardano node socket path;
- `node.magic`: Cardano network magic;
- `networkId`: `0` for preprod, `1` for mainnet;
- `trackedLimitOrderScriptHashes`: deployed limit-order validator script hashes to index;
- `indexDbPath`: execution-index RocksDB path;
- `http.host` and `http.port`.

Example run:

```bash
cargo run -p bloom-execution-indexer -- --config-path bloom-execution-indexer/resources/preprod.config.json
```

Example metrics query:

```bash
curl "http://127.0.0.1:9030/batchers/<pkh>/metrics?fromMs=1760000000000&toMs=1760100000000"
```
