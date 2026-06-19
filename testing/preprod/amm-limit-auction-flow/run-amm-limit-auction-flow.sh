#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"
FLOW_DIR="$ROOT/testing/preprod/amm-limit-auction-flow"
DEFAULT_ENV_FILE="$FLOW_DIR/.env"
EXAMPLE_ENV_FILE="$FLOW_DIR/env.example"
if [[ -n "${FLOW_ENV_FILE:-}" ]]; then
  BASE_ENV_FILE="$FLOW_ENV_FILE"
elif [[ -f "$DEFAULT_ENV_FILE" ]]; then
  BASE_ENV_FILE="$DEFAULT_ENV_FILE"
else
  BASE_ENV_FILE="$EXAMPLE_ENV_FILE"
fi
RUN_DIR="$FLOW_DIR/.run"
RUN_ID="${RUN_ID:-auditor-$(date +%Y%m%d-%H%M%S)}"
FUNDING_CONFIRMED=0
BLOCKFROST_KEY_FILE=""
DEFAULT_REQUIRED_WALLET_LOVELACE=500000000
AUDITOR_FRESH_SETUP="${AUDITOR_FRESH_SETUP:-1}"
DENO_STEP_TIMEOUT_SECS="${DENO_STEP_TIMEOUT_SECS:-240}"
PROVIDER="${PROVIDER:-blockfrost}"
PUBLISH_ORDERS_IN_ONE_TX="${PUBLISH_ORDERS_IN_ONE_TX:-1}"
START_AGENT_BEFORE_ORDERS="${START_AGENT_BEFORE_ORDERS:-1}"
RUN_AMM_LIMIT_PHASE="${RUN_AMM_LIMIT_PHASE:-1}"
AGENT_DISABLE_MEMPOOL="${AGENT_DISABLE_MEMPOOL:-1}"
RESTART_AGENT_AFTER_AUCTION_CONFIRM="${RESTART_AGENT_AFTER_AUCTION_CONFIRM:-0}"
export AUDITOR_FRESH_SETUP DENO_STEP_TIMEOUT_SECS PROVIDER PUBLISH_ORDERS_IN_ONE_TX
export START_AGENT_BEFORE_ORDERS
export RUN_AMM_LIMIT_PHASE
export AGENT_DISABLE_MEMPOOL RESTART_AGENT_AFTER_AUCTION_CONFIRM

NODE_SOCKET_ARG=""
AUTO_CONFIRM=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --node-socket)
      NODE_SOCKET_ARG="${2:?--node-socket requires a path}"
      shift 2
      ;;
    --run-id)
      RUN_ID="${2:?--run-id requires a value}"
      shift 2
      ;;
    *)
      echo "Usage: $0 [--node-socket PATH] [--run-id RUN_ID]" >&2
      exit 1
      ;;
  esac
done

if [[ ! "$RUN_ID" =~ ^[A-Za-z0-9._-]+$ ]]; then
  echo "RUN_ID may contain only letters, numbers, dot, underscore, and dash." >&2
  exit 1
fi
RUN_ENV_FILE="$RUN_DIR/env/$RUN_ID.env"
RUN_STATE_DIR="$RUN_DIR/state/$RUN_ID"
RUN_WALLET_SEED_FILE="$RUN_DIR/wallets/$RUN_ID.seed"
LOG_DIR="$RUN_DIR/logs/$RUN_ID"
REPORT_FILE="$RUN_DIR/reports/$RUN_ID.json"

AGENT_PID=""
cleanup() {
  local exit_code=$?
  if [[ -n "${AGENT_PID:-}" ]]; then
    kill "$AGENT_PID" 2>/dev/null || true
    wait "$AGENT_PID" 2>/dev/null || true
  fi
  if [[ -n "${BLOCKFROST_KEY_FILE:-}" ]]; then
    rm -f "$BLOCKFROST_KEY_FILE"
  fi
  if [[ $exit_code -ne 0 ]]; then
    rm -rf "$RUN_STATE_DIR"
    echo "auditor flow failed; cleaned state $RUN_STATE_DIR" >&2
    echo "logs are in $LOG_DIR" >&2
  fi
  exit "$exit_code"
}
trap cleanup EXIT INT TERM

