# Ad-Hoc Splash Fee Review Resolution Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Resolve production-blocking review findings for ad-hoc fees on Snek-graduated Splash pools, including fee-state correctness, graduation tracking, config validation, and persisted restart safety.

**Architecture:** Keep ad-hoc fee behavior scoped to Splash limit orders whose pool origin is `SnekGraduated`. Treat fee metadata on `ClassifiedPool` as transient execution metadata: accumulate while previewing a recipe, consume exactly once during batch execution, then reset before the pool re-enters state. Persist graduation lineage separately from live pool state so agent restarts retain already-discovered graduated Splash pools and tracked Snek pool inputs.

**Tech Stack:** Rust workspace, `cargo test`, `cargo check`, RocksDB, existing `GraduatedSplashPoolStore`, `SnekPoolInputTracker`, `ClassifiedPool`, liquidity-book execution types.

---

## Reviewer Loop Contract

Run two independent reviewer agents after each implementation batch:

- Reviewer A: fee execution semantics. Scope: `bloom-offchain-cardano/src/pools/classified.rs`, `bloom-offchain-cardano/src/execution_engine/instances.rs`, fee-related tests.
- Reviewer B: graduation state, persistence, and config semantics. Scope: `bloom-offchain-cardano/src/graduation.rs`, `bloom-offchain-cardano/src/event_sink/handler.rs`, `bloom-cardano-agent/src/config.rs`, persistence tests, config tests.

Loop until both reviewers return no Critical or Important findings:

1. Implement the next batch.
2. Run targeted tests for that batch.
3. Ask both reviewers to review only the diff since the batch base SHA.
4. Fix all Critical and Important findings.
5. Re-run the relevant tests.
6. Re-review the fix diff.
7. Continue only after both reviewers say no blocking findings remain.

Minor findings may be fixed immediately if low risk. Otherwise record them in the final response with rationale.

## Task 1: Fee Accumulation Regression Tests

**Files:**
- Modify: `bloom-offchain-cardano/src/pools/classified.rs`
- Test: existing `#[cfg(test)]` module or new focused test module in `bloom-offchain-cardano/src/pools/classified.rs`

**Step 1: Write failing tests**

Add tests that prove:

- A fee-bearing `ClassifiedPool` with existing `pending_operator_fee` adds the next `operator_fee` instead of replacing it.
- If `operator_fee >= gross_input` for ADA-input trades, the trade is rejected as no-trade instead of falling back to fee-free default execution.
- If `operator_fee >= gross_output` for ADA-output trades, the trade is rejected as no-trade instead of producing zero taker output or falling back to fee-free default execution.

Use minimal pool/taker fixtures already present in the module or nearby liquidity-book tests. If no suitable fixtures exist, add a small helper that constructs a `ClassifiedPool` around an existing `AnyPool` test fixture.

**Step 2: Verify tests fail**

Run:

```bash
cargo test -p bloom-offchain-cardano classified_pool_fee -- --nocapture
```

Expected: the new accumulation/no-trade tests fail against current code.

**Step 3: Implement fee accumulation**

In `ClassifiedPool`, replace `with_inner(inner, pending_operator_fee)` usage for fee-bearing swaps with a helper that:

- Adds `operator_fee` to `self.pending_operator_fee`.
- Uses checked arithmetic.
- Returns no successful swap if the fee cannot be accumulated safely.

Do not change direct Splash pools or non-fee-eligible takers.

**Step 4: Verify**

Run:

```bash
cargo test -p bloom-offchain-cardano classified_pool_fee -- --nocapture
```

Expected: all classified fee tests pass.

## Task 2: Consume Pending Fee Exactly Once

**Files:**
- Modify: `bloom-offchain-cardano/src/execution_engine/instances.rs`
- Test: execution-engine test covering `BatchExec` for `ClassifiedPool`

**Step 1: Write failing test**

Add a test that executes a `ClassifiedPool` transition with non-zero `pending_operator_fee` and asserts:

- `ExecutionState::operator_interest` increases by exactly that amount.
- The updated pool emitted by the effect has `pending_operator_fee == 0`.
- Re-executing a transition built from that emitted updated pool does not add the already-consumed fee a second time.

