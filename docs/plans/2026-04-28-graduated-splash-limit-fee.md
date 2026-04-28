# Graduated Splash Limit Fee Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Charge a 1% operator fee on Splash limit-order swaps only when the matched Splash pool was graduated from a Snek.fun quadratic pool.

**Architecture:** Add a persisted "graduated Splash pool" classification derived from transactions that consume a Snek.fun `QuadraticPool` input and produce a Splash `AnyPool` output. Represent Splash makers in the Bloom book as `ClassifiedPool { inner: AnyPool, origin, pending_operator_fee }`. For trades where `origin == SnekGraduated` and the taker is `AnyOrder::Limit`, apply a per-trade ADA-side fee transform inside maker/taker matchmaking: the order spends the gross amount, the pool receives or pays the net amount, and the difference is accumulated as operator interest.

**Tech Stack:** Rust, Cardano ledger parsing, `bloom-offchain-cardano`, `bloom-cardano-agent`, `spectrum-offchain-cardano`, liquidity-book `MarketTaker`/`BatchExec`, cargo tests.

---

## Current Integration Assessment

The Snek.fun behavior already exists in `bloom-offchain-cardano/src/orders/adhoc.rs`. `AdhocOrder` wraps `InstantOrder`, exposes a reduced virtual `input_amount` to matchmaking, and records the hidden fee input. Execution then spends the real order UTxO in `bloom-offchain-cardano/src/execution_engine/instances.rs`, subtracts `removed_input` for the swap, subtracts normal budget/fee, subtracts the proportional ad-hoc fee, and adds that fee to `state.operator_interest`.

Splash limit orders are parsed in `bloom-offchain-cardano/src/orders/limit.rs` and executed in `bloom-offchain-cardano/src/execution_engine/instances.rs`. The same virtual-input pattern is possible, but not by modifying `LimitOrder` alone: whether the 1% fee applies depends on the pool matched against the order, not on the order UTxO. Current `AnyOrder` does not carry pool context.

Graduation is not currently visible as a durable property of `AnyPool`. Bloom's `EvolvingCardanoEntity` indexes `AnyOrder` and `AnyPool`; Snek's agent indexes `AdhocOrder` and `QuadraticPool` separately. A graduation detector therefore needs transaction-level evidence and persistence, otherwise the fee can only be detected in the graduation transaction itself and would be lost for future swaps.

Recommended approach: persist a set of graduated Splash pool IDs, update it from chain events, classify pool entities before inserting them into the Bloom book, and apply the 1% fee in the order-pool trade function. Do not pre-wrap `LimitOrder` with a reduced `input_amount`: limit continuation datums only store one `tradable_input` field, so storing a virtual remainder would make the hidden future fee look like execution budget on the next parse.

## New Entities And APIs

Add these concrete entities:

```rust
// bloom-offchain-cardano/src/graduation.rs
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PoolOrigin {
    Direct,
    SnekGraduated,
}

#[derive(Copy, Clone, Debug)]
pub struct GraduatedPoolFeeConfig {
    pub enabled: bool,
    pub relative_fee_percent: BoundedU64<0, 100>,
}

#[derive(Clone, Debug, Default)]
pub struct GraduatedSplashPoolStore {
    inner: Arc<RwLock<HashSet<Token>>>,
    journal: Arc<RwLock<HashMap<TransactionHash, GraduationJournalEntry>>>,
}

impl GraduatedSplashPoolStore {
    pub fn contains(&self, pool_id: Token) -> bool;
    pub fn insert(&self, pool_id: Token);
    pub fn remove(&self, pool_id: Token);
    pub fn extend<I: IntoIterator<Item = Token>>(&self, ids: I);
    pub fn journal_applied(&self, tx_hash: TransactionHash, entry: GraduationJournalEntry);
    pub fn rollback_tx(&self, tx_hash: TransactionHash, tracker: &SnekPoolInputTracker);
    pub fn load_from_disk(path: &Path) -> anyhow::Result<Self>;
    pub fn flush_to_disk(&self, path: &Path) -> anyhow::Result<()>;
    pub fn load_snapshot(path: &Path) -> anyhow::Result<GraduationStateSnapshot>;
    pub fn flush_snapshot(&self, tracker: &SnekPoolInputTracker, tip: Option<Point>, path: &Path) -> anyhow::Result<()>;
}

#[derive(Copy, Clone, Debug)]
pub struct TradeFee {
    pub operator_lovelace: u64,
    pub gross_input: OnSide<u64>,
    pub maker_input: OnSide<u64>,
    pub gross_output: u64,
    pub taker_output: u64,
}
```

