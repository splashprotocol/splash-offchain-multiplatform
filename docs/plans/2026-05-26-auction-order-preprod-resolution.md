# Auction Order Preprod Resolution Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Stabilize auction order support and the preprod auditor flow so an auditor can run one script, fund a fresh wallet, and verify an auction order is executed by `bloom-cardano-agent`.

**Architecture:** Auction orders are integrated as timed market takers alongside limit orders. The auditor flow creates a fresh wallet, starts the agent with clean local state, funds agent batcher addresses, creates a matching auction ask and limit bid, waits for the agent to execute them, and writes a JSON report with the execution transaction.

**Tech Stack:** Rust workspace, `bloom-cardano-agent`, `bloom-offchain`, `bloom-offchain-cardano`, Deno/Lucid scripts, Blockfrost preprod, Cardano node socket.

---

## Current Evidence

Successful preprod run:

- Run ID: `auditor-20260526-234158`
- Funding tx: `4f86a41c1cdceadf94aa344dd0754e56e17f41782dec0fd2f2965d5f08fda96a`
- Order tx: `949ac2a90cbc834afbdeb1a795a5340f5fb3ba171e21713866787b2c9df99465`
- Auction ref: `949ac2a90cbc834afbdeb1a795a5340f5fb3ba171e21713866787b2c9df99465#1`
- Execution tx: `f9bba6f9e7e135fae2883580c36feb627264dc6858b86e0e96bf5b86a7425e36`
- Verifier status: `ok`

Fresh verification commands already passed:

```bash
bash -n testing/preprod/auction-order-flow/run-auction-flow.sh
deno check --config testing/preprod/auction-order-flow/deno.json \
  testing/preprod/auction-order-flow/create-order-pair.ts \
  testing/preprod/auction-order-flow/setup-preprod-flow.ts \
  testing/preprod/auction-order-flow/verify-auction-flow.ts \
  testing/preprod/auction-order-flow/wallet-info.ts
cargo check -p bloom-cardano-agent
```

## Root Cause

The failed preprod run observed auction and limit orders on ledger, but matchmaking saw only the bid or an empty active frontier. The copied main checkout was missing the auction-critical clock update in `bloom-offchain/src/execution_engine/mod.rs`.

Auction orders depend on ledger time. Without advancing the book clocks from the ledger context before processing pair events, confirmed auction orders can be indexed but not active for matchmaking.

## Task 1: Preserve Engine Fix

**Files:**

- Modify: `bloom-offchain/src/execution_engine/mod.rs`

**Steps:**

1. Keep `LedgerClock` imported from `crate::execution_engine::types`.
2. Keep the `LedgerCx: LedgerClock + Unpin` bound in `execution_part_stream`.
3. Keep the `LCX: LedgerClock` bound in `on_pair_event`.
4. Keep the pre-processing clock advancement:

```rust
let ledger_time = match &event {
    Either::Left(Channel::Ledger(_, cx)) | Either::Right(Channel::Ledger(_, cx)) => cx.posix_time(),
    _ => None,
};
if let Some(time) = ledger_time {
    self.multi_book.get_mut(&pair).advance_clocks(time);
}
```

5. Keep `LCX: LedgerClock + Unpin` bounds on `Executor` stream impls.

**Verify:**

```bash
cargo check -p bloom-cardano-agent
```

Expected: exit code 0. Existing warnings are acceptable.

## Task 2: Preserve Auditor Script Interface

**Files:**

- Modify: `testing/preprod/auction-order-flow/run-auction-flow.sh`
- Modify: `testing/preprod/auction-order-flow/env.example`
- Modify: `testing/preprod/auction-order-flow/README.md`

**Steps:**

1. Keep the public command as:

```bash
./testing/preprod/auction-order-flow/run-auction-flow.sh
```