**Step 2: Verify test fails**

Run:

```bash
cargo test -p bloom-offchain-cardano classified_pool_fee -- --nocapture
```

Expected: test fails because the updated pool still carries the pending fee.

**Step 3: Implement reset-after-consume**

In `BatchExec for Magnet<Make<ClassifiedPool, FinalizedTxOut>>`:

- Read `pending_operator_fee` from `next_pool`.
- Add it to `operator_interest`.
- Return an updated `ClassifiedPool` with identical `inner`, `origin`, and `fee_config`, but `pending_operator_fee: 0`.

Do not reset the consumed/original pool in the failure path.

**Step 4: Verify**

Run:

```bash
cargo test -p bloom-offchain-cardano classified_pool_fee -- --nocapture
```

Expected: all classified/execution fee tests pass.

## Task 3: Graduation Tracker Journaling

**Files:**
- Modify: `bloom-offchain-cardano/src/event_sink/handler.rs`
- Modify if needed: `bloom-offchain-cardano/src/graduation.rs`
- Test: `bloom-offchain-cardano/src/graduation.rs` and/or handler tests

**Step 1: Write failing tests**

Add tests proving:

- A transaction that consumes a tracked Snek pool input and produces a new Snek pool output, but no Splash pool, still updates the tracker.
- Rolling back that transaction restores the consumed Snek ref and removes the produced Snek ref.
- A transaction that consumes a tracked Snek pool input and produces a graduated Splash pool still marks that Splash pool as graduated.
- Produced Snek refs are not inserted directly outside the journal path unless the same journal path can also roll them back.

**Step 2: Verify tests fail**

Run:

```bash
cargo test -p bloom-offchain-cardano graduation -- --nocapture
```

Expected: Snek-only update/rollback test fails against current handler logic or store integration.

**Step 3: Implement unconditional Snek tracker journal**

In `event_sink/handler.rs`:

- Compute consumed and produced Snek refs as today.
- Call `journal_graduated_splash_pools` when either `consumed_snek_refs` or `produced_snek_refs` is non-empty.
- Allow `graduated_splash_ids` to be empty for normal Snek pool updates.
- Keep Splash graduated ID insertion conditional on actual produced Splash pool IDs.
- Ensure produced Snek refs and consumed Snek ref removals are applied through the same journaled operation, so ledger rollback and dropped mempool transactions can reverse them.

**Step 4: Verify**

Run:

```bash
cargo test -p bloom-offchain-cardano graduation -- --nocapture
```

Expected: graduation tracker tests pass.

## Task 4: Persist Graduation State Across Restarts

**Files:**
- Modify: `bloom-offchain-cardano/src/graduation.rs`
- Modify: `bloom-cardano-agent/src/main.rs`
- Modify if needed: `bloom-cardano-agent/src/config.rs`
- Test: `bloom-offchain-cardano/src/graduation.rs`

**Step 1: Write failing persistence tests**

Add tests for:

- `GraduationStateRocksDb::persist_observation` writes graduated Splash pool IDs and tracked Snek pool input refs.
- `GraduationStateRocksDb::load_state` reconstructs both sets.
- Opening an empty RocksDB state directory returns an empty store/tracker state.
- `GraduationStateRocksDb::persist_rollback` applies the inverse delta for rollback.
- Mempool/speculative graduation observations are not persisted as confirmed durable state.
- Confirmed ledger graduation observations are persisted and restored after restart.

**Step 2: Verify tests fail**

Run:

```bash
cargo test -p bloom-offchain-cardano graduation -- --nocapture
```

Expected: persistence tests fail before the RocksDB implementation exists.

**Step 3: Implement persistence**

Use RocksDB, not JSON. Keep the live store/tracker in memory and persist confirmed ledger deltas into a dedicated RocksDB database:

- `graduated:<policy.asset>` -> empty value for graduated Splash pool IDs.
- `snek-input:<tx_hash>#<index>` -> `<policy.asset>` for tracked Snek pool input refs.
- `journal:<tx_hash>` -> compact line-encoded observation delta for rollback after restart.