```rust
// bloom-offchain-cardano/src/pools/classified.rs
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct ClassifiedPool {
    pub inner: AnyPool,
    pub origin: PoolOrigin,
    pub fee_config: GraduatedPoolFeeConfig,
    pub pending_operator_fee: u64,
}
```

`ClassifiedPool` implements `Stable`, `Tradable`, `MarketMaker`, `MakerBehavior`, and `RequiresValidator` by delegating to `inner`. Its `stable_id()` must be exactly `inner.stable_id()` so existing entity versioning and pool identity do not change. `fee_config` is copied from context when the pool is parsed. `pending_operator_fee` is transient execution state: it starts at zero when parsed from ledger, increases only during a fee-adjusted trade, and is consumed by `BatchExec<Make<ClassifiedPool, FinalizedTxOut>>` via `state.add_operator_interest(...)`.

Add a lightweight graduation tracker:

```rust
// bloom-offchain-cardano/src/graduation.rs
#[derive(Clone, Debug, Default)]
pub struct SnekPoolInputTracker {
    inner: Arc<RwLock<HashMap<OutputRef, Token>>>,
}

#[derive(Clone, Debug)]
pub struct GraduationStateSnapshot {
    pub graduated_splash_ids: HashSet<Token>,
    pub live_snek_refs: HashMap<OutputRef, Token>,
    pub rollback_journal: HashMap<TransactionHash, GraduationJournalEntry>,
    pub chain_tip: Option<Point>,
}

#[derive(Clone, Debug)]
pub struct GraduationJournalEntry {
    pub consumed_snek_refs: Vec<(OutputRef, Token)>,
    pub produced_snek_refs: Vec<(OutputRef, Token)>,
    pub graduated_splash_ids: Vec<Token>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SnekQuadraticPoolIdentity {
    pub pool_id: Token,
}
```

`TxViewMut` contains input references and produced outputs, but not the resolved consumed output bodies. Therefore a graduation transaction cannot be detected by re-parsing consumed outputs from `TxViewMut` alone. The tracker must observe Snek quadratic pool outputs when they are created or updated, store them by `OutputRef`, and later mark a transaction as a graduation when one of its input refs appears in the tracker and one of its outputs parses as a Splash `AnyPool`.

Do not use `QuadraticPool::try_from_ledger` for tracking. It filters by executable operator credential and reserve bounds, which is correct for Snek execution but wrong for identity detection. Add `SnekQuadraticPoolIdentity::try_from_ledger_identity(...)` that checks only:
- the output address matches `DegenQuadraticPoolV1` or `DegenQuadraticPoolV1T2T`;
- the datum can yield the `pool_nft`;
- the pool NFT converts to `PoolId`/`Token`.

It must not require `OperatorCred`, `PoolValidation`, active status, cap status, metadata, or mints.

Add one default method to the generic taker behavior API:

```rust
// bloom-offchain/src/execution_engine/liquidity_book/market_taker.rs
pub trait TakerBehaviour {
    // existing methods...

fn graduated_splash_fee_eligible(&self) -> bool {
    false
}
}
```

Override it in `AnyOrder`:

```rust
impl TakerBehaviour for AnyOrder {
    // existing methods...

    fn graduated_splash_fee_eligible(&self) -> bool {
        matches!(self, AnyOrder::Limit(_))
    }
}
```

Add one maker-side hook with a default implementation equal to today:

```rust
// bloom-offchain/src/execution_engine/liquidity_book/market_maker.rs
pub fn default_swap_with_taker<Taker, Maker>(
    target_taker: Taker,
    maker: Maker,
    input: OnSide<u64>,
) -> (TakeInProgress<Taker>, MakeInProgress<Maker>)
where
    Taker: MarketTaker + TakerBehaviour + Copy,
    Maker: MarketMaker + MakerBehavior + Copy,
{
    // existing execute_with_maker logic
}

fn preserve_preview_metadata(self, previewed: Self, rebalanced: Self) -> Self
where
    Self: Sized,
{
    rebalanced
}

fn swap_with_taker<Taker>(
    self,
    target_taker: Taker,
    input: OnSide<u64>,
) -> (TakeInProgress<Taker>, MakeInProgress<Self>)
where
    Taker: MarketTaker + TakerBehaviour + Copy,
    Self: MarketMaker + MakerBehavior + Copy,
{
    default_swap_with_taker(target_taker, self, input)
}
```

`execute_with_maker(...)` should become a small call to `maker.swap_with_taker(target_taker, chunk_size)`. `ClassifiedPool` overrides this hook and calls `default_swap_with_taker(...)` for all non-fee cases, because Rust cannot directly invoke a trait method's default body from an override. When `origin == SnekGraduated`, fee config is enabled, and `target_taker.graduated_splash_fee_eligible()`, it applies the ADA-side transform:

