-- Engine Telemetry schema for persisting ExecutionReport data

-- Reports table: one row per received report
CREATE TABLE IF NOT EXISTS reports (
    id           BIGSERIAL PRIMARY KEY,
    received_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    reporter     TEXT        NOT NULL,        -- SocketAddr as string (ip:port)
    asset_a      TEXT        NOT NULL,        -- Display string of first AssetClass
    asset_b      TEXT        NOT NULL,        -- Display string of second AssetClass
    pair_json    JSONB       NOT NULL,        -- Serialized PairId (backward-compat)
    tx_hash      TEXT                        -- Hex-encoded transaction hash if present
);

CREATE INDEX IF NOT EXISTS reports_received_at_idx ON reports (received_at);
CREATE INDEX IF NOT EXISTS reports_tx_hash_idx ON reports (tx_hash);

-- Executions within a report
CREATE TABLE IF NOT EXISTS executions (
    id              BIGSERIAL PRIMARY KEY,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    report_id       BIGINT     NOT NULL REFERENCES reports(id) ON DELETE CASCADE,
    ord_index       INT        NOT NULL,      -- index in the executions vector to preserve order
    order_id        TEXT       NOT NULL,      -- Token as string (policyId.assetName)
    version         TEXT       NOT NULL,      -- OutputRef as string (txHash#index)
    avg_price       NUMERIC    NOT NULL,      -- AbsolutePrice as a single decimal number
    removed_input   TEXT       NOT NULL,      -- u64 as text
    added_output    TEXT       NOT NULL,      -- u64 as text
    fee             TEXT       NOT NULL,      -- u64 as text
    side            TEXT       NOT NULL       -- "Bid" | "Ask"
);

CREATE INDEX IF NOT EXISTS executions_report_id_idx ON executions (report_id);
CREATE INDEX IF NOT EXISTS executions_created_at_idx ON executions (created_at);

-- Engine internal events attached to a report
CREATE TABLE IF NOT EXISTS events (
    id          BIGSERIAL PRIMARY KEY,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    report_id   BIGINT  NOT NULL REFERENCES reports(id) ON DELETE CASCADE,
    ev_index    INT     NOT NULL,            -- index in the events vector to preserve order
    event_type  TEXT    NOT NULL,            -- Type tag of the event for querying
    event_json  JSONB   NOT NULL             -- Serialized ExecutionEvent
);

CREATE INDEX IF NOT EXISTS events_report_id_idx ON events (report_id);
CREATE INDEX IF NOT EXISTS events_created_at_idx ON events (created_at);
CREATE INDEX IF NOT EXISTS events_event_type_idx ON events (event_type);
