CREATE TABLE IF NOT EXISTS executions
(
    ver           VARCHAR PRIMARY KEY,
    id            VARCHAR   NOT NULL,
    pair          VARCHAR   NOT NULL,
    price_num     BIGINT    NOT NULL,
    price_den     BIGINT    NOT NULL,
    removed_input BIGINT    NOT NULL,
    added_output  BIGINT    NOT NULL,
    side          VARCHAR   NOT NULL,
    meta          VARCHAR   NOT NULL,
    reporter      VARCHAR   NOT NULL,
    created_at    TIMESTAMP NOT NULL
);