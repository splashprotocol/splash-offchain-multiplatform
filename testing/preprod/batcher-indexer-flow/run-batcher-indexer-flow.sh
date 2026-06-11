#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
RUN_ID="${RUN_ID:-batcher-indexer-$(date -u +%Y%m%d-%H%M%S)}"
RUN_BASE="${SCRIPT_DIR}/.run"
RUN_ROOT="${RUN_BASE}/runs/${RUN_ID}"
STATE_DIR="${RUN_ROOT}/state"
LOG_DIR="${RUN_ROOT}/logs"
REPORT_DIR="${RUN_BASE}/reports"
REPORT_FILE="${REPORT_DIR}/${RUN_ID}.json"
INDEXER_PID=""

cleanup() {
  local status=$?
  if [[ -n "${INDEXER_PID:-}" ]] && ps -p "${INDEXER_PID}" >/dev/null 2>&1; then
    kill "${INDEXER_PID}" >/dev/null 2>&1 || true
    wait "${INDEXER_PID}" >/dev/null 2>&1 || true
  fi
  if [[ "$status" != "0" ]]; then
    echo "batcher indexer flow failed; run state is in ${RUN_ROOT}" >&2
    echo "logs are in ${LOG_DIR}" >&2
  fi
  exit "$status"
}
trap cleanup EXIT INT TERM

prompt() {
  local label="$1"
  local default_value="${2:-}"
  local answer
  if [[ ! -t 0 ]]; then
    printf '%s\n' "$default_value"
    return 0
  fi
  if [[ -n "$default_value" ]]; then
    read -r -p "${label} [${default_value}]: " answer
    printf '%s\n' "${answer:-$default_value}"
  else
    read -r -p "${label}: " answer
    printf '%s\n' "$answer"
  fi
}

