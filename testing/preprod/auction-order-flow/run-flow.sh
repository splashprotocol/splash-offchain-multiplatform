#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-observe}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"
FLOW_DIR="$ROOT/testing/preprod/auction-order-flow"
ENV_FILE="${FLOW_ENV_FILE:-$FLOW_DIR/.env}"
RUN_DIR="$FLOW_DIR/.run"

if [[ "$MODE" != "observe" && "$MODE" != "submit" ]]; then
  echo "Usage: $0 observe|submit" >&2
  exit 1
fi

if [[ ! -f "$ENV_FILE" ]]; then
  echo "Missing $ENV_FILE. Copy env.example first." >&2
  exit 1
fi

set -a
CLI_SUBMIT="${SUBMIT:-}"
source "$ENV_FILE"
set +a
if [[ -n "$CLI_SUBMIT" ]]; then
  SUBMIT="$CLI_SUBMIT"
fi
if [[ "${NETWORK:-preprod}" != "preprod" ]]; then
  echo "This harness is preprod-only; set NETWORK=preprod." >&2
  exit 1
fi
RUN_ID="${RUN_ID:-$(date +%Y%m%d-%H%M%S)}"
if [[ ! "$RUN_ID" =~ ^[A-Za-z0-9._-]+$ ]]; then
  echo "RUN_ID may contain only letters, numbers, dot, underscore, and dash." >&2
  exit 1
fi
export RUN_ID

mkdir -p "$RUN_DIR/logs"
export RUN_STATE_DIR="$RUN_DIR/state/$RUN_ID"
export AGENT_LOG_FILE="$RUN_DIR/logs/agent.$RUN_ID.log"

if [[ "${SUBMIT:-0}" != "1" ]]; then
  echo "Dry-run only: SUBMIT=1 is required before starting bloom-cardano-agent on ${NETWORK:-preprod}." >&2
  if [[ "$MODE" == "observe" ]]; then
    export AUCTION_TX_HASH="${EXISTING_AUCTION_TX_HASH:?EXISTING_AUCTION_TX_HASH is required}"
    export AUCTION_OUTPUT_INDEX="${EXISTING_AUCTION_OUTPUT_INDEX:?EXISTING_AUCTION_OUTPUT_INDEX is required}"
    deno run --allow-read --allow-env --allow-net "$FLOW_DIR/snapshot-auction-order.ts"
    deno run --allow-read --allow-env --allow-net "$FLOW_DIR/preflight-exact-liquidity.ts"
  elif [[ "${CREATE_COUNTER_ORDER:-1}" == "1" ]]; then
    deno run --allow-read --allow-env --allow-net "$FLOW_DIR/create-counter-limit-order.ts"
  fi
  exit 0
fi

if [[ "$MODE" == "observe" ]]; then
  export AUCTION_TX_HASH="${EXISTING_AUCTION_TX_HASH:?EXISTING_AUCTION_TX_HASH is required}"
  export AUCTION_OUTPUT_INDEX="${EXISTING_AUCTION_OUTPUT_INDEX:?EXISTING_AUCTION_OUTPUT_INDEX is required}"
  deno run --allow-read --allow-env --allow-net "$FLOW_DIR/snapshot-auction-order.ts" \
    | tee "$RUN_DIR/logs/snapshot-auction-order.json"
  export AUCTION_INITIAL_DATUM="$(jq -r '.datum' "$RUN_DIR/logs/snapshot-auction-order.json")"
  deno run --allow-read --allow-env --allow-net "$FLOW_DIR/preflight-exact-liquidity.ts"
fi

wait_for_agent_ready() {
  local deadline=$((SECONDS + ${AGENT_READY_TIMEOUT_SECS:-180}))
  local health_url="http://${AGENT_HEALTH_ADDR:-127.0.0.1:9024}/health"
  while (( SECONDS < deadline )); do
    if ! kill -0 "$AGENT_PID" 2>/dev/null; then
      echo "agent exited before readiness" >&2
      tail -200 "$AGENT_LOG_FILE" >&2 || true
      exit 1
    fi
    if curl -sf "$health_url" >/dev/null 2>&1 &&
       grep -q "Health API listening" "$AGENT_LOG_FILE" 2>/dev/null &&
       grep -q "Tip reached, waiting for new blocks" "$AGENT_LOG_FILE" 2>/dev/null; then
      return 0
    fi
    sleep 2
  done
  echo "agent did not become ready before submitting ${NETWORK:-preprod} orders" >&2
  tail -200 "$AGENT_LOG_FILE" >&2 || true
  exit 1
}

"$FLOW_DIR/run-agent.sh" &
AGENT_PID="$!"
trap 'kill "$AGENT_PID" 2>/dev/null || true' EXIT
wait_for_agent_ready

if [[ "$MODE" == "submit" ]]; then
  deno run --allow-read --allow-env --allow-net "$FLOW_DIR/create-counter-limit-order.ts" \
    | tee "$RUN_DIR/logs/create-counter-limit-order.json"
  export TARGET_COUNTER_ORDER_TX_HASH="$(jq -r 'select(.txHash) | .txHash' "$RUN_DIR/logs/create-counter-limit-order.json" | tail -1)"
  export TARGET_COUNTER_ORDER_OUTPUT_INDEX="$(jq -r 'select(.outputIndex != null) | .outputIndex' "$RUN_DIR/logs/create-counter-limit-order.json" | tail -1)"
  export TARGET_COUNTER_ORDER_PRICE_NUM="$(jq -r 'select(.counterPriceNum != null) | .counterPriceNum' "$RUN_DIR/logs/create-counter-limit-order.json" | tail -1)"
  export TARGET_COUNTER_ORDER_PRICE_DENOM="$(jq -r 'select(.counterPriceDenom != null) | .counterPriceDenom' "$RUN_DIR/logs/create-counter-limit-order.json" | tail -1)"
  export TARGET_COUNTER_ORDER_DATUM="$(jq -r 'select(.datum != null) | .datum' "$RUN_DIR/logs/create-counter-limit-order.json" | tail -1)"
  deno run --allow-read --allow-env --allow-net "$FLOW_DIR/preflight-exact-liquidity.ts"
  deno run --allow-read --allow-env --allow-net "$FLOW_DIR/create-auction-order.ts" \
    | tee "$RUN_DIR/logs/create-auction-order.json"
  export AUCTION_TX_HASH="$(jq -r 'select(.txHash) | .txHash' "$RUN_DIR/logs/create-auction-order.json" | tail -1)"
  export AUCTION_OUTPUT_INDEX="$(jq -r 'select(.outputIndex != null) | .outputIndex' "$RUN_DIR/logs/create-auction-order.json" | tail -1)"
  export AUCTION_INITIAL_DATUM="$(jq -r 'select(.datum != null) | .datum' "$RUN_DIR/logs/create-auction-order.json" | tail -1)"
else
  :
fi

deno run --allow-read --allow-env --allow-net "$FLOW_DIR/verify-auction-flow.ts"