- If the pool input asset is ADA: `maker_input = gross_input - fee(gross_input)`; the taker still removes `gross_input`; the pool swap uses `maker_input`; `pending_operator_fee += gross_input - maker_input`.
- If the pool output asset is ADA: perform the pool swap with the gross input, compute `operator_fee = fee(gross_output)`, return `taker_output = gross_output - operator_fee`, and accumulate the fee.
- If neither side is ADA: no fee until business defines a token-denominated fee.

Use `PairId::assets()` and `Side` to map `OnSide` to base/quote assets. Avoid pair-level classification; the exact `ClassifiedPool::stable_id()` controls whether the fee applies.

`MakeInProgress::finalized()` currently replays `target.swap(...)` from the original maker to rebalance excess. That can drop transient fee metadata from the previewed `next` maker. Update finalization to call `target.preserve_preview_metadata(previewed_next, rebalanced)` before returning the final maker. `ClassifiedPool` must copy the fee accumulated on `previewed_next` into the rebalanced result.

Add fee-aware preselection before an eligible limit trade can be accepted. `TLBState::preselect_market_maker(...)`, `dummy_swap(...)`, and `try_optimized_swap(...)` currently see only price, demand, side, and maker. Change these functions to also receive `&target_taker`, then use a maker hook such as:

```rust
fn effective_trade_preview<Taker>(
    &self,
    taker: &Taker,
    gross_input: OnSide<u64>,
) -> Option<FillPreview>
where
    Taker: MarketTaker + TakerBehaviour + Copy;
```

The default implementation returns today's `real_price`/input preview. `ClassifiedPool` returns a preview priced from gross taker input to net taker output after the 1% fee. Matching must reject the maker when this effective net price does not overlap `target_taker.price()`. Add explicit tests where the gross quote satisfies a limit but the post-fee net quote fails.

## Data Flow

1. Graduation tracker scans transaction outputs and records every Snek `QuadraticPool` UTxO by `OutputRef`.
2. For each transaction, the tracker checks whether any input ref belongs to a known Snek pool.
3. If a Snek pool input is present, the tracker parses the transaction outputs as Splash `AnyPool`.
4. Each produced Splash pool ID from that transaction is inserted into `GraduatedSplashPoolStore`.
5. The tracker removes consumed Snek pool refs from its own live map after processing the transaction.
6. When a Splash pool UTxO is parsed for Bloom execution, wrap it as `ClassifiedPool` with `origin = SnekGraduated` if its ID is in the store, otherwise `Direct`.
7. The Bloom book becomes `TLB<AnyOrder, ClassifiedPool, PairId, ExUnits>`.
8. Normal direct pools delegate to current `AnyPool` math unchanged.
9. Graduated pools charge only when the taker is a limit order and the trade has ADA on one side.
10. `BatchExec` for the classified pool adds the transition's `pending_operator_fee` to operator interest, writes the resulting inner pool output normally, and resets transient fee state before the predicted pool is synced back into the book.

## Non-Goals

- Do not charge directly created Splash pools.
- Do not change on-chain validators or datum formats.
- Do not apply the 1% fee to Splash deposit/redeem/DAO/royalty orders.
- Do not apply the fee to order-order matching unless the business explicitly defines which pool should justify the fee.

## Open Business Decisions

Before implementation, confirm these two details:

- Fee asset: match Snek behavior and charge only from ADA-side value. For ADA-to-token swaps, reduce virtual ADA input. For token-to-ADA swaps, charge 1% from produced ADA output during execution. For token-to-token pools, fee is zero unless business wants token-denominated fees.
- Order-order matching: recommended default is no graduated-pool fee because no pool is involved. If order-order fills should be disabled for pairs with graduated pools, that is a separate policy change.

---

### Task 1: Add Regression Tests For The Existing Snek Ad-Hoc Pattern

**Files:**
- Modify: `bloom-offchain-cardano/src/orders/adhoc.rs`
- Modify: `bloom-offchain-cardano/src/execution_engine/instances.rs`

**Step 1: Write tests documenting current behavior**

Add or extend unit tests proving:
- `AdhocFeeStructure { relative_fee_percent: 1 }` maps `100_000_000` lovelace body to `1_000_000`.
- ADA-to-token `AdhocOrder::try_from_ledger` exposes `99_000_000` virtual input for a `100_000_000` lovelace instant order.
- token-to-ADA keeps virtual token input unchanged and computes the 1% fee from `added_output` during execution.

**Step 2: Run tests**

```bash
cargo test -p bloom-offchain-cardano adhoc -- --nocapture
```

Expected: all current ad-hoc tests pass and new tests pass. These tests are a safety net before reusing the model for limits.

**Step 3: Commit**