prompt_secret() {
  local label="$1"
  local default_value="${2:-}"
  local answer
  if [[ ! -t 0 ]]; then
    printf '%s\n' "$default_value"
    return 0
  fi
  if [[ -n "$default_value" ]]; then
    read -r -s -p "${label} [provided by env, press Enter to use]: " answer
    printf '\n' >&2
    printf '%s\n' "${answer:-$default_value}"
  else
    read -r -s -p "${label}: " answer
    printf '\n' >&2
    printf '%s\n' "$answer"
  fi
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

find_free_port() {
  local port
  for port in "${BATCHER_INDEXER_HTTP_PORT:-9030}" 9031 9032 9033 9034 9035 9036 9037 9038 9039; do
    if ! lsof -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
      printf '%s\n' "$port"
      return 0
    fi
  done
  echo "No free HTTP port found in 9030..9039" >&2
  exit 1
}

start_datetime_default() {
  if command -v python3 >/dev/null 2>&1; then
    python3 -c 'from datetime import datetime, timedelta, timezone; print((datetime.now(timezone.utc)-timedelta(days=1)).replace(microsecond=0).isoformat().replace("+00:00","Z"))'
  else
    printf 'now\n'
  fi
}

clean_previous_runs() {
  mkdir -p "${RUN_BASE}/runs" "$REPORT_DIR"
  if [[ "${KEEP_BATCHER_INDEXER_RUNS:-0}" == "1" ]]; then
    return 0
  fi
  find "${RUN_BASE}/runs" -mindepth 1 -maxdepth 1 -type d -exec rm -rf {} +
}

main() {
  cd "$REPO_ROOT"
  require_cmd cargo
  require_cmd curl
  require_cmd deno
  require_cmd jq
  require_cmd lsof
  require_cmd python3

  clean_previous_runs
  mkdir -p "$STATE_DIR" "$LOG_DIR" "$REPORT_DIR"

  local socket_default="${CARDANO_NODE_SOCKET_PATH:-${NODE_SOCKET:-}}"
  local node_socket
  node_socket="$(prompt "Cardano node socket path" "$socket_default")"
  if [[ -z "$node_socket" ]]; then
    echo "Cardano node socket path is required." >&2
    exit 1
  fi
  if [[ ! -S "$node_socket" ]]; then
    echo "Node socket is not available at $node_socket" >&2
    exit 1
  fi

  local from_default="${BATCHER_INDEXER_FROM:-$(start_datetime_default)}"
  local from_datetime
  from_datetime="$(prompt "Start datetime (now or ISO-8601 UTC)" "$from_default")"
  if [[ -z "$from_datetime" ]]; then
    echo "Start datetime is required." >&2
    exit 1
  fi

  local blockfrost_project_id="${BLOCKFROST_PROJECT_ID:-}"
  if [[ "$from_datetime" != "now" && -z "$blockfrost_project_id" ]]; then
    blockfrost_project_id="$(prompt_secret "Preprod Blockfrost project id for timestamp-to-block lookup" "")"
  fi
  if [[ "$from_datetime" != "now" && -z "$blockfrost_project_id" ]]; then
    echo "A Blockfrost preprod project id is required to resolve historical datetime to block hash." >&2
    exit 1
  fi

  export CARDANO_NODE_SOCKET_PATH="$node_socket"
  export BATCHER_INDEXER_FROM="$from_datetime"
  export BLOCKFROST_PROJECT_ID="$blockfrost_project_id"

  local limit_order_hash
  limit_order_hash="${LIMIT_ORDER_SCRIPT_HASH:-$(jq -r '.limitOrder.hash' "${REPO_ROOT}/bloom-cardano-agent/resources/preprod.deployment.json")}"
  if [[ -z "$limit_order_hash" || "$limit_order_hash" == "null" ]]; then
    echo "Cannot resolve limit-order script hash. Set LIMIT_ORDER_SCRIPT_HASH." >&2
    exit 1
  fi
  local limit_order_address
  limit_order_address="$(deno run --no-lock --config "${REPO_ROOT}/testing/preprod/amm-limit-auction-flow/deno.json" \
    --allow-env --allow-read "${SCRIPT_DIR}/limit-order-address.ts" "$limit_order_hash")"
  export BATCHER_INDEXER_FAST_FORWARD_ADDRESS="$limit_order_address"

  if [[ "$from_datetime" == "now" ]]; then
    cardano-cli query tip --testnet-magic 1 > "${RUN_ROOT}/tip.json"
    export TIP_JSON="${RUN_ROOT}/tip.json"
  fi

  echo "Resolving start point for ${from_datetime}..."
  deno run --no-lock --allow-read --allow-env --allow-net \
    "${SCRIPT_DIR}/resolve-chain-point.ts" > "${RUN_ROOT}/chain-point.json"

  local http_port
  http_port="$(find_free_port)"
  deno run --no-lock --allow-read --allow-write --allow-env \
    "${SCRIPT_DIR}/generate-indexer-config.ts" \
    --chain-point "${RUN_ROOT}/chain-point.json" \
    --node-socket "$node_socket" \
    --state-dir "$STATE_DIR" \
    --http-port "$http_port" \
    --deployment "${REPO_ROOT}/bloom-cardano-agent/resources/preprod.deployment.json" \
    --out "${RUN_ROOT}/indexer.config.json"

  echo "Building bloom-execution-indexer..."
  cargo build -p bloom-execution-indexer > "${LOG_DIR}/cargo-build.log" 2>&1

  echo "Starting bloom-execution-indexer on http://127.0.0.1:${http_port}..."
  RUST_LOG="${BATCHER_INDEXER_RUST_LOG:-warn}" "${REPO_ROOT}/target/debug/bloom-execution-indexer" \
    --config-path "${RUN_ROOT}/indexer.config.json" \
    > "${LOG_DIR}/indexer.log" 2>&1 &
  INDEXER_PID="$!"
  echo "$INDEXER_PID" > "${RUN_ROOT}/indexer.pid"

  local from_ms
  from_ms="$(python3 - "${RUN_ROOT}/chain-point.json" <<'PY'
from datetime import datetime
import json
import os
import time
import sys

point = json.load(open(sys.argv[1]))
activity = point.get("fastForwardedToScriptActivity")
if activity and activity.get("direction") == "beforeRequestedPoint" and point.get("blockTime"):
    print(int(point["blockTime"]) * 1000)
else:
    value = os.environ["BATCHER_INDEXER_FROM"]
    print(int(time.time() * 1000) if value == "now" else int(datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp() * 1000))
PY
)"
  echo "Waiting for indexer to sync and expose batcher data..."
  local allow_partial_flag=()
  if [[ "${BATCHER_INDEXER_ALLOW_PARTIAL:-0}" == "1" ]]; then
    allow_partial_flag=(--allow-partial)
  fi
  deno run --no-lock --allow-read --allow-write --allow-net \
    "${SCRIPT_DIR}/wait-and-report.ts" \
    --base-url "http://127.0.0.1:${http_port}" \
    --from-ms "$from_ms" \
    --report "$REPORT_FILE" \
    --run-id "$RUN_ID" \
    --chain-point "${RUN_ROOT}/chain-point.json" \
    --log-dir "$LOG_DIR" \
    --timeout-secs "${BATCHER_INDEXER_WAIT_SECS:-900}" \
    ${allow_partial_flag:+"${allow_partial_flag[@]}"} \
    > "${LOG_DIR}/report.stdout.json" 2> "${LOG_DIR}/report.stderr.log"

  cat "${LOG_DIR}/report.stdout.json"
  echo "Batcher indexer flow completed. Report: ${REPORT_FILE}"
}

main "$@"