2. Do not require `--yes`, `--blueprint`, or socket arguments from auditors.
3. Prompt for Blockfrost key when `BLOCKFROST_PROJECT_ID` is missing.
4. Auto-detect `/Users/aleksandr/node-external/node.socket`; prompt only if not found.
5. Print fresh wallet and funding addresses before waiting for tADA.
6. Use existing deployed auction validator reference by default.
7. Keep safe timing defaults:

```bash
AUCTION_STEP_LEN_SECS=600
MIN_SPAN_REMAINING_SECS=120
```

**Verify:**

```bash
bash -n testing/preprod/auction-order-flow/run-auction-flow.sh
```

Expected: exit code 0.

## Task 3: Preserve Deno Flow Checks

**Files:**

- Modify: `testing/preprod/auction-order-flow/setup-preprod-flow.ts`
- Modify: `testing/preprod/auction-order-flow/create-order-pair.ts`
- Modify: `testing/preprod/auction-order-flow/verify-auction-flow.ts`
- Modify: `testing/preprod/auction-order-flow/wallet-info.ts`

**Steps:**

1. `setup-preprod-flow.ts` must support existing validator reference from env.
2. `create-order-pair.ts` must reject auction spans that are too close to expiry.
3. `verify-auction-flow.ts` must check spending transaction lookup before treating the auction UTxO as still unspent.
4. `wallet-info.ts` must work before the full flow env is generated.

**Verify:**

```bash
deno check --config testing/preprod/auction-order-flow/deno.json \
  testing/preprod/auction-order-flow/create-order-pair.ts \
  testing/preprod/auction-order-flow/setup-preprod-flow.ts \
  testing/preprod/auction-order-flow/verify-auction-flow.ts \
  testing/preprod/auction-order-flow/wallet-info.ts
```

Expected: all files type-check.

## Task 4: Run One Clean Preprod Flow

**Files:**

- Runtime state: `testing/preprod/auction-order-flow/.run/`

**Steps:**

1. Start from repo root:

```bash
./testing/preprod/auction-order-flow/run-auction-flow.sh
```

2. Enter Blockfrost preprod key when prompted.
3. Confirm or enter node socket path.
4. Send at least `500 tADA` to the printed fresh wallet address.
5. Wait for the script to fund the agent, create orders, verify execution, and write the report.

**Expected final output:**

```json
{
  "status": "ok",
  "executionTx": "<tx hash>"
}
```

## Task 5: Capture Auditor Evidence

**Files:**

- Runtime report: `testing/preprod/auction-order-flow/.run/reports/<run-id>.json`
- Agent log: `testing/preprod/auction-order-flow/.run/logs/<run-id>/agent.log`
- Docs: `testing/preprod/auction-order-flow/README.md`

**Steps:**

1. Record run ID, wallet, funding tx, auction tx, auction ref, and execution tx.
2. Confirm agent log contains `Successfully formed a batch`.
3. Confirm verifier reports `spent_by_agent_flow`.
4. Confirm final report status is `ok`.
5. Update README with the minimum auditor command and expected prompts if needed.

## Task 6: Commit Resolution

**Files:**

- All auction support source files.
- All `testing/preprod/auction-order-flow/` files.
- This plan.

**Steps:**

1. Review staged changes:

```bash
git status --short
git diff --staged --stat
```

2. Avoid staging unrelated local files.
3. Commit:

```bash
git add bloom-offchain/src/execution_engine/mod.rs
git add testing/preprod/auction-order-flow
git add docs/plans/2026-05-26-auction-order-preprod-resolution.md
git commit -m "test: add preprod auction order auditor flow"
```

4. If auction engine/source integration changes are not already committed in this checkout, include them in a separate commit before the test-flow commit.

## Done Criteria

- `cargo check -p bloom-cardano-agent` passes.
- Deno flow scripts type-check.
- `run-auction-flow.sh` passes shell syntax check.
- One fresh preprod run produces `status: ok`.
- Report contains an execution transaction that spends the auction order output.
- Auditor command is documented as a single script invocation.