```bash
git add bloom-offchain-cardano/src/orders/adhoc.rs bloom-offchain-cardano/src/execution_engine/instances.rs
git commit -m "test: document ad-hoc fee behavior"
```

### Task 2: Introduce A Graduated Splash Pool Classification Type

**Files:**
- Create: `bloom-offchain-cardano/src/graduation.rs`
- Modify: `bloom-offchain-cardano/src/lib.rs`
- Modify: `bloom-offchain-cardano/src/event_sink/context.rs`

**Step 1: Define the data model**

Create small runtime types:

```rust
#[derive(Copy, Clone, Debug, Default)]
pub struct GraduatedPoolFeeConfig {
    pub enabled: bool,
    pub relative_fee_percent: BoundedU64<0, 100>,
}

#[derive(Clone, Debug, Default)]
pub struct GraduatedSplashPoolStore {
    inner: Arc<RwLock<HashSet<Token>>>,
    journal: Arc<RwLock<HashMap<TransactionHash, GraduationJournalEntry>>>,
}

impl GraduatedSplashPoolStore {
    pub fn contains(&self, pool_id: Token) -> bool {
        self.inner.read().unwrap().contains(&pool_id)
    }

    pub fn insert(&self, pool_id: Token) {
        self.inner.write().unwrap().insert(pool_id);
    }

    pub fn remove(&self, pool_id: Token) {
        self.inner.write().unwrap().remove(&pool_id);
    }
}
```

Use `Token` because `AnyPool::stable_id()` returns `Token`.

**Step 2: Expose through context**

Add `graduated_pool_fee_config` and `graduated_pool_store` to `HandlerContextProto`/`HandlerContext<I>`, then implement `Has<GraduatedPoolFeeConfig>` and `Has<GraduatedSplashPoolStore>`.

**Step 3: Run compile check**

```bash
cargo check -p bloom-offchain-cardano
```

Expected: new context fields require caller updates, so this may fail until Task 3. Keep the failure as the handoff point if working task-by-task.

**Step 4: Commit**

```bash
git add bloom-offchain-cardano/src/graduation.rs bloom-offchain-cardano/src/lib.rs bloom-offchain-cardano/src/event_sink/context.rs
git commit -m "feat: add graduated pool fee context"
```

### Task 3: Add Bloom Agent Configuration And Runtime State

**Files:**
- Modify: `bloom-cardano-agent/src/config.rs`
- Modify: `bloom-cardano-agent/src/main.rs`
- Modify: `bloom-cardano-agent/src/context.rs`
- Modify: Bloom app config templates for preprod/mainnet
- Modify: Bloom deployment loading for Snek validator hashes

**Step 1: Add config**

Add an optional Bloom config section:

```rust
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraduatedPoolFeeConfigSection {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_graduated_fee_percent")]
    pub relative_fee_percent: BoundedU64<0, 100>,
    #[serde(default)]
    pub known_graduated_pool_ids: Vec<Token>,
    #[serde(default)]
    pub graduation_backfill_from_point: Option<Point>,
    #[serde(default)]
    pub graduated_pool_store_db_path: Option<PathBuf>,
    #[serde(default)]
    pub snek_deployment_file: Option<PathBuf>,
}
```

`AppConfig` should contain `graduated_pool_fee: Option<GraduatedPoolFeeConfigSection>` with `None` equivalent to disabled. Paths are required only when detection/backfill is enabled. Production config should set `enabled: true` and `relativeFeePercent: 1` only after bootstrap is complete.

**Step 2: Thread state into contexts**

Create one shared `GraduatedSplashPoolStore` and one shared `SnekPoolInputTracker` in `bloom-cardano-agent/src/main.rs` and pass them into all handler/execution contexts that need to parse entities or execute recipes.

**Step 3: Load Snek deployment**

Bloom deployment currently does not include Degen quadratic validators. Parse the Snek deployment file using the existing Snek deployment schema (`SnekDeployedValidators`/`SnekProtocolScriptHashes`) or move the shared schema into a reusable crate. Thread the Snek script hashes into the graduation tracker context.

Add a concrete context:

```rust
pub struct GraduationTrackerContext {
    pub snek_scripts: SnekProtocolScriptHashes,
    pub splash_scripts: ProtocolScriptHashes,
    pub pool_validation: PoolValidation,
}
```

Implement:
- `Has<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>>`
- `Has<DeployedScriptInfo<{ DegenQuadraticPoolV1T2T as u8 }>>`
- all `Has<DeployedScriptInfo<...>>` required by `AnyPool::try_from_ledger`
- `Has<PoolValidation>`

**Step 4: Make bootstrap mandatory**

