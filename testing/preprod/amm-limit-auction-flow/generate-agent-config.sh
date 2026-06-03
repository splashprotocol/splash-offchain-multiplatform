#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"
FLOW_DIR="$ROOT/testing/preprod/amm-limit-auction-flow"
ENV_FILE="${FLOW_ENV_FILE:-$FLOW_DIR/.env}"
RUN_DIR="$FLOW_DIR/.run"
RUN_ID="${RUN_ID:-$(date +%Y%m%d-%H%M%S)}"
if [[ ! "$RUN_ID" =~ ^[A-Za-z0-9._-]+$ ]]; then
  echo "RUN_ID may contain only letters, numbers, dot, underscore, and dash." >&2
  exit 1
fi
export RUN_ID
RUN_STATE_DIR="${RUN_STATE_DIR:-$RUN_DIR/state/$RUN_ID}"

if [[ ! -f "$ENV_FILE" ]]; then
  echo "Missing $ENV_FILE. Copy env.example first." >&2
  exit 1
fi

set -a
CLI_SUBMIT="${SUBMIT:-}"
CLI_PROVIDER="${PROVIDER:-}"
CLI_BLOCKFROST_KEY_FILE="${BLOCKFROST_KEY_FILE:-}"
source "$ENV_FILE"
set +a
if [[ -n "$CLI_SUBMIT" ]]; then
  SUBMIT="$CLI_SUBMIT"
fi
if [[ -n "$CLI_PROVIDER" ]]; then
  PROVIDER="$CLI_PROVIDER"
fi
if [[ -n "$CLI_BLOCKFROST_KEY_FILE" ]]; then
  BLOCKFROST_KEY_FILE="$CLI_BLOCKFROST_KEY_FILE"
fi
export PROVIDER BLOCKFROST_KEY_FILE
if [[ "${NETWORK:-preprod}" != "preprod" ]]; then
  echo "This harness is preprod-only; set NETWORK=preprod." >&2
  exit 1
fi

KOIOS_BASE_URL="${KOIOS_BASE_URL:-https://preprod.koios.rest/api/v1}"
if [[ "${PROVIDER:-koios}" == "blockfrost" ]]; then
  if [[ -z "${BLOCKFROST_KEY_FILE:-}" ]]; then
    echo "BLOCKFROST_KEY_FILE is required when PROVIDER=blockfrost for agent config." >&2
    exit 1
  fi
  EXPLORER_CONFIG="$(jq -n --arg key_path "$BLOCKFROST_KEY_FILE" '{blockfrostKeyPath:$key_path}')"
else
  EXPLORER_CONFIG="$(jq '.explorer' "$ROOT/${BASE_AGENT_CONFIG:-bloom-cardano-agent/resources/preprod.config.json}")"
fi
if [[ -z "${AGENT_CHAIN_SYNC_SLOT:-}" || -z "${AGENT_CHAIN_SYNC_HASH:-}" ]]; then
  tip_json="$(curl -fsS "$KOIOS_BASE_URL/tip")"
  AGENT_CHAIN_SYNC_SLOT="$(printf '%s' "$tip_json" | jq -r '.[0].abs_slot')"
  AGENT_CHAIN_SYNC_HASH="$(printf '%s' "$tip_json" | jq -r '.[0].hash')"
fi
if [[ -z "$AGENT_CHAIN_SYNC_SLOT" || "$AGENT_CHAIN_SYNC_SLOT" == "null" ||
      -z "$AGENT_CHAIN_SYNC_HASH" || "$AGENT_CHAIN_SYNC_HASH" == "null" ]]; then
  echo "Could not resolve preprod chain-sync start point" >&2
  exit 1
fi
case "${AGENT_DISABLE_MEMPOOL:-1}" in
  1|true|TRUE|yes|YES) DISABLE_MEMPOOL_JSON=true ;;
  0|false|FALSE|no|NO) DISABLE_MEMPOOL_JSON=false ;;
  *)
    echo "AGENT_DISABLE_MEMPOOL must be 1/0 or true/false." >&2
    exit 1
    ;;
esac

mkdir -p "$RUN_STATE_DIR" "$RUN_DIR/logs"

jq \
  --arg node_socket "$NODE_SOCKET" \
  --arg operator_key "$OPERATOR_KEY_CBOR_HEX" \
  --arg db_path "$RUN_STATE_DIR/chain-sync" \
  --arg health "$AGENT_HEALTH_ADDR" \
  --argjson chain_sync_slot "$AGENT_CHAIN_SYNC_SLOT" \
  --arg chain_sync_hash "$AGENT_CHAIN_SYNC_HASH" \
  --arg ref_tx "$AUCTION_VALIDATOR_REF_TX_HASH" \
  --argjson ref_ix "$AUCTION_VALIDATOR_REF_OUTPUT_INDEX" \
  --arg hash "$AUCTION_VALIDATOR_HASH" \
  --argjson cost_mem "$AUCTION_VALIDATOR_COST_MEM" \
  --argjson cost_steps "$AUCTION_VALIDATOR_COST_STEPS" \
  --argjson marginal_mem "$AUCTION_VALIDATOR_MARGINAL_COST_MEM" \
  --argjson marginal_steps "$AUCTION_VALIDATOR_MARGINAL_COST_STEPS" \
  --argjson max_cost "$AUCTION_MAX_COST_PER_EX_STEP" \
  --argjson min_out "$AUCTION_MIN_MARGINAL_OUTPUT" \
  --argjson explorer "$EXPLORER_CONFIG" \
  --argjson disable_mempool "$DISABLE_MEMPOOL_JSON" \
  '
  .node.path = $node_socket
  | .operatorKey = $operator_key
  | .explorer = $explorer
  | .disableMempool = $disable_mempool
  | .chainSync.startingPoint = { Specific: [$chain_sync_slot, $chain_sync_hash] }
  | .chainSync.replayFromPoint = { Specific: [$chain_sync_slot, $chain_sync_hash] }
  | .chainSync.disableRollbacksUntil = $chain_sync_slot
  | .chainSync.dbPath = $db_path
  | .healthListenAddr = $health
  | .auctionOrders = [{
      validator: {
        hash: $hash,
        referenceUtxo: { txHash: $ref_tx, outputIndex: $ref_ix },
        cost: { mem: $cost_mem, steps: $cost_steps },
        marginalCost: { mem: $marginal_mem, steps: $marginal_steps }
      },
      maxCostPerExStep: $max_cost,
      minMarginalOutput: $min_out
    }]
  ' \
  "$ROOT/${BASE_AGENT_CONFIG:-bloom-cardano-agent/resources/preprod.config.json}" \
  > "$RUN_DIR/agent.${NETWORK:-preprod}.auction.json"

echo "$RUN_DIR/agent.${NETWORK:-preprod}.auction.json"
