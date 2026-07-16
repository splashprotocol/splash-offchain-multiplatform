# Snek Decimal Fees Implementation Plan

**Goal:** Allow `snek-cardano-agent` to collect `1.3% + 0.5 ADA/trade` while keeping creator accounting at `0.3%`.

**Architecture:** Store ad-hoc relative fees internally as basis points. Keep legacy integer `relativeFeePercent` compatible, and add a precise `relativeFeeBps` config path for `130`.

**Tech Stack:** Rust, serde config parsing, bounded-integer, Cargo tests.

---

### Task 1: Add BPS Fee Model Tests

**Files:**
- Modify: `bloom-offchain-cardano/src/orders/adhoc.rs`
- Modify: `snek-cardano-agent/src/config.rs`

**Steps:**
1. Add unit tests that prove `AdhocFeeStructure { relative_fee_bps: 130 }` charges `1.3%`.
2. Add config tests for `relativeFeeBps: 130`.
3. Add config tests proving legacy `relativeFeePercent: 1` maps to `100 bps`.

### Task 2: Implement BPS Support

**Files:**
- Modify: `bloom-offchain-cardano/src/orders/adhoc.rs`
- Modify: `snek-cardano-agent/src/config.rs`
- Modify: `snek-cardano-agent/resources/preprod.config.json`
- Modify: `snek-cardano-agent/resources/mainnet.config.json.template`

**Steps:**
1. Replace integer percent storage in `AdhocFeeStructure` with `relative_fee_bps`.
2. Calculate fees as `body * relative_fee_bps / 10_000`.
3. Parse `relativeFeeBps` directly when present.
4. Preserve `relativeFeePercent` as a legacy fallback.
5. Set Snek configs to `relativeFeeBps: 130`.
6. Leave `validation-rules.json` `minFeeLovelace: 500000` unchanged.

### Task 3: Verify and Review

**Commands:**
- `cargo fmt --all`
- `cargo test -p bloom-offchain-cardano orders::adhoc -- --nocapture`
- `cargo test -p snek-cardano-agent -- --nocapture`
- `cargo check -p snek-cardano-agent`

**Review loop:**
1. Run reviewer subagent after implementation and verification.
2. Fix Critical/Important feedback.
3. Rerun the same verification commands.
4. Commit and push branch.
