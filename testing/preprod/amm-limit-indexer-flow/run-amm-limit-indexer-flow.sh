#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
AMM_FLOW_DIR="${REPO_ROOT}/testing/preprod/amm-limit-auction-flow"
BATCHER_FLOW_DIR="${REPO_ROOT}/testing/preprod/batcher-indexer-flow"
RUN_BASE="${SCRIPT_DIR}/.run"
RUN_ID="${RUN_ID:-amm-limit-indexer-$(date -u +%Y%m%d-%H%M%S)}"
RUN_ROOT="${RUN_BASE}/runs/${RUN_ID}"
STATE_DIR="${RUN_ROOT}/state"
LOG_DIR="${RUN_ROOT}/logs"
REPORT_DIR="${RUN_BASE}/reports"
REPORT_FILE="${REPORT_DIR}/${RUN_ID}.json"
FLOW_ENV_FILE="${RUN_ROOT}/flow.env"
INDEXER_PID=""
AGENT_PID=""
BLOCKFROST_KEY_FILE=""
SETUP_JSON="${LOG_DIR}/setup-preprod-flow.json"
WALLET_INFO_JSON="${LOG_DIR}/wallet-info.json"
AGENT_INFO_JSON="${LOG_DIR}/agent-info.json"
GOOD_ORDER_LOG_DIR="${LOG_DIR}/orders/good"
BAD_ORDER_LOG_DIR="${LOG_DIR}/orders/bad"
INDEXER_RUN_ID="${RUN_ID}-indexer"
INDEXER_REPORT_FILE="${BATCHER_FLOW_DIR}/.run/reports/${INDEXER_RUN_ID}.json"

cleanup() {
  local status=$?
  if [[ -n "${INDEXER_PID:-}" ]] && ps -p "${INDEXER_PID}" >/dev/null 2>&1; then
    kill "${INDEXER_PID}" >/dev/null 2>&1 || true
    wait "${INDEXER_PID}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${AGENT_PID:-}" ]] && ps -p "${AGENT_PID}" >/dev/null 2>&1; then
    kill "${AGENT_PID}" >/dev/null 2>&1 || true
    wait "${AGENT_PID}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${BLOCKFROST_KEY_FILE:-}" ]]; then
    rm -f "${BLOCKFROST_KEY_FILE}" >/dev/null 2>&1 || true
  fi
  if [[ "$status" != "0" ]]; then
    echo "amm limit indexer auditor flow failed; run state is in ${RUN_ROOT}" >&2
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

run_deno_json() {
  local out_file="$1"
  local script="$2"
  shift 2
  deno run --config "${AMM_FLOW_DIR}/deno.json" --allow-read --allow-write --allow-env --allow-net --allow-ffi "$script" "$@" \
    > "$out_file"
  cat "$out_file"
}

set_env_value() {
  local key="$1"
  local value="$2"
  if grep -q "^${key}=" "$FLOW_ENV_FILE" 2>/dev/null; then
    perl -0pi -e "s#^${key}=.*\$#${key}=${value}#m" "$FLOW_ENV_FILE"
  else
    printf '%s=%s\n' "$key" "$value" >> "$FLOW_ENV_FILE"
  fi
}

update_min_unix_time() {
  local current_min="$1"
  local candidate="$2"
  if [[ -z "$candidate" || "$candidate" == "null" ]]; then
    printf '%s\n' "$current_min"
    return
  fi
  if [[ -z "$current_min" || "$current_min" == "null" || "$candidate" -lt "$current_min" ]]; then
    printf '%s\n' "$candidate"
  else
    printf '%s\n' "$current_min"
  fi
}

blockfrost_get() {
  local path="$1"
  curl -fsS -H "project_id: ${BLOCKFROST_PROJECT_ID}" \
    "https://cardano-preprod.blockfrost.io/api/v0/$path"
}

wait_blockfrost_address_utxo() {
  local address="$1"
  local tx_hash="$2"
  local output_index="$3"
  local deadline=$((SECONDS + 240))
  while (( SECONDS < deadline )); do
    local body
    body="$(blockfrost_get "addresses/$address/utxos?order=desc&count=100")"
    if [[ "$output_index" == "*" ]]; then
      if printf '%s' "$body" | jq -e --arg tx "$tx_hash" 'any(.[]; .tx_hash == $tx)' >/dev/null; then
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
  echo "Timed out waiting for Blockfrost UTxO ${tx_hash}#${output_index} at ${address}" >&2
  return 1
}

wait_provider_settle() {
  local settle_secs="${BLOCKFROST_SETTLE_SECS:-20}"
  if (( settle_secs > 0 )); then
    sleep "$settle_secs"
  fi
}