require_file() {
  if [[ ! -f "$1" ]]; then
    echo "Missing required file: $1" >&2
    exit 1
  fi
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

first_existing_file() {
  local path
  for path in "$@"; do
    if [[ -f "$path" ]]; then
      printf '%s\n' "$path"
      return 0
    fi
  done
  return 1
}

first_existing_socket() {
  local path
  for path in "$@"; do
    if [[ -S "$path" ]]; then
      printf '%s\n' "$path"
      return 0
    fi
  done
  return 1
}

set_env_value() {
  local key="$1"
  local value="$2"
  if grep -q "^${key}=" "$RUN_ENV_FILE"; then
    perl -0pi -e "s#^${key}=.*\$#${key}=${value}#m" "$RUN_ENV_FILE"
  else
    printf '%s=%s\n' "$key" "$value" >> "$RUN_ENV_FILE"
  fi
}

koios_post() {
  local path="$1"
  local body="$2"
  curl -fsS -H 'content-type: application/json' \
    "https://preprod.koios.rest/api/v1/$path" \
    -d "$body"
}

blockfrost_get() {
  local path="$1"
  if [[ -z "${BLOCKFROST_PROJECT_ID:-}" ]]; then
    echo "BLOCKFROST_PROJECT_ID is required for Blockfrost lookup." >&2
    return 1
  fi
  curl -fsS -H "project_id: $BLOCKFROST_PROJECT_ID" \
    "https://cardano-preprod.blockfrost.io/api/v0/$path"
}

activate_blockfrost() {
  local project_id="${BLOCKFROST_PROJECT_ID:-}"
  if [[ -z "$project_id" ]]; then
    if [[ ! -t 0 ]]; then
      echo "BLOCKFROST_PROJECT_ID is required for Blockfrost, but stdin is not interactive." >&2
      return 1
    fi
    read -r -s -p "Enter Preprod Blockfrost project id: " project_id
    printf '\n' >&2
  fi
  if [[ -z "$project_id" ]]; then
    echo "Empty Blockfrost project id; cannot use Blockfrost." >&2
    return 1
  fi
  BLOCKFROST_KEY_FILE="$RUN_STATE_DIR/blockfrost.key"
  mkdir -p "$RUN_STATE_DIR"
  printf '%s\n' "$project_id" > "$BLOCKFROST_KEY_FILE"
  chmod 600 "$BLOCKFROST_KEY_FILE"
  export PROVIDER=blockfrost
  export BLOCKFROST_PROJECT_ID="$project_id"
  export BLOCKFROST_KEY_FILE
  set_env_value PROVIDER blockfrost
}

tx_info() {
  local tx_hash="$1"
  if [[ "${PROVIDER:-koios}" == "blockfrost" ]]; then
    local body
    if ! body="$(blockfrost_get "txs/$tx_hash" 2>/dev/null)"; then
      printf '[]\n'
      return 0
    fi
    printf '%s' "$body" \
      | jq '[{tx_hash:.hash, block_hash:.block, block_height:.block_height, abs_slot:.slot}]'
  else
    koios_post tx_info "{\"_tx_hashes\":[\"$tx_hash\"]}"
  fi
}

block_info() {
  local block_hash="$1"
  if [[ "${PROVIDER:-koios}" == "blockfrost" ]]; then
    blockfrost_get "blocks/$block_hash" \
      | jq '[{hash:.hash, block_height:.height, abs_slot:.slot, parent_hash:.previous_block}]'
  else
    koios_post block_info "{\"_block_hashes\":[\"$block_hash\"]}"
  fi
}

activate_blockfrost_fallback() {
  if [[ "${PROVIDER:-koios}" != "koios" ]]; then
    return 1
  fi
  echo "Koios returned HTTP 429. Falling back to Blockfrost for transaction-building steps." >&2
  activate_blockfrost
}

run_deno_json() {
  local out_file="$1"
  local script="$2"
  shift 2
  local err_file="$out_file.stderr"
  local timeout_secs="${DENO_STEP_TIMEOUT_SECS:-90}"
  while true; do
    deno run --allow-read --allow-write --allow-env --allow-net "$script" "$@" > "$out_file" 2> "$err_file" &
    local deno_pid="$!"
    local deadline=$((SECONDS + timeout_secs))
    local exit_code=""
    while (( SECONDS < deadline )); do
      if ! kill -0 "$deno_pid" 2>/dev/null; then
        set +e
        wait "$deno_pid"
        exit_code="$?"
        set -e
        break
      fi
      sleep 1
    done
    if [[ -z "$exit_code" ]]; then
      kill "$deno_pid" 2>/dev/null || true
      wait "$deno_pid" 2>/dev/null || true
      exit_code=124
      printf 'Deno step timed out after %s seconds: %s\n' "$timeout_secs" "$script" >> "$err_file"
    fi
    if [[ "$exit_code" == "0" ]]; then
      cat "$out_file"
      rm -f "$err_file"
      return 0
    fi
    cat "$err_file" >&2 || true
    if { grep -q "KoiosError:.*429" "$err_file" || [[ "$exit_code" == "124" ]]; } &&
       activate_blockfrost_fallback; then
      rm -f "$out_file" "$err_file"
      continue
    fi
    return "$exit_code"
  done
}

wait_tx_info() {
  local tx_hash="$1"
  local out_file="$2"
  local deadline=$((SECONDS + 240))
  local provider_name="${PROVIDER:-koios}"
  echo "Waiting for ${provider_name} to index tx $tx_hash..."
  while (( SECONDS < deadline )); do
    local body
    body="$(tx_info "$tx_hash")"
    if [[ "$(printf '%s' "$body" | jq 'length')" != "0" ]]; then
      printf '%s\n' "$body" > "$out_file"
      echo "Indexed tx $tx_hash."
      return 0
    fi
    sleep 5
  done
  echo "Timed out waiting for tx $tx_hash in ${provider_name}" >&2
  return 1
}

wait_wallet_funded() {
  local address="$1"
  local required_lovelace="$2"
  local collateral_lovelace="${3:-5000000}"
  if [[ "${PROVIDER:-koios}" != "blockfrost" ]]; then
    return 0
  fi
  local deadline=$((SECONDS + 600))
  echo "Waiting for Blockfrost to show funded wallet $address..."
  while (( SECONDS < deadline )); do
    local body
    local err_file="$LOG_DIR/wait-wallet-funded.err"
    if body="$(blockfrost_get "addresses/$address/utxos?order=desc&count=100" 2>"$err_file")"; then
      local total
      total="$(printf '%s' "$body" | jq '[.[]?.amount[]? | select(.unit == "lovelace") | .quantity | tonumber] | add // 0')"
      local collateral_ready
      collateral_ready="$(printf '%s' "$body" | jq --argjson min "$collateral_lovelace" \
        'any(.[]?; (.amount | length) == 1 and (.amount[0].unit == "lovelace") and ((.amount[0].quantity | tonumber) >= $min))')"
      echo "Observed wallet funding: $total lovelace, collateral_ready=$collateral_ready"
      if (( total >= required_lovelace )) && [[ "$collateral_ready" == "true" ]]; then
        echo "Wallet funded: $total lovelace."
        return 0
      fi
    elif [[ -s "$err_file" ]]; then
      echo "Blockfrost wallet lookup failed: $(tr '\n' ' ' < "$err_file")"
    fi
    sleep 5
  done
  echo "Timed out waiting for funded wallet $address in Blockfrost" >&2
  return 1
}

wait_provider_settle() {
  if [[ "${PROVIDER:-koios}" == "blockfrost" ]]; then
    local settle_secs="${BLOCKFROST_SETTLE_SECS:-20}"
    if (( settle_secs > 0 )); then
      echo "Waiting ${settle_secs}s for Blockfrost wallet UTxO view to settle..."
      sleep "$settle_secs"
    fi
  fi
}

wait_blockfrost_address_utxo() {
  local address="$1"
  local tx_hash="$2"
  local output_index="$3"
  if [[ "${PROVIDER:-koios}" != "blockfrost" ]]; then
    return 0
  fi
  local deadline=$((SECONDS + 240))
  echo "Waiting for Blockfrost address UTxO ${tx_hash}#${output_index}..."
  while (( SECONDS < deadline )); do
    local body
    body="$(blockfrost_get "addresses/$address/utxos?order=desc&count=100")"
    if [[ "$output_index" == "*" ]]; then
      if printf '%s' "$body" | jq -e \
        --arg tx "$tx_hash" \
        'any(.[]; .tx_hash == $tx)' >/dev/null; then
        return 0
      fi
    elif printf '%s' "$body" | jq -e \
      --arg tx "$tx_hash" \
      --argjson ix "$output_index" \
      'any(.[]; .tx_hash == $tx and .output_index == $ix)' >/dev/null; then
      return 0
    fi
    sleep 5
  done
  echo "Timed out waiting for Blockfrost address UTxO ${tx_hash}#${output_index}" >&2
  return 1
}

wait_for_agent_ready() {
  local deadline=$((SECONDS + 240))
  local health_url="http://${AGENT_HEALTH_ADDR:-127.0.0.1:9024}/health"
  while (( SECONDS < deadline )); do
    if ! kill -0 "$AGENT_PID" 2>/dev/null; then
      echo "agent exited before readiness" >&2
      tail -200 "$AGENT_LOG_FILE" >&2 || true
      return 1
    fi
    if curl -sf "$health_url" >/dev/null 2>&1 &&
       grep -q "Tip reached, waiting for new blocks" "$AGENT_LOG_FILE" 2>/dev/null; then
      return 0
    fi
    sleep 2
  done
  echo "agent did not become ready" >&2
  tail -200 "$AGENT_LOG_FILE" >&2 || true
  return 1
}

start_agent_from_tx_info() {
  local tx_info_file="$1"
  if [[ -n "${AGENT_PID:-}" ]]; then
    return 0
  fi
  local block_hash
  block_hash="$(jq -r '.[0].block_hash' "$tx_info_file")"
  local block
  block="$(block_info "$block_hash")"
  local start_block_hash
  start_block_hash="$(printf '%s' "$block" | jq -r '.[0].parent_hash')"
  local start_block
  start_block="$(block_info "$start_block_hash")"
  START_BLOCK_HEIGHT="$(printf '%s' "$start_block" | jq -r '.[0].block_height')"
  AGENT_CHAIN_SYNC_SLOT="$(printf '%s' "$start_block" | jq -r '.[0].abs_slot')"
  AGENT_CHAIN_SYNC_HASH="$(printf '%s' "$start_block" | jq -r '.[0].hash')"
  export AGENT_CHAIN_SYNC_SLOT AGENT_CHAIN_SYNC_HASH
  export RUN_ID RUN_STATE_DIR
  export AGENT_LOG_FILE="$LOG_DIR/agent.log"

  echo "Starting agent from block $START_BLOCK_HEIGHT..."
  SUBMIT=1 BUILD_AGENT="${BUILD_AGENT:-1}" "$FLOW_DIR/run-agent.sh" &
  AGENT_PID="$!"
  echo "Agent PID: $AGENT_PID"
}

restart_agent_from_tx_info() {
  local tx_info_file="$1"
  if [[ -n "${AGENT_PID:-}" ]]; then
    kill "$AGENT_PID" 2>/dev/null || true
    wait "$AGENT_PID" 2>/dev/null || true
    AGENT_PID=""
  fi
  rm -rf "$RUN_STATE_DIR"
  start_agent_from_tx_info "$tx_info_file"
}

require_cmd curl
require_cmd jq
require_cmd perl
require_cmd deno
require_file "$BASE_ENV_FILE"

rm -rf "$RUN_STATE_DIR" "$LOG_DIR"
rm -f "$REPORT_FILE"
mkdir -p "$RUN_DIR/env" "$RUN_DIR/wallets" "$LOG_DIR" "$(dirname "$REPORT_FILE")"
cp "$BASE_ENV_FILE" "$RUN_ENV_FILE"
if [[ "${AUDITOR_FRESH_SETUP:-0}" == "1" && "${REUSE_RUN_WALLET:-0}" != "1" ]]; then
  deno run --allow-write "$FLOW_DIR/generate-wallet-seed.ts" "$RUN_WALLET_SEED_FILE" >/dev/null
  chmod 600 "$RUN_WALLET_SEED_FILE"
  echo "Generated fresh run wallet seed: $RUN_WALLET_SEED_FILE"
elif [[ -f "$RUN_WALLET_SEED_FILE" ]]; then
  echo "Using existing run wallet seed: $RUN_WALLET_SEED_FILE"
else
  deno run --allow-write "$FLOW_DIR/generate-wallet-seed.ts" "$RUN_WALLET_SEED_FILE" >/dev/null
  chmod 600 "$RUN_WALLET_SEED_FILE"
  echo "Generated fresh run wallet seed: $RUN_WALLET_SEED_FILE"
fi
set_env_value WALLET_SEED_FILE "$RUN_WALLET_SEED_FILE"
CLI_REQUIRED_WALLET_LOVELACE="${REQUIRED_WALLET_LOVELACE:-}"
CLI_PROVIDER="${PROVIDER:-}"
CLI_BLOCKFROST_PROJECT_ID="${BLOCKFROST_PROJECT_ID:-}"
CLI_AUCTION_STEP_LEN_SECS="${AUCTION_STEP_LEN_SECS:-}"
CLI_MIN_SPAN_REMAINING_SECS="${MIN_SPAN_REMAINING_SECS:-}"
set -a
source "$RUN_ENV_FILE"
set +a
if [[ -n "$CLI_PROVIDER" ]]; then
  PROVIDER="$CLI_PROVIDER"
fi
if [[ -n "$CLI_BLOCKFROST_PROJECT_ID" ]]; then
  BLOCKFROST_PROJECT_ID="$CLI_BLOCKFROST_PROJECT_ID"
fi

NODE_SOCKET_PATH="${NODE_SOCKET_ARG:-${NODE_SOCKET:-}}"
if [[ -z "$NODE_SOCKET_PATH" || "$NODE_SOCKET_PATH" == "/data/cardano-node/ipc/node.socket" ]]; then
  NODE_SOCKET_PATH="$(
    first_existing_socket \
      "$ROOT/node.socket" \
      "${CARDANO_NODE_SOCKET_PATH:-}" \
      "${CARDANO_NODE_SOCKET:-}" \
      "$HOME/.cardano-node/node.socket" \
      "/data/cardano-node/ipc/node.socket" \
      "/var/lib/cardano-node/node.socket" \
      2>/dev/null || true
  )"