On startup:
- load a complete `GraduationStateSnapshot` from `graduated_pool_store_db_path` if it exists; the snapshot must include graduated Splash IDs, live Snek refs, rollback journal, and chain tip;
- merge `known_graduated_pool_ids`;
- if `graduation_backfill_from_point` is set, block startup until replay/backfill has populated both the graduated Splash ID store and live Snek ref tracker up to the current chain tip;
- if fee config is enabled and there is no current persisted tracker snapshot and no backfill-to-tip, fail fast; `knownGraduatedPoolIds` alone is enough to classify already-known Splash pools but is not enough for automatic future graduation detection;
- log a deterministic count and hash of graduated pool IDs before enabling fee logic.

**Step 5: Run compile check**

```bash
cargo check -p bloom-cardano-agent
```

Expected: compile errors identify every context constructor that needs the new fields.

**Step 6: Commit**

```bash
git add bloom-cardano-agent/src/config.rs bloom-cardano-agent/src/main.rs bloom-cardano-agent/src/context.rs bloom-cardano-agent/resources
git commit -m "feat: configure graduated Splash pool fee"
```

### Task 4: Detect Graduation Transactions

**Files:**
- Create: `bloom-offchain-cardano/src/graduation/tracker.rs`
- Modify: `bloom-offchain-cardano/src/event_sink/handler.rs`
- Modify: `bloom-cardano-agent/src/entity.rs`
- Modify: `bloom-cardano-agent/src/main.rs`
- Modify: `spectrum-offchain-cardano/src/data/quadratic_pool.rs` tests only if helper coverage is needed

**Step 1: Add tracker tests**

Write tests around a tracker:

```rust
pub struct SnekPoolInputTracker { ... }

impl SnekPoolInputTracker {
    pub fn apply_tx<Ctx>(
        &self,
        tx_hash: TransactionHash,
        inputs: &[TransactionInput],
        outputs: &[(usize, TransactionOutput)],
        ctx: &Ctx,
        graduated_store: &GraduatedSplashPoolStore,
    ) -> GraduationJournalEntry;

    pub fn rollback_tx(&self, entry: GraduationJournalEntry, graduated_store: &GraduatedSplashPoolStore);
}
```

The tests should prove:
- Snek quadratic pool outputs are recorded by `OutputRef`;
- a transaction is a graduation when at least one input ref matches a recorded Snek pool and at least one output parses as `AnyPool`;
- direct Splash pool creation without a tracked Snek input returns empty;
- a Snek input without a Splash output returns empty;
- consumed Snek pool refs are removed after processing.

**Step 2: Add identity parser**

Add `SnekQuadraticPoolIdentity::try_from_ledger_identity(...)`. The identity parser must be able to recognize Snek outputs with:
- `DegenQuadraticPoolV1`
- `DegenQuadraticPoolV1T2T`

It must not depend on `OperatorCred`, `PoolValidation`, metadata, mints, executable status, or reserve bounds.

**Step 3: Provide Splash parser context**

For produced Splash pool outputs, the detector still needs the validators required by `AnyPool::try_from_ledger` and `PoolValidation`.

**Step 4: Integrate into ledger event handling**

For each ledger transaction:
- call `SnekPoolInputTracker::apply_tx(...)`, which atomically determines consumed Snek refs, produced Snek refs, and graduated Splash IDs;
- insert graduated Splash IDs into `GraduatedSplashPoolStore` inside `apply_tx(...)`;
- record the returned `GraduationJournalEntry` under `tx_hash`;
- flush the complete `GraduationStateSnapshot` after the ledger transaction is applied.

This must be a dedicated `GraduationEventHandler` in the ledger handler chain only, before the normal Bloom entity handler classifies produced Splash pools. Do not mutate durable graduation state from mempool handlers; mempool migrations that are later dropped must not classify pools permanently.

**Step 5: Add rollback journal**

On `TxApplied`, `apply_tx(...)` must return and persist:
- consumed Snek refs removed from `SnekPoolInputTracker`;
- produced Snek refs recorded into `SnekPoolInputTracker`;
- graduated Splash IDs inserted into `GraduatedSplashPoolStore`.

On `TxUnapplied`, use the journal to:
- remove graduated Splash IDs inserted by that transaction;
- remove produced Snek refs from the tracker;
- restore consumed Snek refs to the tracker.

Persist the store, live tracker refs, rollback journal, and chain tip atomically after every `TxApplied` and `TxUnapplied`. Do not leave persistence optional; rollback safety depends on it.

**Step 6: Add helper-level detector tests**

The detector should return Splash pool IDs when:
- at least one input ref is known by `SnekPoolInputTracker`
- at least one output parses as `AnyPool`

