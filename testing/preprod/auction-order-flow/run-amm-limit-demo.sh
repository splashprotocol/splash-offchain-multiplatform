#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT_DIR"

FLOW_ENV_FILE="${FLOW_ENV_FILE:-testing/preprod/auction-order-flow/.env}"
export FLOW_ENV_FILE

env_file_has() {
  local key="$1"
  [ -f "$FLOW_ENV_FILE" ] && grep -q "^${key}=" "$FLOW_ENV_FILE"
}

if [ -z "${WALLET_SEED_FILE:-}" ] && ! env_file_has "WALLET_SEED_FILE"; then
  read -r -p "Wallet seed file: " WALLET_SEED_FILE
  export WALLET_SEED_FILE
fi

PROVIDER="${PROVIDER:-blockfrost}"
export PROVIDER

if [ "$PROVIDER" = "blockfrost" ] &&
  [ -z "${BLOCKFROST_PROJECT_ID:-}" ] &&
  ! env_file_has "BLOCKFROST_PROJECT_ID"; then
  read -r -s -p "Preprod Blockfrost project id: " BLOCKFROST_PROJECT_ID
  printf "\n"
  export BLOCKFROST_PROJECT_ID
fi

if [ "$PROVIDER" = "maestro" ] &&
  [ -z "${MAESTRO_API_KEY:-}" ] &&
  ! env_file_has "MAESTRO_API_KEY"; then
  read -r -s -p "Preprod Maestro API key: " MAESTRO_API_KEY
  printf "\n"
  export MAESTRO_API_KEY
fi

if [ -z "${SUBMIT:-}" ]; then
  read -r -p "Submit transactions on preprod? Type 1 to submit, Enter for dry-run: " SUBMIT_INPUT
  SUBMIT="${SUBMIT_INPUT:-0}"
  export SUBMIT
fi

printf "Deploying demo AMM pool...\n"
deno run --allow-env --allow-read --allow-net \
  --config testing/preprod/auction-order-flow/deno.json \
  testing/preprod/auction-order-flow/deploy-amm-pool.ts

printf "Creating demo limit order...\n"
deno run --allow-env --allow-read --allow-net \
  --config testing/preprod/auction-order-flow/deno.json \
  testing/preprod/auction-order-flow/create-limit-order.ts