fi
if [[ -t 0 ]]; then
  if [[ -n "$NODE_SOCKET_PATH" ]]; then
    read -r -p "Cardano node socket path [$NODE_SOCKET_PATH]: " NODE_SOCKET_REPLY
    NODE_SOCKET_PATH="${NODE_SOCKET_REPLY:-$NODE_SOCKET_PATH}"
  else
    read -r -p "Cardano node socket path: " NODE_SOCKET_PATH
  fi
fi
if [[ -z "$NODE_SOCKET_PATH" ]]; then
  echo "NODE_SOCKET is required. Pass --node-socket PATH or set NODE_SOCKET." >&2
  exit 1
fi
if [[ ! -S "$NODE_SOCKET_PATH" ]]; then
  echo "Node socket is not available at $NODE_SOCKET_PATH" >&2
  exit 1
fi

set_env_value NETWORK preprod
set_env_value PROVIDER "${PROVIDER:-koios}"
set_env_value SUBMIT 1
set_env_value NODE_SOCKET "$NODE_SOCKET_PATH"
set_env_value AGENT_DISABLE_MEMPOOL "$AGENT_DISABLE_MEMPOOL"
if [[ "${PROVIDER:-koios}" == "blockfrost" ]]; then
  activate_blockfrost
fi

FRESH_SETUP=0
if [[ "${AUDITOR_FRESH_SETUP:-0}" == "1" ]]; then
  FRESH_SETUP=1