wait_wallet_asset_utxo() {
  local policy="$1"
  local name_hex="$2"
  local min_amount="$3"
  local min_lovelace="${4:-0}"
  WAIT_ASSET_POLICY="$policy" \
  WAIT_ASSET_NAME_HEX="$name_hex" \
  WAIT_MIN_AMOUNT="$min_amount" \
  WAIT_MIN_LOVELACE="$min_lovelace" \
  WAIT_TIMEOUT_SECS="${WAIT_TIMEOUT_SECS:-240}" \
  WAIT_POLL_SECS="${WAIT_POLL_SECS:-5}" \
  run_deno_json "${LOG_DIR}/wait-wallet-asset-utxo.json" \
    "${SCRIPT_DIR}/wait-wallet-asset-utxo.ts"
}

wait_wallet_funded() {
  local address="$1"
  local required_lovelace="$2"
  local collateral_lovelace="${3:-5000000}"
  local deadline=$((SECONDS + 900))
  while (( SECONDS < deadline )); do
    local body
    body="$(blockfrost_get "addresses/$address/utxos?order=desc&count=100")"
    local total
    total="$(printf '%s' "$body" | jq '[.[]?.amount[]? | select(.unit == "lovelace") | .quantity | tonumber] | add // 0')"
    local collateral_ready
    if (( collateral_lovelace <= 0 )); then
      collateral_ready="true"
    else
      collateral_ready="$(printf '%s' "$body" | jq --argjson min "$collateral_lovelace" \
        'any(.[]?; (.amount | length) == 1 and (.amount[0].unit == "lovelace") and ((.amount[0].quantity | tonumber) >= $min))')"
    fi
    if (( total >= required_lovelace )) && [[ "$collateral_ready" == "true" ]]; then
      return 0
    fi
    sleep 5
  done
  echo "Timed out waiting for wallet funding at ${address}" >&2
  return 1
}

have_resume_setup() {
  [[ "${REUSE_EXISTING_WALLET:-0}" == "1" ]] \
    && [[ -f "$SETUP_JSON" ]] \
    && [[ -f "${LOG_DIR}/funding-tx-info.json" ]] \
    && [[ -f "$WALLET_INFO_JSON" ]]
}

have_resume_pool() {
  [[ "${REUSE_EXISTING_WALLET:-0}" == "1" ]] \
    && [[ -f "${LOG_DIR}/deploy-royalty-pool.json" ]] \
    && [[ -f "${LOG_DIR}/amm-pool-tx-info.json" ]]
}

hydrate_auction_env_from_setup() {
  local base_unit quote_unit
  base_unit="$(jq -r '.baseUnit' "$SETUP_JSON")"
  quote_unit="$(jq -r '.quoteUnit' "$SETUP_JSON")"
  set_env_value AUCTION_BASE_POLICY "${base_unit:0:56}"
  set_env_value AUCTION_BASE_NAME_HEX "${base_unit:56}"
  set_env_value AUCTION_QUOTE_POLICY "${quote_unit:0:56}"
  set_env_value AUCTION_QUOTE_NAME_HEX "${quote_unit:56}"
  set_env_value AUCTION_BASE_AMOUNT "${AUCTION_BASE_AMOUNT:-1000}"
  set_env_value AUCTION_PRICE_START_NUM "${AUCTION_PRICE_START_NUM:-2}"
  set_env_value AUCTION_PRICE_START_DENOM "${AUCTION_PRICE_START_DENOM:-1}"
  set_env_value AUCTION_START_TIME_POSIX "${AUCTION_START_TIME_POSIX:-0}"
}

wait_tx_info() {
  local tx_hash="$1"
  local out_file="$2"
  local deadline=$((SECONDS + 240))
  while (( SECONDS < deadline )); do
    local body
    if body="$(blockfrost_get "txs/$tx_hash" 2>/dev/null)"; then
      printf '%s' "$body" | jq '[{tx_hash:.hash, block_hash:.block, block_height:.block_height, abs_slot:.slot, block_time:.block_time}]' > "$out_file"
      return 0
    fi
    sleep 5
  done
  echo "Timed out waiting for tx $tx_hash in Blockfrost" >&2
  return 1
}

recover_exec_tx_info_from_verify() {
  local verify_file="$1"
  local out_file="$2"
  if [[ -f "$out_file" ]]; then
    return 0
  fi
  if [[ ! -f "$verify_file" ]]; then
    return 1
  fi
  local exec_tx
  exec_tx="$(jq -r '.spendingTx // empty' "$verify_file")"
  if [[ -z "$exec_tx" || "$exec_tx" == "null" ]]; then
    return 1
  fi
  wait_tx_info "$exec_tx" "$out_file"
}