Implementation requirements:

- Persist only confirmed ledger state. Do not write speculative mempool observations.
- Persist observations and rollbacks as RocksDB transactions so the store, tracker, and rollback journal keys move together.
- Persist the rollback journal in RocksDB; otherwise a restart before rollback can leave graduated flags stuck.
- Add helper methods to snapshot `GraduatedSplashPoolStore` and `SnekPoolInputTracker`.
- Load store, tracker, and rollback journal together so restart restores both lineage sets and can still undo rolled-back blocks.
- Add an explicit Bloom agent config field for the graduation state path. Suggested shape:

```json
{
  "graduationStateDbPath": "bloom-cardano-agent/state/graduation-state.rocksdb"
}
```

- Wire agent startup to open RocksDB from `graduationStateDbPath`.
- Wire confirmed ledger apply/rollback to persist the corresponding RocksDB delta after successful graduation tracker mutation.
- Require `graduationStateDbPath` when graduated fees are enabled.

**Step 4: Verify**

Run:

```bash
cargo test -p bloom-offchain-cardano graduation -- --nocapture
cargo check -p bloom-cardano-agent
```

Expected: tests pass and Bloom agent compiles.

## Task 5: Graduated Fee Config Validation And Safe Math

**Files:**
- Modify: `bloom-offchain-cardano/src/graduation.rs`
- Modify: `bloom-cardano-agent/src/config.rs`
- Test: relevant config test modules

**Step 1: Write failing tests**

Add tests proving:

- `relativeFeePercent >= 100` is rejected, not saturated, because a 100% fee cannot leave positive net ADA for execution.
- Large `amount` fee calculation near `u64::MAX` does not overflow.
- Default `graduatedPoolFee` is disabled unless explicitly enabled.
- Integrity validation fails when graduated fees are enabled with `relativeFeePercent >= 100`.
- Integrity validation fails when graduated fees are enabled but required Snek graduation script hashes or persistence path are missing.

**Step 2: Verify tests fail**

Run:

```bash
cargo test -p bloom-offchain-cardano graduation -- --nocapture
cargo test -p bloom-cardano-agent config -- --nocapture
```

Expected: validation/default tests fail against current code.

**Step 3: Implement validation**

- Replace silent saturation at config boundary with explicit validation.
- Use `u128` multiplication in `GraduatedPoolFeeConfig::fee`.
- Change `GraduatedPoolFeeAppConfig::default()` and serde default to `enabled: false`.
- Keep explicit production config responsible for enabling the feature.
- Keep direct Splash pools unaffected when graduated fees are disabled or when pool origin is `Direct`.

**Step 4: Verify**

Run:

```bash
cargo test -p bloom-offchain-cardano graduation -- --nocapture
cargo test -p bloom-cardano-agent config -- --nocapture
cargo check -p bloom-cardano-agent
```

Expected: tests pass and Bloom agent compiles.

## Task 6: Shared Minimum Lovelace Constant

**Files:**
- Modify: `spectrum-offchain-cardano/src/data/pool.rs`

**Step 1: Replace magic number**

Import and use `MIN_SAFE_LOVELACE_VALUE` instead of hard-coded `1_000_000`.

**Step 2: Verify**

Run:

```bash
cargo check -p spectrum-offchain-cardano
```

Expected: package compiles.

## Final Verification Gate

Run the full targeted suite:

```bash
cargo test -p bloom-offchain-cardano graduation -- --nocapture
cargo test -p bloom-offchain-cardano classified_pool_fee -- --nocapture
cargo test -p bloom-offchain liquidity_book -- --nocapture
cargo test -p bloom-cardano-agent config -- --nocapture
cargo check -p bloom-cardano-agent
cargo check -p spectrum-offchain-cardano
```

Then run both reviewer agents against the final diff from the original branch head to `HEAD`.

Completion criteria:

- All commands exit 0.
- Reviewer A reports no Critical or Important findings.
- Reviewer B reports no Critical or Important findings.
- Final diff contains only PR-related changes.