fi
if [[ "$FRESH_SETUP" == "1" ]]; then
  if [[ -n "$CLI_REQUIRED_WALLET_LOVELACE" ]]; then
    export REQUIRED_WALLET_LOVELACE="$CLI_REQUIRED_WALLET_LOVELACE"
  else
    export REQUIRED_WALLET_LOVELACE="$DEFAULT_REQUIRED_WALLET_LOVELACE"
  fi
  export FLOW_ENV_FILE="$RUN_ENV_FILE"
  WALLET_INFO_JSON="$LOG_DIR/wallet-info.json"
  run_deno_json "$WALLET_INFO_JSON" "$FLOW_DIR/wallet-info.ts" >/dev/null
  PRE_SETUP_WALLET_ADDRESS="$(jq -r '.wallet' "$WALLET_INFO_JSON")"
  echo "Preprod auditor fresh setup"
  echo "wallet address: $PRE_SETUP_WALLET_ADDRESS"
  echo "wallet minimum: $REQUIRED_WALLET_LOVELACE lovelace (500 tADA default)"
  echo "node socket:    $NODE_SOCKET_PATH"
  if [[ "$AUTO_CONFIRM" != "1" ]]; then
    echo "Send at least $REQUIRED_WALLET_LOVELACE lovelace to the wallet address above for setup, agent funding, order publication, and retry fees, then press Enter."
    read -r
    FUNDING_CONFIRMED=1
  fi
  wait_wallet_funded "$PRE_SETUP_WALLET_ADDRESS" "$REQUIRED_WALLET_LOVELACE"
  echo "Running fresh preprod setup for isolated assets..."
  export NODE_SOCKET="$NODE_SOCKET_PATH"
  export SETUP_TOKEN_SUFFIX="${RUN_ID: -12}"
  run_deno_json "$LOG_DIR/setup-preprod-flow.json" "$FLOW_DIR/setup-preprod-flow.ts"