normalize_classic_pool_hashes() {
  local deployment_config="$1"
  local swap_hash
  swap_hash="$(jq -r '.constFnPoolSwap.hash // empty' "$deployment_config")"
  if [[ -z "$swap_hash" ]]; then
    echo "Deployment config ${deployment_config} does not contain constFnPoolSwap.hash" >&2
    return 1
  fi
  jq --arg swap_hash "$swap_hash" '
    .constFnPoolV1.hash = $swap_hash
    | .constFnPoolV2.hash = $swap_hash
    | if has("constFnPoolFeeSwitchBidirFee") then
        .constFnPoolFeeSwitchBidirFee.hash = $swap_hash
      else
        .
      end
  ' "$deployment_config" > "${deployment_config}.tmp"
  mv "${deployment_config}.tmp" "$deployment_config"
}

block_info() {
  local block_ref="$1"
  blockfrost_get "blocks/$block_ref" \
    | jq '[{hash:.hash, block_height:.height, abs_slot:.slot, parent_hash:.previous_block, block_time:.time}]'
}

launch_agent_process() {
  (
    cd "$REPO_ROOT"
    FLOW_ENV_FILE="$FLOW_ENV_FILE" \
    RUN_ID="$RUN_ID" \
    RUN_STATE_DIR="$RUN_STATE_DIR" \
    AGENT_LOG_FILE="$AGENT_LOG_FILE" \
    BUILD_AGENT="${BUILD_AGENT:-1}" \
    SUBMIT=1 \
    PROVIDER=blockfrost \
    BLOCKFROST_KEY_FILE="$BLOCKFROST_KEY_FILE" \
    "${AMM_FLOW_DIR}/run-agent.sh"
  ) &
  AGENT_PID="$!"
}

rewind_agent_chain_sync_point() {
  local steps="${1:-32}"
  local current_hash="${AGENT_CHAIN_SYNC_HASH:-}"
  if [[ -z "$current_hash" ]]; then
    echo "cannot rewind agent chain-sync point: AGENT_CHAIN_SYNC_HASH is empty" >&2
    return 1
  fi

  local block=""
  local parent_hash=""
  local i=0
  while (( i < steps )); do
    block="$(block_info "$current_hash")"
    parent_hash="$(printf '%s' "$block" | jq -r '.[0].parent_hash // empty')"
    if [[ -z "$parent_hash" || "$parent_hash" == "null" ]]; then
      break
    fi
    current_hash="$parent_hash"
    i=$((i + 1))
  done

  block="$(block_info "$current_hash")"
  export AGENT_CHAIN_SYNC_SLOT
  AGENT_CHAIN_SYNC_SLOT="$(printf '%s' "$block" | jq -r '.[0].abs_slot')"
  export AGENT_CHAIN_SYNC_HASH
  AGENT_CHAIN_SYNC_HASH="$(printf '%s' "$block" | jq -r '.[0].hash')"
  printf 'rewound agent chain-sync point by %s blocks to slot=%s hash=%s\n' \
    "$i" "$AGENT_CHAIN_SYNC_SLOT" "$AGENT_CHAIN_SYNC_HASH" >&2
}

reset_agent_chain_sync_point_to_tip() {
  unset AGENT_CHAIN_SYNC_SLOT
  unset AGENT_CHAIN_SYNC_HASH
  printf 'reset agent chain-sync point to current tip fallback\n' >&2
}

wait_for_agent_ready() {
  local deadline=$((SECONDS + 300))
  local health_url="http://${AGENT_HEALTH_ADDR:-127.0.0.1:9024}/health"
  local intersection_retries=0
  while (( SECONDS < deadline )); do
    if ! kill -0 "$AGENT_PID" 2>/dev/null; then
      if grep -q "IntersectionNotFound" "${LOG_DIR}/agent.log" 2>/dev/null && (( intersection_retries < 5 )); then
        intersection_retries=$((intersection_retries + 1))
        printf 'agent startup hit IntersectionNotFound, retrying (%s/5)\n' "$intersection_retries" >&2
        if (( intersection_retries < 4 )); then
          rewind_agent_chain_sync_point $((intersection_retries * 32))
        else
          reset_agent_chain_sync_point_to_tip
        fi
        sleep 5
        launch_agent_process
        continue
      fi
      echo "agent exited before readiness" >&2
      tail -200 "${LOG_DIR}/agent.log" >&2 || true
      return 1
    fi
    if curl -sf "$health_url" >/dev/null 2>&1 &&
       grep -q "Tip reached, waiting for new blocks" "${LOG_DIR}/agent.log" 2>/dev/null; then
      return 0
    fi
    sleep 2
  done
  echo "agent did not become ready" >&2
  tail -200 "${LOG_DIR}/agent.log" >&2 || true
  return 1
}