It should return empty when:
- outputs contain a Splash pool but inputs do not contain a Snek pool
- inputs contain a Snek pool but outputs do not contain a Splash pool
- parsing fails for either side

**Step 7: Add bootstrap/backfill**

Because the fee applies to pools graduated before the new code is deployed, bootstrap is mandatory. Support both:
- startup backfill scan over the chain-sync DB from `graduationBackfillFromPoint`, using the same tracker, producing a complete `GraduationStateSnapshot`;
- config allowlist `knownGraduatedPoolIds`, merged into `GraduatedSplashPoolStore` at startup for already-known pools.

For automatic future graduation detection, allowlist alone is insufficient. The running process also needs either a persisted live Snek ref snapshot or a backfill/replay to tip so `SnekPoolInputTracker` contains Snek pools that existed before startup.

**Step 8: Run tests**

```bash
cargo test -p bloom-offchain-cardano graduation -- --nocapture
cargo test -p bloom-cardano-agent graduation -- --nocapture
```

Expected: direct Splash pool creation is not marked; Snek-to-Splash migration is marked.

**Step 9: Commit**

```bash
git add bloom-offchain-cardano/src/graduation/tracker.rs bloom-offchain-cardano/src/event_sink/handler.rs bloom-cardano-agent/src/entity.rs bloom-cardano-agent/src/main.rs bloom-cardano-agent/src/config.rs
git commit -m "feat: detect graduated Splash pools"
```

### Task 5: Add Classified Splash Pool Wrapper

**Files:**
- Create: `bloom-offchain-cardano/src/pools/classified.rs`
- Modify: `bloom-offchain-cardano/src/pools/mod.rs`
- Modify: `bloom-cardano-agent/src/entity.rs`
- Modify: `bloom-cardano-agent/src/main.rs`

**Step 1: Write unit tests**

Tests should prove:
- `ClassifiedPool::stable_id()` equals `AnyPool::stable_id()`;
- `ClassifiedPool::pair_id()` equals `AnyPool::pair_id()`;
- `ClassifiedPool::liquidity()`, `static_price()`, `real_price()`, `quality()`, and `available_liquidity_on_side()` delegate to the inner pool when `origin == Direct`;
- parsing a pool whose ID is in `GraduatedSplashPoolStore` produces `PoolOrigin::SnekGraduated`;
- parsing a direct pool produces `PoolOrigin::Direct`;
- `pending_operator_fee` is zero immediately after ledger parsing.

**Step 2: Implement wrapper**

Use this shape:

```rust
#[derive(Debug, Copy, Clone)]
pub struct ClassifiedPool {
    pub inner: AnyPool,
    pub origin: PoolOrigin,
    pub fee_config: GraduatedPoolFeeConfig,
    pub pending_operator_fee: Lovelace,
}
```

Add `TryFromLedger<TransactionOutput, Ctx> for ClassifiedPool` that first parses `AnyPool`, then classifies it using `GraduatedSplashPoolStore` and copies `GraduatedPoolFeeConfig` from context.

**Step 3: Switch Bloom book maker type**

Change Bloom agent execution from `TLB<AnyOrder, AnyPool, PairId, ExUnits>` to `TLB<AnyOrder, ClassifiedPool, PairId, ExUnits>`. Keep ledger-level `AnyPool` support untouched for other modules. Do not switch Bloom to `Fifo`; that is Snek-specific in the current agents.

**Step 4: Run tests**

```bash
cargo test -p bloom-offchain-cardano classified_pool -- --nocapture
cargo test -p bloom-cardano-agent classified_pool -- --nocapture
```

Expected: direct pools behave exactly like current `AnyPool`; graduated IDs are classified.

**Step 5: Commit**

```bash
git add bloom-offchain-cardano/src/pools/classified.rs bloom-offchain-cardano/src/pools/mod.rs bloom-cardano-agent/src/entity.rs bloom-cardano-agent/src/main.rs
git commit -m "feat: classify Splash pools by origin"
```

### Task 6: Add Per-Trade Fee Transform In Matchmaking

**Files:**
- Modify: `bloom-offchain/src/execution_engine/liquidity_book/market_taker.rs`
- Modify: `bloom-offchain/src/execution_engine/liquidity_book/market_maker.rs`
- Modify: `bloom-offchain/src/execution_engine/liquidity_book/mod.rs`
- Modify: `bloom-offchain/src/execution_engine/liquidity_book/core.rs`
- Modify: `bloom-offchain/src/execution_engine/liquidity_book/state/mod.rs`
- Modify: `bloom-offchain-cardano/src/orders/mod.rs`
- Modify: `bloom-offchain-cardano/src/pools/classified.rs`

**Step 1: Add tests around the generic default**