fi

set_env_value AUCTION_START_TIME_POSIX "$(( $(date +%s) - 60 ))"
set_env_value AUCTION_STEP_LEN_SECS "${CLI_AUCTION_STEP_LEN_SECS:-600}"
set_env_value MIN_SPAN_REMAINING_SECS "${CLI_MIN_SPAN_REMAINING_SECS:-120}"

export FLOW_ENV_FILE="$RUN_ENV_FILE"
RUNTIME_PROVIDER="${PROVIDER:-}"
RUNTIME_BLOCKFROST_PROJECT_ID="${BLOCKFROST_PROJECT_ID:-}"
RUNTIME_BLOCKFROST_KEY_FILE="${BLOCKFROST_KEY_FILE:-}"
set -a
source "$RUN_ENV_FILE"
set +a
if [[ -n "$RUNTIME_PROVIDER" ]]; then
  PROVIDER="$RUNTIME_PROVIDER"
fi
if [[ -n "$RUNTIME_BLOCKFROST_PROJECT_ID" ]]; then
  BLOCKFROST_PROJECT_ID="$RUNTIME_BLOCKFROST_PROJECT_ID"
fi
if [[ -n "$RUNTIME_BLOCKFROST_KEY_FILE" ]]; then
  BLOCKFROST_KEY_FILE="$RUNTIME_BLOCKFROST_KEY_FILE"
fi
export PROVIDER BLOCKFROST_PROJECT_ID BLOCKFROST_KEY_FILE

INFO_JSON="$LOG_DIR/agent-info.json"
run_deno_json "$INFO_JSON" "$FLOW_DIR/agent-info.ts" >/dev/null
WALLET_ADDRESS="$(jq -r '.wallet' "$INFO_JSON")"
OPERATOR_KEY_CBOR_HEX="$(jq -r '.operatorKey' "$INFO_JSON")"
AGENT_FUNDING_ADDRESS="$(jq -r '.funding[0]' "$INFO_JSON")"
AGENT_FUNDING_ADDRESSES=()
while IFS= read -r funding_address; do
  AGENT_FUNDING_ADDRESSES+=("$funding_address")
done < <(jq -r '.funding[]' "$INFO_JSON")
export OPERATOR_KEY_CBOR_HEX
export AGENT_FUNDING_ADDRESS
export AGENT_FUNDING_LOVELACE="${AGENT_FUNDING_LOVELACE:-50000000}"
if [[ "$FRESH_SETUP" == "1" ]]; then
  if [[ -n "$CLI_REQUIRED_WALLET_LOVELACE" ]]; then
    export REQUIRED_WALLET_LOVELACE="$CLI_REQUIRED_WALLET_LOVELACE"
  else
    export REQUIRED_WALLET_LOVELACE="$DEFAULT_REQUIRED_WALLET_LOVELACE"
  fi
else
  export REQUIRED_WALLET_LOVELACE="${REQUIRED_WALLET_LOVELACE:-$DEFAULT_REQUIRED_WALLET_LOVELACE}"
fi
set_env_value OPERATOR_KEY_CBOR_HEX "$OPERATOR_KEY_CBOR_HEX"