start_agent_from_tx_info() {
  local tx_info_file="$1"
  if [[ "${AGENT_START_FROM_CURRENT_TIP:-0}" == "1" ]]; then
    reset_agent_chain_sync_point_to_tip
    export RUN_STATE_DIR="${STATE_DIR}/agent"
    export AGENT_LOG_FILE="${LOG_DIR}/agent.log"
    launch_agent_process
    return
  fi
  local block_hash
  block_hash="$(jq -r '.[0].block_hash' "$tx_info_file")"
  local block
  block="$(block_info "$block_hash")"
  local start_block_hash
  start_block_hash="$(printf '%s' "$block" | jq -r '.[0].parent_hash')"
  local start_block
  start_block="$(block_info "$start_block_hash")"
  export AGENT_CHAIN_SYNC_SLOT
  AGENT_CHAIN_SYNC_SLOT="$(printf '%s' "$start_block" | jq -r '.[0].abs_slot')"
  export AGENT_CHAIN_SYNC_HASH
  AGENT_CHAIN_SYNC_HASH="$(printf '%s' "$start_block" | jq -r '.[0].hash')"
  export RUN_STATE_DIR="${STATE_DIR}/agent"
  export AGENT_LOG_FILE="${LOG_DIR}/agent.log"
  launch_agent_process
}

stop_agent() {
  if [[ -n "${AGENT_PID:-}" ]] && kill -0 "$AGENT_PID" 2>/dev/null; then
    kill "$AGENT_PID" 2>/dev/null || true
    wait "$AGENT_PID" 2>/dev/null || true
  fi
  pkill -f "${RUN_ID}.*bloom-cardano-agent" >/dev/null 2>&1 || true
  pkill -f "target/debug/bloom-cardano-agent" >/dev/null 2>&1 || true
  local lock_file="${STATE_DIR}/agent/chain-sync/LOCK"
  local deadline=$((SECONDS + 30))
  while [[ -e "$lock_file" ]] && (( SECONDS < deadline )); do
    if ! lsof "$lock_file" >/dev/null 2>&1; then
      break
    fi
    sleep 1
  done
  AGENT_PID=""
}

clean_previous_runs() {
  mkdir -p "${RUN_BASE}/runs" "$REPORT_DIR"
  if [[ "${PURGE_AMM_LIMIT_INDEXER_RUNS:-0}" == "1" ]]; then
    find "${RUN_BASE}/runs" -mindepth 1 -maxdepth 1 -type d -exec rm -rf {} +
  fi
}

kill_previous_processes() {
  pkill -f "bash testing/preprod/amm-limit-indexer-flow/run-amm-limit-indexer-flow.sh" >/dev/null 2>&1 || true
  pkill -f "testing/preprod/amm-limit-indexer-flow/.run/runs/.*/agent.log" >/dev/null 2>&1 || true
  pkill -f "testing/preprod/batcher-indexer-flow/.run/runs" >/dev/null 2>&1 || true
  pkill -f "target/debug/bloom-execution-indexer" >/dev/null 2>&1 || true
}

iso_from_unix() {
  python3 -c "from datetime import datetime, timezone; print(datetime.fromtimestamp(${1}, tz=timezone.utc).replace(microsecond=0).isoformat().replace('+00:00','Z'))"
}