In liquidity-book tests, prove `MakerBehavior::swap_with_taker(...)` default produces the same `TakeInProgress` and `MakeInProgress` as current `execute_with_maker(...)` for a simple fake taker/maker.

**Step 2: Add taker eligibility**

Add `graduated_splash_fee_eligible()` defaulting to false on `TakerBehaviour`, and override it in the existing manual `TakerBehaviour for AnyOrder` impl to return true only for `AnyOrder::Limit`. Put it on `TakerBehaviour`, not `MarketTaker`, because `AnyOrder` currently derives `MarketTaker` and already has a manual `TakerBehaviour` impl.

**Step 3: Replace generic call site**

In `execute_with_maker(...)`, replace the body with:

```rust
maker.swap_with_taker(target_taker, chunk_size)
```

Expected behavior is unchanged for all existing makers.

**Step 4: Make preselection fee-aware**

Modify `TLBState::preselect_market_maker(...)`, `dummy_swap(...)`, and `try_optimized_swap(...)` to receive `&target_taker`. For normal makers, the preview is unchanged. For `ClassifiedPool` with a graduated pool and eligible taker, compute `FillPreview.price` from gross taker input and net taker output after the 1% fee.

`preselect_market_maker(...)` must filter every maker's effective preview by `side.wrap(target_taker.price()).overlaps(preview.price)` before `min_by_key`/`max_by_key` chooses the best maker. Do not select one maker first and reject it later, because that can hide a later valid maker. Add tests for:
- gross ADA-to-token quote satisfies a limit but net quote fails;
- gross token-to-ADA quote satisfies a limit but net quote fails;
- two makers where the best gross graduated pool fails post-fee but a second maker passes and is selected;
- direct pool still uses current gross quote behavior.

**Step 5: Preserve transient fee through maker finalization**

Modify `MakeInProgress::finalized()` in `core.rs` so the rebalanced maker returned from `target.swap(...)` preserves transient metadata from the previewed maker:

```rust
let rebalanced = target.preserve_preview_metadata(next, rebalanced);
```

The default implementation returns `rebalanced`. `ClassifiedPool` copies accumulated `pending_operator_fee` from `next` into `rebalanced`.

**Step 6: Implement `ClassifiedPool::swap_with_taker`**

For non-eligible trades, delegate to the default behavior.

For eligible graduated trades:
- compute `(base_asset, quote_asset) = self.pair_id().assets()`;
- map `OnSide::Ask(_)` to base input and quote output;
- map `OnSide::Bid(_)` to quote input and base output;
- if input asset is `AssetClass::Native`, reduce maker input by `fee(gross_input)`, run inner pool swap with the reduced input, remove `gross_input` from taker, add pool output to taker, and increase `pending_operator_fee`;
- if output asset is `AssetClass::Native`, run inner pool swap with gross input, reduce taker output by `fee(gross_output)`, and increase `pending_operator_fee`;
- otherwise delegate with no fee.

Use saturating or checked arithmetic consistently with existing order code. Recommended: return no trade (`Next::Succ` with unchanged values) if `fee >= gross_input` or `fee >= gross_output`, because 100% fee configs should not create negative amounts.

**Step 7: Write integration tests**

Build synthetic recipe tests for:
- ordinary Splash pool plus limit order: no extra fee, same output as today;
- direct Splash pool plus limit order: no extra fee;
- graduated Splash pool plus grid order: no extra fee;
- graduated Splash pool plus ADA-to-token limit order: taker removes gross ADA, pool receives net ADA, operator fee is 1%;
- graduated Splash pool plus token-to-ADA limit order: pool pays gross ADA, taker receives net ADA, operator fee is 1%;
- gross quote passes but net quote fails: no trade is formed;
- `MakeInProgress::finalized()` preserves `pending_operator_fee` from the previewed maker;
- partial limit fill: residual datum `tradable_input` is reduced by gross input consumed, so no hidden future fee is stored in the limit datum;
- terminal limit fill: redeemer output receives net result and operator output receives fee.

**Step 8: Commit**

```bash
git add bloom-offchain/src/execution_engine/liquidity_book/market_taker.rs bloom-offchain/src/execution_engine/liquidity_book/market_maker.rs bloom-offchain/src/execution_engine/liquidity_book/mod.rs bloom-offchain/src/execution_engine/liquidity_book/core.rs bloom-offchain/src/execution_engine/liquidity_book/state/mod.rs bloom-offchain-cardano/src/orders/mod.rs bloom-offchain-cardano/src/pools/classified.rs
git commit -m "feat: apply graduated Splash fee during pool trades"
```

### Task 7: Execute Classified Pools And Pay Operator Fee

**Files:**
- Modify: `bloom-offchain-cardano/src/execution_engine/instances.rs`
- Modify: `bloom-cardano-agent/src/context.rs`