echo "Preprod auditor flow"
echo "wallet address:        $WALLET_ADDRESS"
echo "agent funding addresses:"
printf '  %s\n' "${AGENT_FUNDING_ADDRESSES[@]}"
echo "agent funding amount:  $AGENT_FUNDING_LOVELACE lovelace each"
echo "wallet minimum:        $REQUIRED_WALLET_LOVELACE lovelace (500 tADA default)"
echo "node socket:           $NODE_SOCKET_PATH"
if [[ "$AUTO_CONFIRM" != "1" && "$FUNDING_CONFIRMED" != "1" ]]; then
  echo "Send at least $REQUIRED_WALLET_LOVELACE lovelace to the wallet address above for setup, agent funding, order publication, and retry fees, then press Enter."
  read -r
fi

echo "Funding agent..."
if [[ "$FRESH_SETUP" == "1" || "${SKIP_AGENT_FUNDING:-0}" == "1" ]]; then
  if [[ "$FRESH_SETUP" == "1" ]]; then
    FUNDING_TX_HASH="$(jq -r '.mintTxHash // .setupTxHash' "$LOG_DIR/setup-preprod-flow.json")"
  else
    FUNDING_TX_HASH="${SETUP_TX_HASH:-${AUCTION_VALIDATOR_REF_TX_HASH:-}}"
  fi
  if [[ -z "$FUNDING_TX_HASH" ]]; then
    echo "SKIP_AGENT_FUNDING=1 requires SETUP_TX_HASH or AUCTION_VALIDATOR_REF_TX_HASH in env." >&2
    exit 1
  fi
  jq -n \
    --arg txHash "$FUNDING_TX_HASH" \
    --arg address "$AGENT_FUNDING_ADDRESS" \
    --arg lovelace "$AGENT_FUNDING_LOVELACE" \
    '{submit:true, txHash:$txHash, address:$address, value:{lovelace:$lovelace}, source:"pre-funded"}' \
    | tee "$LOG_DIR/fund-agent.json"
else
  AGENT_FUNDING_ADDRESS_LIST="$(IFS=,; printf '%s' "${AGENT_FUNDING_ADDRESSES[*]}")"
  export AGENT_FUNDING_ADDRESS_LIST
  run_deno_json "$LOG_DIR/fund-agent.json" "$FLOW_DIR/fund-agent.ts"
  FUNDING_TX_HASH="$(jq -r '.txHash' "$LOG_DIR/fund-agent.json")"
fi
wait_tx_info "$FUNDING_TX_HASH" "$LOG_DIR/funding-tx-info.json"
wait_blockfrost_address_utxo "$WALLET_ADDRESS" "$FUNDING_TX_HASH" "*"
wait_provider_settle
if [[ "${START_AGENT_BEFORE_ORDERS:-0}" == "1" ]]; then
  start_agent_from_tx_info "$LOG_DIR/funding-tx-info.json"
  wait_for_agent_ready
fi