main() {
  require_cmd cargo
  require_cmd curl
  require_cmd deno
  require_cmd jq
  require_cmd perl
  require_cmd pkill
  require_cmd python3

  clean_previous_runs
  kill_previous_processes
  mkdir -p "$STATE_DIR" "$LOG_DIR" "$REPORT_DIR" "$GOOD_ORDER_LOG_DIR" "$BAD_ORDER_LOG_DIR"
  cp "${AMM_FLOW_DIR}/env.example" "$FLOW_ENV_FILE"

  local socket_default="${CARDANO_NODE_SOCKET_PATH:-${NODE_SOCKET:-}}"
  NODE_SOCKET="$(prompt "Cardano node socket path" "$socket_default")"
  if [[ -z "$NODE_SOCKET" || ! -S "$NODE_SOCKET" ]]; then
    echo "Node socket is not available at ${NODE_SOCKET}" >&2
    exit 1
  fi
  export NODE_SOCKET

  BLOCKFROST_PROJECT_ID="$(prompt_secret "Preprod Blockfrost project id" "${BLOCKFROST_PROJECT_ID:-}")"
  if [[ -z "$BLOCKFROST_PROJECT_ID" ]]; then
    echo "Blockfrost project id is required." >&2
    exit 1
  fi
  export BLOCKFROST_PROJECT_ID
  BLOCKFROST_KEY_FILE="${STATE_DIR}/blockfrost.key"
  printf '%s\n' "$BLOCKFROST_PROJECT_ID" > "$BLOCKFROST_KEY_FILE"
  chmod 600 "$BLOCKFROST_KEY_FILE"

  local required_wallet_lovelace="${REQUIRED_WALLET_LOVELACE:-500000000}"
  local required_wallet_collateral_lovelace="${REQUIRED_WALLET_COLLATERAL_LOVELACE:-5000000}"
  if [[ "${REUSE_EXISTING_WALLET:-0}" == "1" ]]; then
    required_wallet_collateral_lovelace="${REUSED_WALLET_COLLATERAL_LOVELACE:-0}"
  fi
  local wallet_seed_file="${RUN_ROOT}/wallet.seed"
  if [[ "${REUSE_EXISTING_WALLET:-0}" == "1" && -f "$wallet_seed_file" ]]; then
    chmod 600 "$wallet_seed_file"
  else
    deno run --allow-write "${AMM_FLOW_DIR}/generate-wallet-seed.ts" "$wallet_seed_file" >/dev/null
    chmod 600 "$wallet_seed_file"
  fi
  set_env_value WALLET_SEED_FILE "$wallet_seed_file"
  set_env_value NODE_SOCKET "$NODE_SOCKET"
  set_env_value PROVIDER blockfrost
  set_env_value BLOCKFROST_PROJECT_ID "$BLOCKFROST_PROJECT_ID"
  set_env_value SUBMIT 1
  set_env_value AGENT_DISABLE_MEMPOOL 1

  if ! have_resume_setup; then
    FLOW_ENV_FILE="$FLOW_ENV_FILE" \
    WALLET_SEED_FILE="$wallet_seed_file" \
    NODE_SOCKET="$NODE_SOCKET" \
    RUN_ID="$RUN_ID" \
    SETUP_TOKEN_SUFFIX="${RUN_ID: -12}" \
    BLOCKFROST_PROJECT_ID="$BLOCKFROST_PROJECT_ID" \
    PROVIDER=blockfrost \
    deno run --config "${AMM_FLOW_DIR}/deno.json" --allow-read --allow-write --allow-env --allow-net \
      --allow-ffi \
      "${AMM_FLOW_DIR}/wallet-info.ts" > "$WALLET_INFO_JSON"
  fi

  local wallet_address
  wallet_address="$(jq -r '.wallet' "$WALLET_INFO_JSON")"
  if ! have_resume_setup; then
    echo "wallet address:        $wallet_address"
    echo "wallet minimum:        ${required_wallet_lovelace} lovelace (500 tADA default)"
    if [[ -t 0 ]]; then
      echo "Send at least ${required_wallet_lovelace} lovelace to the wallet above for setup, pool deployment, 11 order publications, and fees, then press Enter."
      read -r
    fi
    wait_wallet_funded "$wallet_address" "$required_wallet_lovelace" "$required_wallet_collateral_lovelace"
    wait_provider_settle

    FLOW_ENV_FILE="$FLOW_ENV_FILE" \
    WALLET_SEED_FILE="$wallet_seed_file" \
    NODE_SOCKET="$NODE_SOCKET" \
    RUN_ID="$RUN_ID" \
    SETUP_TOKEN_SUFFIX="${RUN_ID: -12}" \
    BLOCKFROST_PROJECT_ID="$BLOCKFROST_PROJECT_ID" \
    PROVIDER=blockfrost \
    run_deno_json "$SETUP_JSON" "${AMM_FLOW_DIR}/setup-preprod-flow.ts"
  fi

  local deployment_config
  deployment_config="$(jq -r '.deploymentConfig' "$SETUP_JSON")"
  hydrate_auction_env_from_setup
  set_env_value BLOCKFROST_PROJECT_ID "$BLOCKFROST_PROJECT_ID"
  set_env_value NODE_SOCKET "$NODE_SOCKET"
  set_env_value AGENT_DISABLE_MEMPOOL 1
  set_env_value PROVIDER blockfrost

  FLOW_ENV_FILE="$FLOW_ENV_FILE" BLOCKFROST_PROJECT_ID="$BLOCKFROST_PROJECT_ID" PROVIDER=blockfrost \
    deno run --config "${AMM_FLOW_DIR}/deno.json" --allow-read --allow-env --allow-net --allow-ffi \
    "${AMM_FLOW_DIR}/agent-info.ts" > "$AGENT_INFO_JSON"
  set_env_value OPERATOR_KEY_CBOR_HEX "$(jq -r '.operatorKey' "$AGENT_INFO_JSON")"

  local funding_tx_hash
  funding_tx_hash="$(jq -r '.mintTxHash // .setupTxHash' "$SETUP_JSON")"
  if [[ ! -f "${LOG_DIR}/funding-tx-info.json" ]]; then
    wait_tx_info "$funding_tx_hash" "${LOG_DIR}/funding-tx-info.json"
    wait_provider_settle
  fi

  start_agent_from_tx_info "${LOG_DIR}/funding-tx-info.json"
  wait_for_agent_ready

  export POOL_TOKEN_SUFFIX="${RUN_ID: -12}"
  export POOL_MINT_DEMO_ASSETS=1
  export POOL_X_AMOUNT="${POOL_X_AMOUNT:-1000000}"
  export POOL_Y_AMOUNT="${POOL_Y_AMOUNT:-1000000}"
  export POOL_WALLET_X_AMOUNT=0
  export POOL_WALLET_Y_AMOUNT="${POOL_WALLET_Y_AMOUNT:-26000}"
  export POOL_LOVELACE="${POOL_LOVELACE:-12000000}"
  export POOL_LP_FEE_NUM="${POOL_LP_FEE_NUM:-99700}"
  export POOL_TREASURY_FEE_NUM="${POOL_TREASURY_FEE_NUM:-0}"
  export POOL_ROYALTY_FEE_NUM="${POOL_ROYALTY_FEE_NUM:-0}"
  export POOL_TREASURY_X="${POOL_TREASURY_X:-0}"
  export POOL_TREASURY_Y="${POOL_TREASURY_Y:-0}"
  export POOL_ROYALTY_X="${POOL_ROYALTY_X:-0}"
  export POOL_ROYALTY_Y="${POOL_ROYALTY_Y:-0}"
  export POOL_ROYALTY_NONCE="${POOL_ROYALTY_NONCE:-0}"
  export FLOW_ENV_FILE
  export PROVIDER=blockfrost
  export SUBMIT=1
  export BLOCKFROST_PROJECT_ID

  local amm_pool_tx_hash
  if ! have_resume_pool; then
    run_deno_json "${LOG_DIR}/deploy-royalty-pool.json" "${AMM_FLOW_DIR}/deploy-royalty-pool.ts"
    amm_pool_tx_hash="$(jq -r '.txHash' "${LOG_DIR}/deploy-royalty-pool.json")"
    wait_tx_info "$amm_pool_tx_hash" "${LOG_DIR}/amm-pool-tx-info.json"
    wait_blockfrost_address_utxo "$wallet_address" "$amm_pool_tx_hash" "*"
    wait_provider_settle
  else
    amm_pool_tx_hash="$(jq -r '.txHash' "${LOG_DIR}/deploy-royalty-pool.json")"
  fi

  local limit_input_policy limit_input_name_hex limit_output_policy limit_output_name_hex
  limit_input_policy="$(jq -r '.assetY.policy' "${LOG_DIR}/deploy-royalty-pool.json")"
  limit_input_name_hex="$(jq -r '.assetY.nameHex' "${LOG_DIR}/deploy-royalty-pool.json")"
  limit_output_policy="$(jq -r '.assetX.policy' "${LOG_DIR}/deploy-royalty-pool.json")"
  limit_output_name_hex="$(jq -r '.assetX.nameHex' "${LOG_DIR}/deploy-royalty-pool.json")"
  set_env_value LIMIT_INPUT_POLICY "$limit_input_policy"
  set_env_value LIMIT_INPUT_NAME_HEX "$limit_input_name_hex"
  set_env_value LIMIT_OUTPUT_POLICY "$limit_output_policy"
  set_env_value LIMIT_OUTPUT_NAME_HEX "$limit_output_name_hex"
  set_env_value LIMIT_MINT_DEMO_INPUT 0
  wait_wallet_asset_utxo "$limit_input_policy" "$limit_input_name_hex" 2000 1000000

  local -a good_order_txs=()
  local -a good_order_output_indexes=()
  local -a good_order_exec_txs=()
  local first_good_tx_hash=""
  local min_good_block_time=""
  local i
  for i in $(seq 1 10); do
    local create_file tx_info_file verify_file exec_tx_info_file
    create_file="${GOOD_ORDER_LOG_DIR}/good-$(printf '%02d' "$i")-create.json"
    tx_info_file="${GOOD_ORDER_LOG_DIR}/good-$(printf '%02d' "$i")-tx-info.json"
    verify_file="${GOOD_ORDER_LOG_DIR}/good-$(printf '%02d' "$i")-verify.json"
    exec_tx_info_file="${GOOD_ORDER_LOG_DIR}/good-$(printf '%02d' "$i")-exec-tx-info.json"

    if [[ -f "$create_file" && -f "$verify_file" ]]; then
      local recovered_tx recovered_output recovered_exec
      recovered_tx="$(jq -r '.txHash' "$create_file")"
      recovered_output="$(jq -r '.outputIndex' "$create_file")"
      good_order_txs+=("$recovered_tx")
      good_order_output_indexes+=("$recovered_output")
      if [[ -f "$tx_info_file" ]]; then
        if [[ -z "$first_good_tx_hash" ]]; then
          first_good_tx_hash="$recovered_tx"
        fi
        min_good_block_time="$(
          update_min_unix_time \
            "$min_good_block_time" \
            "$(jq -r '.[0].block_time' "$tx_info_file")"
        )"
      fi
      recovered_exec="$(jq -r '.spendingTx // empty' "$verify_file")"
      if [[ -n "$recovered_exec" && "$recovered_exec" != "null" ]]; then
        good_order_exec_txs+=("$recovered_exec")
        recover_exec_tx_info_from_verify "$verify_file" "$exec_tx_info_file"
        if [[ -f "$exec_tx_info_file" ]]; then
          continue
        fi
      fi
    fi

    export LIMIT_TRADABLE_INPUT=2000
    export LIMIT_MIN_MARGINAL_OUTPUT=1
    export LIMIT_BASE_PRICE_NUM=1
    export LIMIT_BASE_PRICE_DENOM="${LIMIT_TRADABLE_INPUT}"
    export LIMIT_LOVELACE_BUDGET="${LIMIT_LOVELACE_BUDGET_GOOD:-5000000}"
    run_deno_json "$create_file" "${AMM_FLOW_DIR}/create-limit-order.ts"
    local tx_hash output_index
    tx_hash="$(jq -r '.txHash' "$create_file")"
    output_index="$(jq -r '.outputIndex' "$create_file")"
    good_order_txs+=("$tx_hash")
    good_order_output_indexes+=("$output_index")
    wait_tx_info "$tx_hash" "$tx_info_file"
    wait_blockfrost_address_utxo "$wallet_address" "$tx_hash" "*"
    wait_provider_settle
    if [[ -z "$first_good_tx_hash" ]]; then
      first_good_tx_hash="$tx_hash"
    fi
    min_good_block_time="$(
      update_min_unix_time \
        "$min_good_block_time" \
        "$(jq -r '.[0].block_time' "$tx_info_file")"
    )"

    LIMIT_TX_HASH="$tx_hash" \
    LIMIT_OUTPUT_INDEX="$output_index" \
    AGENT_LOG_FILE="${LOG_DIR}/agent.log" \
    FLOW_ENV_FILE="$FLOW_ENV_FILE" \
    VERIFY_TIMEOUT_SECS="${VERIFY_TIMEOUT_SECS:-900}" \
    VERIFY_POLL_SECS="${VERIFY_POLL_SECS:-10}" \
    PROVIDER=blockfrost \
    BLOCKFROST_PROJECT_ID="$BLOCKFROST_PROJECT_ID" \
    deno run --config "${AMM_FLOW_DIR}/deno.json" --allow-read --allow-env --allow-net --allow-ffi \
      "${AMM_FLOW_DIR}/verify-limit-order-flow.ts" \
      > "$verify_file"
    local exec_tx
    exec_tx="$(jq -sr 'map(.spendingTx // empty) | map(select(. != null and . != "")) | last // empty' "$verify_file")"
    if [[ -z "$exec_tx" ]]; then
      echo "Unable to determine spendingTx from ${verify_file}" >&2
      exit 1
    fi
    good_order_exec_txs+=("$exec_tx")
    wait_tx_info "$exec_tx" "$exec_tx_info_file"
    wait_provider_settle

    stop_agent
    start_agent_from_tx_info "$exec_tx_info_file"
    wait_for_agent_ready
    wait_provider_settle
  done

  export LIMIT_TRADABLE_INPUT=2000
  export LIMIT_MIN_MARGINAL_OUTPUT=1900
  export LIMIT_BASE_PRICE_NUM=1
  export LIMIT_BASE_PRICE_DENOM=1
  export LIMIT_LOVELACE_BUDGET="${LIMIT_LOVELACE_BUDGET_BAD:-5000000}"
  run_deno_json "${BAD_ORDER_LOG_DIR}/bad-create.json" "${AMM_FLOW_DIR}/create-limit-order.ts"
  local bad_tx_hash bad_output_index
  bad_tx_hash="$(jq -r '.txHash' "${BAD_ORDER_LOG_DIR}/bad-create.json")"
  bad_output_index="$(jq -r '.outputIndex' "${BAD_ORDER_LOG_DIR}/bad-create.json")"
  wait_tx_info "$bad_tx_hash" "${BAD_ORDER_LOG_DIR}/bad-tx-info.json"
  wait_blockfrost_address_utxo "$wallet_address" "$bad_tx_hash" "*"
  wait_provider_settle

  LIMIT_TX_HASH="$bad_tx_hash" \
  LIMIT_OUTPUT_INDEX="$bad_output_index" \
  FLOW_ENV_FILE="$FLOW_ENV_FILE" \
  VERIFY_TIMEOUT_SECS="${BAD_ORDER_VERIFY_TIMEOUT_SECS:-180}" \
  VERIFY_POLL_SECS="${VERIFY_POLL_SECS:-10}" \
  PROVIDER=blockfrost \
  BLOCKFROST_PROJECT_ID="$BLOCKFROST_PROJECT_ID" \
  deno run --config "${AMM_FLOW_DIR}/deno.json" --allow-read --allow-env --allow-net --allow-ffi \
    "${SCRIPT_DIR}/verify-open-limit-order.ts" \
    > "${BAD_ORDER_LOG_DIR}/bad-verify.json"

  local indexer_from
  indexer_from="$(iso_from_unix "$((min_good_block_time - 60))")"
  env \
    RUN_ID="$INDEXER_RUN_ID" \
    NODE_SOCKET="$NODE_SOCKET" \
    CARDANO_NODE_SOCKET_PATH="$NODE_SOCKET" \
    BLOCKFROST_PROJECT_ID="$BLOCKFROST_PROJECT_ID" \
    BATCHER_INDEXER_FROM="$indexer_from" \
    bash "${BATCHER_FLOW_DIR}/run-batcher-indexer-flow.sh" \
    > "${LOG_DIR}/batcher-indexer.stdout.log"

  local executed_orders eligible_orders batchers_json metrics_json
  executed_orders="$(jq -r '.metrics[0].executedOrders' "$INDEXER_REPORT_FILE")"
  eligible_orders="$(jq -r '.metrics[0].eligibleOrders' "$INDEXER_REPORT_FILE")"
  if [[ "$executed_orders" != "10" || "$eligible_orders" != "10" ]]; then
    echo "Unexpected indexer metrics: eligibleOrders=${eligible_orders}, executedOrders=${executed_orders}" >&2
    cat "$INDEXER_REPORT_FILE" >&2
    exit 1
  fi
  batchers_json="$(jq '.batchers' "$INDEXER_REPORT_FILE")"
  metrics_json="$(jq '.metrics' "$INDEXER_REPORT_FILE")"

  local good_orders_json good_exec_json bad_order_verification_json
  good_orders_json="$(printf '%s\n' "${good_order_txs[@]}" | jq -R . | jq -s .)"
  good_exec_json="$(printf '%s\n' "${good_order_exec_txs[@]}" | jq -R . | jq -s .)"
  bad_order_verification_json="$(jq -s '.' "${BAD_ORDER_LOG_DIR}/bad-verify.json")"

  jq -n \
    --arg status "ok" \
    --arg runId "$RUN_ID" \
    --arg wallet "$wallet_address" \
    --arg fundingTx "$funding_tx_hash" \
    --arg poolTx "$amm_pool_tx_hash" \
    --arg badOrderTx "$bad_tx_hash" \
    --arg badOrderRef "${bad_tx_hash}#${bad_output_index}" \
    --arg firstGoodTx "$first_good_tx_hash" \
    --arg indexerReport "$INDEXER_REPORT_FILE" \
    --arg logs "$LOG_DIR" \
    --argjson goodOrderTxs "$good_orders_json" \
    --argjson goodExecutionTxs "$good_exec_json" \
    --argjson badOrderVerification "$bad_order_verification_json" \
    --argjson batchers "$batchers_json" \
    --argjson metrics "$metrics_json" \
    '{
      status:$status,
      runId:$runId,
      wallet:$wallet,
      fundingTx:$fundingTx,
      poolTx:$poolTx,
      firstGoodOrderTx:$firstGoodTx,
      goodOrderTxs:$goodOrderTxs,
      goodExecutionTxs:$goodExecutionTxs,
      badOrderTx:$badOrderTx,
      badOrderRef:$badOrderRef,
      badOrderVerification:$badOrderVerification,
      indexerReport:$indexerReport,
      batchers:$batchers,
      metrics:$metrics,
      logs:$logs
    }' | tee "$REPORT_FILE"

  echo "AMM limit indexer auditor flow completed. Report: ${REPORT_FILE}"
}

main "$@"