**Step 1: Add execution tests**

Test `BatchExec<Make<ClassifiedPool, FinalizedTxOut>>`:
- delegates validator selection to inner `AnyPool`;
- writes the same pool output as `AnyPool` for direct pools;
- calls `state.add_operator_interest(pending_operator_fee)` for graduated trades;
- returns an execution effect whose produced `ClassifiedPool` has `pending_operator_fee = 0`, because the fee is transient and has been consumed into operator interest.

**Step 2: Implement execution routing**

Add:

```rust
impl<Ctx> BatchExec<ExecutionState, EffectPreview<ClassifiedPool>, Ctx>
    for Magnet<Make<ClassifiedPool, FinalizedTxOut>>
```

The implementation unwraps `inner`, dispatches through existing `AnyPool` execution, then adds only this transition's `pending_operator_fee` to `ExecutionState`.

Do not add the fee in limit-order execution. The value movement is already represented by the taker receiving/removing gross or net values and the maker pool receiving/paying the corresponding net or gross values. Adding the operator interest at maker execution keeps ownership tied to the graduated pool. Reset `pending_operator_fee` to zero in the produced classified pool before the effect is synced back into the book, otherwise future recipes can double-count the same fee.

**Step 3: Run tests**

```bash
cargo test -p bloom-offchain-cardano classified_pool -- --nocapture
cargo test -p bloom-offchain-cardano execution_engine::instances -- --nocapture
```

Expected: direct pool execution remains unchanged; graduated pool execution creates operator interest exactly equal to accumulated fee.

**Step 4: Commit**

```bash
git add bloom-offchain-cardano/src/execution_engine/instances.rs bloom-cardano-agent/src/context.rs
git commit -m "feat: pay operator fee from classified pool trades"
```

### Task 8: Guard Order-Order Matching And Multi-Pool Ambiguity

**Files:**
- Modify: `bloom-cardano-agent/src/main.rs`
- Modify: `bloom-offchain-cardano/src/execution_engine/instances.rs`
- Test: relevant liquidity-book or execution-engine tests

**Step 1: Add explicit tests**

Tests should prove:
- order-order fills do not charge the graduated pool fee;
- if two pools exist for the same pair and only one is graduated, only execution through that exact pool charges the fee.

**Step 2: Confirm config policy**

If business wants every swap on a pair after graduation to pay the fee, implement pair-level classification explicitly. Otherwise keep pool-ID classification.

**Step 3: Run tests**

```bash
cargo test -p bloom-offchain-cardano graduated_pool_ambiguity -- --nocapture
```

Expected: no fee leakage to direct pools or order-order matching.

**Step 4: Commit**

```bash
git add bloom-cardano-agent/src/main.rs bloom-offchain-cardano/src/execution_engine/instances.rs
git commit -m "test: prevent graduated fee leakage"
```

### Task 9: End-To-End Validation

**Files:**
- Modify: `splash-testing-cardano/` or existing integration fixtures if present
- Modify: deployment config examples

**Step 1: Add an E2E fixture**

Create or reuse a scenario:
- Snek quadratic pool UTxO exists.
- Migration transaction consumes it and produces a Splash pool UTxO.
- A limit order swaps against that Splash pool.
- Operator reward output includes normal operator fee plus 1% graduated fee.

**Step 2: Add direct-pool negative scenario**

Create a direct Splash pool and execute an equivalent limit order. Assert no graduated fee is charged.

**Step 3: Run validation**

```bash
cargo test -p splash-testing-cardano graduated_splash_limit_fee -- --nocapture
cargo test --workspace --exclude spectrum-cardano-lib -- --nocapture
```

Expected: graduated scenario charges exactly 1%; direct pool scenario does not.

**Step 4: Commit**

```bash
git add splash-testing-cardano bloom-cardano-agent/resources
git commit -m "test: validate graduated Splash limit fee end to end"
```

---

## Rollout Notes

1. Deploy with `enabled: false` first and verify graduation detection metrics/logs.
2. Backfill or configure the known graduated Splash pool IDs.
3. Enable `relativeFeePercent: 1` on a canary operator.
4. Compare expected operator interest against submitted transactions for ADA-to-token and token-to-ADA fills.
5. Enable across all Bloom/Splash operators only after direct-pool negative checks are clean.

## Main Risks

- Missing historical graduations: solved by backfill or explicit startup allowlist.
- Charging the wrong pool when several pools share a pair: solved by pool-ID classification, not pair classification.
- Partial-fill double charging: solved by keeping limit datums gross and storing fee only as transient classified-pool execution state.
- Token-to-token fee ambiguity: keep zero fee until business defines the fee asset.