AMM_POOL_TX_HASH=""
AMM_LIMIT_TX_HASH=""
AMM_LIMIT_EXECUTION_TX=""
if [[ "${RUN_AMM_LIMIT_PHASE:-1}" == "1" ]]; then
  echo "Deploying AMM pool for auditor limit-order execution check..."
  export POOL_TOKEN_SUFFIX="${RUN_ID: -12}"
  export POOL_MINT_DEMO_ASSETS=1
  export POOL_WALLET_X_AMOUNT="${POOL_WALLET_X_AMOUNT:-0}"
  export POOL_WALLET_Y_AMOUNT="${POOL_WALLET_Y_AMOUNT:-3000}"
  run_deno_json "$LOG_DIR/deploy-amm-pool.json" "$FLOW_DIR/deploy-amm-pool.ts"
  AMM_POOL_TX_HASH="$(jq -r '.txHash' "$LOG_DIR/deploy-amm-pool.json")"
  wait_tx_info "$AMM_POOL_TX_HASH" "$LOG_DIR/amm-pool-tx-info.json"
  wait_blockfrost_address_utxo "$WALLET_ADDRESS" "$AMM_POOL_TX_HASH" "*"
  wait_provider_settle

  LIMIT_INPUT_POLICY="$(jq -r '.assetY.policy' "$LOG_DIR/deploy-amm-pool.json")"
  LIMIT_INPUT_NAME_HEX="$(jq -r '.assetY.nameHex' "$LOG_DIR/deploy-amm-pool.json")"
  LIMIT_OUTPUT_POLICY="$(jq -r '.assetX.policy' "$LOG_DIR/deploy-amm-pool.json")"
  LIMIT_OUTPUT_NAME_HEX="$(jq -r '.assetX.nameHex' "$LOG_DIR/deploy-amm-pool.json")"
  export LIMIT_INPUT_POLICY LIMIT_INPUT_NAME_HEX LIMIT_OUTPUT_POLICY LIMIT_OUTPUT_NAME_HEX
  export LIMIT_TRADABLE_INPUT="${AMM_LIMIT_TRADABLE_INPUT:-2000}"
  export LIMIT_MIN_MARGINAL_OUTPUT="${AMM_LIMIT_MIN_MARGINAL_OUTPUT:-900}"
  export LIMIT_BASE_PRICE_NUM="${AMM_LIMIT_BASE_PRICE_NUM:-1}"
  export LIMIT_BASE_PRICE_DENOM="${AMM_LIMIT_BASE_PRICE_DENOM:-3}"
  export LIMIT_LOVELACE_BUDGET="${AMM_LIMIT_LOVELACE_BUDGET:-2100000}"
  export LIMIT_MINT_DEMO_INPUT=0
  set_env_value LIMIT_INPUT_POLICY "$LIMIT_INPUT_POLICY"
  set_env_value LIMIT_INPUT_NAME_HEX "$LIMIT_INPUT_NAME_HEX"
  set_env_value LIMIT_OUTPUT_POLICY "$LIMIT_OUTPUT_POLICY"
  set_env_value LIMIT_OUTPUT_NAME_HEX "$LIMIT_OUTPUT_NAME_HEX"

  echo "Creating limit order against deployed AMM pool..."
  run_deno_json "$LOG_DIR/create-amm-limit-order.json" "$FLOW_DIR/create-limit-order.ts"
  AMM_LIMIT_TX_HASH="$(jq -r '.txHash' "$LOG_DIR/create-amm-limit-order.json")"
  AMM_LIMIT_OUTPUT_INDEX="$(jq -r '.outputIndex' "$LOG_DIR/create-amm-limit-order.json")"
  export LIMIT_TX_HASH="$AMM_LIMIT_TX_HASH"
  export LIMIT_OUTPUT_INDEX="$AMM_LIMIT_OUTPUT_INDEX"
  wait_tx_info "$AMM_LIMIT_TX_HASH" "$LOG_DIR/amm-limit-tx-info.json"

  if [[ "${RUN_AMM_LIMIT_EXECUTION_VERIFY:-0}" == "1" ]]; then
    echo "Verifying AMM-backed limit order execution..."
    VERIFY_TIMEOUT_SECS="${VERIFY_TIMEOUT_SECS:-900}" \
    VERIFY_POLL_SECS="${VERIFY_POLL_SECS:-10}" \
    run_deno_json "$LOG_DIR/verify-limit-order-flow.json" "$FLOW_DIR/verify-limit-order-flow.ts"
    AMM_LIMIT_EXECUTION_TX="$(jq -r '.spendingTx // empty' "$LOG_DIR/verify-limit-order-flow.json")"
  else
    jq -n \
      --arg status "published_on_preprod" \
      --arg txHash "$AMM_LIMIT_TX_HASH" \
      --arg outputIndex "$AMM_LIMIT_OUTPUT_INDEX" \
      '{status:$status, txHash:$txHash, outputIndex:($outputIndex | tonumber)}' \
      | tee "$LOG_DIR/verify-limit-order-flow.json"
  fi
fi

if [[ -n "${REUSE_ORDER_LOG_DIR:-}" ]]; then
  require_file "$REUSE_ORDER_LOG_DIR/create-counter-limit-order.json"
  require_file "$REUSE_ORDER_LOG_DIR/create-auction-order.json"
  echo "Reusing existing counter limit order and auction order from $REUSE_ORDER_LOG_DIR..."
  cp "$REUSE_ORDER_LOG_DIR/create-counter-limit-order.json" "$LOG_DIR/create-counter-limit-order.json"
  cp "$REUSE_ORDER_LOG_DIR/create-auction-order.json" "$LOG_DIR/create-auction-order.json"
  if [[ -f "$REUSE_ORDER_LOG_DIR/create-order-pair.json" ]]; then
    cp "$REUSE_ORDER_LOG_DIR/create-order-pair.json" "$LOG_DIR/create-order-pair.json"
  fi
elif [[ "${PUBLISH_ORDERS_IN_ONE_TX:-1}" == "1" ]]; then
  if [[ -n "${ORDER_INPUT_TX_HASH:-}" ]]; then
    wait_blockfrost_address_utxo "$WALLET_ADDRESS" "$ORDER_INPUT_TX_HASH" "*"
  fi
  echo "Creating counter limit order and auction order in one transaction..."
  run_deno_json "$LOG_DIR/create-order-pair.json" "$FLOW_DIR/create-order-pair.ts"
  jq '.counter' "$LOG_DIR/create-order-pair.json" > "$LOG_DIR/create-counter-limit-order.json"
  jq '.auction' "$LOG_DIR/create-order-pair.json" > "$LOG_DIR/create-auction-order.json"
else
  echo "Creating counter limit order..."
  run_deno_json "$LOG_DIR/create-counter-limit-order.json" "$FLOW_DIR/create-counter-limit-order.ts"
