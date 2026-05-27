#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"
FLOW_DIR="$ROOT/testing/preprod/auction-order-flow"
ENV_FILE="${FLOW_ENV_FILE:-$FLOW_DIR/.env}"
RUN_DIR="$FLOW_DIR/.run"

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

if [[ "${SUBMIT:-0}" != "1" ]]; then
  echo "SUBMIT=1 is required before starting bloom-cardano-agent on ${NETWORK:-preprod}." >&2
  exit 1
fi

mkdir -p "$RUN_DIR/logs"
export AGENT_LOG_FILE="${AGENT_LOG_FILE:-$RUN_DIR/logs/agent.$(date +%Y%m%d-%H%M%S).log}"
export RUN_ID="${RUN_ID:-$(date +%Y%m%d-%H%M%S)}"
if [[ ! "$RUN_ID" =~ ^[A-Za-z0-9._-]+$ ]]; then
  echo "RUN_ID may contain only letters, numbers, dot, underscore, and dash." >&2
  exit 1
fi
CONFIG_PATH="$("$FLOW_DIR/generate-agent-config.sh")"
LOG4RS_RUNTIME_CONFIG="$RUN_DIR/log4rs.$RUN_ID.yaml"

cat > "$LOG4RS_RUNTIME_CONFIG" <<YAML
refresh_rate: 30 seconds
appenders:
  stdout:
    kind: console
    encoder:
      pattern: "{d(%Y-%m-%d %H:%M:%S)} {l:5.5} {t} {m}{n}"
root:
  level: trace
  appenders:
    - stdout
loggers:
  chain_sync:
    level: trace
  agent_main:
    level: trace
YAML

if [[ "${BUILD_AGENT:-1}" == "1" ]]; then
  cargo build --package bloom-cardano-agent
fi

LOG_PIPE="$RUN_DIR/logs/agent.$RUN_ID.pipe"
rm -f "$LOG_PIPE"
mkfifo "$LOG_PIPE"
AGENT_CHILD_PID=""
TEE_CHILD_PID=""
cleanup() {
  local exit_code=$?
  if [[ -n "${AGENT_CHILD_PID:-}" ]]; then
    kill "$AGENT_CHILD_PID" 2>/dev/null || true
    wait "$AGENT_CHILD_PID" 2>/dev/null || true
  fi
  if [[ -n "${TEE_CHILD_PID:-}" ]]; then
    kill "$TEE_CHILD_PID" 2>/dev/null || true
    wait "$TEE_CHILD_PID" 2>/dev/null || true
  fi
  rm -f "$LOG_PIPE"
  exit "$exit_code"
}
trap cleanup EXIT INT TERM

tee "$AGENT_LOG_FILE" < "$LOG_PIPE" &
TEE_CHILD_PID="$!"

"${AGENT_BIN:-target/debug/bloom-cardano-agent}" \
  --config-path "$CONFIG_PATH" \
  --deployment-path "$ROOT/${DEPLOYMENT_CONFIG:-bloom-cardano-agent/resources/preprod.deployment.json}" \
  --validation-rules-path "$ROOT/${VALIDATION_RULES:-bloom-cardano-agent/resources/validation-rules.json.template}" \
  --log4rs-path "${LOG4RS_CONFIG:-$LOG4RS_RUNTIME_CONFIG}" \
  > "$LOG_PIPE" 2>&1 &
AGENT_CHILD_PID="$!"

wait "$AGENT_CHILD_PID"