fi
TARGET_COUNTER_ORDER_TX_HASH="$(jq -r '.txHash' "$LOG_DIR/create-counter-limit-order.json")"
TARGET_COUNTER_ORDER_OUTPUT_INDEX="$(jq -r '.outputIndex' "$LOG_DIR/create-counter-limit-order.json")"
TARGET_COUNTER_ORDER_PRICE_NUM="$(jq -r '.counterPriceNum' "$LOG_DIR/create-counter-limit-order.json")"
TARGET_COUNTER_ORDER_PRICE_DENOM="$(jq -r '.counterPriceDenom' "$LOG_DIR/create-counter-limit-order.json")"
TARGET_COUNTER_ORDER_DATUM="$(jq -r '.datum' "$LOG_DIR/create-counter-limit-order.json")"
export TARGET_COUNTER_ORDER_TX_HASH TARGET_COUNTER_ORDER_OUTPUT_INDEX
export TARGET_COUNTER_ORDER_PRICE_NUM TARGET_COUNTER_ORDER_PRICE_DENOM
export TARGET_COUNTER_ORDER_DATUM
wait_tx_info "$TARGET_COUNTER_ORDER_TX_HASH" "$LOG_DIR/counter-tx-info.json"
if [[ "${START_AGENT_BEFORE_ORDERS:-0}" != "1" ]]; then
  wait_provider_settle
fi

echo "Running exact-liquidity preflight..."
run_deno_json "$LOG_DIR/preflight-exact-liquidity.json" "$FLOW_DIR/preflight-exact-liquidity.ts"

if [[ -z "${REUSE_ORDER_LOG_DIR:-}" && "${PUBLISH_ORDERS_IN_ONE_TX:-1}" != "1" ]]; then
  wait_blockfrost_address_utxo "$WALLET_ADDRESS" "$TARGET_COUNTER_ORDER_TX_HASH" 1
  echo "Creating auction order..."
  run_deno_json "$LOG_DIR/create-auction-order.json" "$FLOW_DIR/create-auction-order.ts"
fi
AUCTION_TX_HASH="$(jq -r '.txHash' "$LOG_DIR/create-auction-order.json")"
AUCTION_OUTPUT_INDEX="$(jq -r '.outputIndex' "$LOG_DIR/create-auction-order.json")"
AUCTION_INITIAL_DATUM="$(jq -r '.datum' "$LOG_DIR/create-auction-order.json")"
export AUCTION_TX_HASH AUCTION_OUTPUT_INDEX AUCTION_INITIAL_DATUM
wait_tx_info "$AUCTION_TX_HASH" "$LOG_DIR/auction-tx-info.json"
if [[ "${START_AGENT_BEFORE_ORDERS:-0}" != "1" ]]; then
  wait_provider_settle
fi
if [[ "${RESTART_AGENT_AFTER_AUCTION_CONFIRM:-1}" == "1" ]]; then
  echo "Restarting agent from confirmed auction-order block..."
  restart_agent_from_tx_info "$LOG_DIR/auction-tx-info.json"
fi

if [[ -z "${AGENT_PID:-}" ]]; then
  EARLIEST_BLOCK_HASH="$(
    jq -s -r '[.[0][0], .[1][0], .[2][0]] | min_by(.block_height) | .block_hash' \
      "$LOG_DIR/funding-tx-info.json" \
      "$LOG_DIR/counter-tx-info.json" \
      "$LOG_DIR/auction-tx-info.json"
  )"
  EARLIEST_TX_INFO="$LOG_DIR/earliest-tx-info.json"
  jq -n --arg block_hash "$EARLIEST_BLOCK_HASH" '[{block_hash:$block_hash}]' > "$EARLIEST_TX_INFO"
  start_agent_from_tx_info "$EARLIEST_TX_INFO"
fi
wait_for_agent_ready

echo "Verifying auction execution..."
VERIFY_TIMEOUT_SECS="${VERIFY_TIMEOUT_SECS:-900}" \
VERIFY_POLL_SECS="${VERIFY_POLL_SECS:-10}" \
run_deno_json "$LOG_DIR/verify-auction-flow.json" "$FLOW_DIR/verify-auction-flow.ts"

jq -n \
  --arg runId "$RUN_ID" \
  --arg wallet "$WALLET_ADDRESS" \
  --arg fundingAddress "$AGENT_FUNDING_ADDRESS" \
  --arg fundingTx "$FUNDING_TX_HASH" \
  --arg counterTx "$TARGET_COUNTER_ORDER_TX_HASH" \
  --arg auctionTx "$AUCTION_TX_HASH" \
  --arg auctionRef "$AUCTION_TX_HASH#$AUCTION_OUTPUT_INDEX" \
  --arg executionTx "$(jq -r '.spendingTx // empty' "$LOG_DIR/verify-auction-flow.json")" \
  --arg ammPoolTx "$AMM_POOL_TX_HASH" \
  --arg ammLimitTx "$AMM_LIMIT_TX_HASH" \
  --arg ammLimitExecutionTx "$AMM_LIMIT_EXECUTION_TX" \
  --arg logs "$LOG_DIR" \
  '{status:"ok", runId:$runId, wallet:$wallet, fundingAddress:$fundingAddress, fundingTx:$fundingTx, ammPoolTx:$ammPoolTx, ammLimitTx:$ammLimitTx, ammLimitExecutionTx:$ammLimitExecutionTx, counterTx:$counterTx, auctionTx:$auctionTx, auctionRef:$auctionRef, executionTx:$executionTx, logs:$logs}' \
  | tee "$REPORT_FILE"

echo "Auditor flow completed. Report: $REPORT_FILE"
