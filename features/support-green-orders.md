# Support Green Orders in Matchmaking Engine

## Green Orders

### On-chain

On-chain part is defined in terms of Cardano eUTxO programmable model and Aiken language

Green Orders are virtual messages signed by user and verified by on-chain Account (UTxO with script) to authorize swaps.
Spec: https://github.com/splashprotocol/aleph/blob/main/spec/aleph.pdf

Structure here for Intent (Virtual Order)
https://github.com/splashprotocol/aleph/blob/03b756c76f1177f7c53ab20407ea1c25b175a5b1/validators/witness.ak#L27

### Off-chain

Represent green order as a structure where account state is concatenated with intent. This way it will be compatible with the way onchain orders are represented currently in the rust code: 
```rust
pub struct EvolvingCardanoEntity(
    pub Bundled<Either<Baked<AnyOrder, OutputRef>, Baked<AnyPool, OutputRef>>, FinalizedTxOut>,
);
```
FinalizedTxOut - utxo matching order's state, in the case of green order it should be account's state utxo.

## Account

An entity living on-chain, persists over many state transitions, each state represented as a utxo.

Account Datum:
https://github.com/splashprotocol/aleph/blob/03b756c76f1177f7c53ab20407ea1c25b175a5b1/validators/account.ak#L10

It delegates validation to a witness here https://github.com/splashprotocol/aleph/blob/03b756c76f1177f7c53ab20407ea1c25b175a5b1/validators/account.ak#L48

witness is a validator for a batch of limit order swaps
https://github.com/splashprotocol/aleph/blob/03b756c76f1177f7c53ab20407ea1c25b175a5b1/validators/witness.ak#L37

Account Script:
https://github.com/splashprotocol/aleph/blob/03b756c76f1177f7c53ab20407ea1c25b175a5b1/validators/account.ak#L32


## Matchmaking Engine

Implemented in the bloom-offchain package, see  LiquidityBook implementation for TLB; The implementation is abstarct, ie the support for green orders should be added using existing abstractions (MarketTaker, etc)

## Green Order Processing Flow

1. Parse account state (requires TryFromLedger<TransactionOutput, C> for Account)
2. Save account state into a local storage (inmemory)
3. When an intent (virtual order) is received via HTTP API, retrieve the account the message belongs to and assemble GreenOrder structure (to match virtual intent with account you have to require specifying account_id when submitting an order);
4. Send the order to the engine (incoming orders are processed in the Executor's Stream implementation in the bloom-offchain package)
5. Executor saves the order into liquidity book and will process it when a matching order is found

## Successful implementation

Green order structure is added into EvolvingCardanoEntity (bloom-cardano-agent package), main.rs compiles, all traits implemented;

## Packages that are not relevant for the task

```
members = [
    "cardano-submit-api",
    "cardano-explorer",
    "snek-cardano-agent",
    "cardano-offchain-stableswap",
    "splash-dao-administration",
    "splash-dao-agent",
    "splash-dao-offchain",
    "engine-telemetry",
    "cardano-mempool-proxy",
    "cardano-utxo-monitor",
    "graphite",
    "splash-lp-indexer",
    "splash-testing"
]
```

## Implementation plan

### Step 1: Define Account and Intent data structures

Create Rust structures matching the on-chain Aiken types for Account and Intent.

#### Relevant components

| Component | Modification |
|-----------|-------------|
| `bloom-offchain-cardano/src/orders/green.rs` (new file) | Define `Account` struct with fields: magic (Data), allowlist (Vec<ScriptHash>), nonce (Vec<i64>), hot_cred (two VerificationKey), cold_cred (VerificationKeyHash), store ([u8; 32] - MPT root hash). Define `Intent` struct with fields: target_nonce (i64, Nonce), leaving_asset (PolicyId, AssetName), leaving_amount (u64), arriving_asset (PolicyId, AssetName), expected_arriving_amount (u64), fee_lovelace (u64), operator (VerificationKeyHash). Define `AuthorizedIntention` struct with fields: intent (Intent), remainder (u64), auth (Auth). Define `Auth` enum with variants: `Sig { signature, prefix, postfix }` for first execution, `Path { proof: MerkleProof }` for continuing partial fills. Define `GreenOrder` struct combining Account reference + AuthorizedIntention + account UTxO reference. |
| `bloom-offchain-cardano/src/orders/mod.rs:18-27` | Add `Green(GreenOrder)` variant to `AnyOrder` enum |

#### Requirements

| Requirement | Components | Description |
|-------------|------------|-------------|
| Account datum parsing | TryFromPData for Account | Parse PlutusData into Account struct following pattern in `limit.rs:256-297` |
| Intent decoding | Intent struct | Decode intent bytes from signed message |
| Signature verification | GreenOrder | Verify intent signature against account's hot_cred |
| MPT dependency | Cargo.toml | Add `mutree` crate (https://github.com/cfcosta/mutree) for Merkle Patricia Tree operations |
| Auth variants | Auth enum | `Sig` for first-time execution (new intent), `Path` for continuing partial fills (with MPT proof) |

### Step 2: Implement parsing of accounts from ledger

Parse Account UTxOs from on-chain data and store them in memory for later association with intents.

#### Relevant components

| Component | Modification |
|-----------|-------------|
| `bloom-offchain-cardano/src/orders/green.rs` | Implement `TryFromLedger<TransactionOutput, C> for Account` following pattern in `limit.rs:396-490`. Check validator address via `test_address()`, extract PlutusData datum, parse using TryFromPData |
| `bloom-offchain-cardano/src/event_sink/context.rs:30-78` | Add `DeployedScriptInfo<AccountV1>` to HandlerContextProto and HandlerContext. Add Has<DeployedScriptInfo<AccountV1>> impl |
| `spectrum-offchain-cardano/src/deployment.rs` | Add `AccountV1` variant to DeployedScriptHash enum for account script identification |
| `bloom-cardano-agent/src/` (new module) | Create `AccountIndex` - in-memory storage mapping account_id (Token/OutputRef) to (Account state, MPT instance). The MPT instance (using `mutree` crate) maintains the local copy of the account's store tree, enabling partial fill tracking. Implement insert/get/update methods. When account is first seen, initialize empty MPT. |
| `bloom-offchain-cardano/src/event_sink/handler.rs` | Add handler similar to `PairUpdateHandler` that extracts Account entities from ledger events and stores them in AccountIndex |

#### Requirements

| Requirement | Components | Description |
|-------------|------------|-------------|
| Account script hash config | ProtocolScriptHashes, deployment config | Add account validator script hash to deployment configuration |
| Account state tracking | AccountIndex | Track account UTxO state (OutputRef + datum) to later associate with intents |
| Context bounds | HandlerContext | Add necessary Has<> bounds for Account parsing context |
| MPT initialization | AccountIndex | When a new account is indexed, verify store hash matches empty MPT root (or reconstruct MPT state if account has existing partial fills) |
| MPT persistence | AccountIndex | Consider persisting MPT state to disk to survive restarts (or reconstruct from chain on startup) |

### Step 3: Update intent-relay and implement TCP receiver in bloom-cardano-agent

Modify intent-relay to include account_id in forwarded intents. Add TCP listener to bloom-cardano-agent to receive intents from intent-relay.

#### Architecture

```
User → HTTP POST → intent-relay → RocksDB queue → TCP forward → bloom-cardano-agent TCP listener
                   (with account_id)                            (constructs GreenOrder)
```

#### Relevant components

| Component | Modification |
|-----------|-------------|
| `intent-relay/src/intent.rs:1-8` | Extend `AuthedIntent` to include `account_id: [u8; 32]` (Token serialized). Update `encode()`/`decode()` to handle new field. |
| `intent-relay/src/server.rs:48-68` | Modify `SubmitIntentRequest` to include `account_id: HexString` field. Update `From<SubmitIntentRequest> for AuthedIntent` to include account_id. |
| `intent-relay/src/sender.rs` | No changes needed - TcpSender already sends encoded AuthedIntent bytes |
| `bloom-cardano-agent/src/intent_receiver.rs` (new) | TCP listener that: 1) Accepts connections on configured port 2) Reads AuthedIntent bytes from stream 3) Decodes using `AuthedIntent::decode()` 4) Looks up account in AccountIndex by account_id 5) Verifies signature against account's hot_cred 6) Constructs GreenOrder with MptSnapshot 7) Sends to Executor via channel |
| `bloom-cardano-agent/src/config.rs` | Add `intent_listener_addr: SocketAddr` for TCP listener binding |

#### Requirements

| Requirement | Components | Description |
|-------------|------------|-------------|
| account_id in AuthedIntent | intent-relay/intent.rs | Add 32-byte account_id field, update encode/decode with length prefix |
| account_id in HTTP request | intent-relay/server.rs | Require account_id in SubmitIntentRequest JSON body |
| TCP listener | bloom-cardano-agent/intent_receiver.rs | Bind to configured address, accept connections, spawn handler per connection |
| Stream parsing | intent_receiver.rs | Read length-prefixed messages from TCP stream, handle partial reads |
| Signature validation | intent_receiver.rs | Verify intent signature against account's hot_cred before accepting |
| Account lookup | intent_receiver.rs | Retrieve account state + MPT from AccountIndex; reject if not found |
| MptSnapshot creation | intent_receiver.rs | Create MptSnapshot from current MPT state for embedding in GreenOrder |
| Channel to Executor | intent_receiver.rs | Send constructed GreenOrder via mpsc channel |
| Connection handling | intent_receiver.rs | Handle multiple concurrent connections, reconnection from intent-relay |

#### TCP Protocol

```
┌────────────────────────────────────────────────────────────────┐
│ Message format (length-prefixed):                              │
│                                                                │
│ ┌──────────┬──────────────────────────────────────────────────┐│
│ │ 4 bytes  │ N bytes                                          ││
│ │ (len BE) │ AuthedIntent encoded                             ││
│ └──────────┴──────────────────────────────────────────────────┘│
│                                                                │
│ AuthedIntent encoding (updated):                               │
│ ┌──────────┬───────────────────────────────────────────────┐  │
│ │ 32 bytes │ account_id (Token bytes)                      │  │
│ ├──────────┼───────────────────────────────────────────────┤  │
│ │ 4 bytes  │ intent length                                 │  │
│ │ N bytes  │ intent bytes                                  │  │
│ ├──────────┼───────────────────────────────────────────────┤  │
│ │ 4 bytes  │ prefix length                                 │  │
│ │ N bytes  │ prefix bytes                                  │  │
│ ├──────────┼───────────────────────────────────────────────┤  │
│ │ 4 bytes  │ postfix length                                │  │
│ │ N bytes  │ postfix bytes                                 │  │
│ ├──────────┼───────────────────────────────────────────────┤  │
│ │ 4 bytes  │ signature length                              │  │
│ │ N bytes  │ signature bytes                               │  │
│ ├──────────┼───────────────────────────────────────────────┤  │
│ │ 32 bytes │ credential                                    │  │
│ └──────────┴───────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────┘
```

### Step 4: Implement order behaviour logic

Implement MarketTaker and TakerBehaviour traits for GreenOrder to integrate with the liquidity book matching engine.

#### Relevant components

| Component | Modification |
|-----------|-------------|
| `bloom-offchain-cardano/src/orders/green.rs` | Implement `MarketTaker for GreenOrder` following pattern in `limit.rs:181-230`: side() from asset pair, input()/output() from intent amounts, price() from expected_arriving_amount/leaving_amount ratio, budget()/fee() from fee_lovelace, time_bounds() (may need extension to Intent) |
| `bloom-offchain-cardano/src/orders/green.rs` | Implement `TakerBehaviour for GreenOrder` following pattern in `limit.rs:124-179`: with_applied_trade() updates amounts, calculates remainder, and computes proportional fee_remainder = (leaving_remainder * fee_lovelace / leaving_amount); checks termination when remainder == 0. with_budget_corrected() adjusts fee, try_terminate() checks completion |
| `bloom-offchain-cardano/src/orders/mod.rs:29-85` | Extend AnyOrder's TakerBehaviour impl to delegate to GreenOrder variant |
| `bloom-offchain-cardano/src/orders/mod.rs:87-121` | Extend AnyOrder's TryFromLedger to include GreenOrder (though GreenOrder comes from HTTP, not ledger) |

#### Requirements

| Requirement | Components | Description |
|-------------|------------|-------------|
| Price calculation | MarketTaker::price() | Calculate AbsolutePrice from intent's expected amounts ratio |
| Partial fill with remainder | TakerBehaviour::with_applied_trade() | When partially filled: compute remainder = leaving_amount - consumed; compute proportional fee_remainder; return Next::Succ with updated GreenOrder containing new remainder |
| Full fill termination | TakerBehaviour::with_applied_trade() | When remainder == 0, return Next::Term(TerminalTake) indicating order is complete |
| Nonce tracking | GreenOrder | Track target_nonce for replay protection; validated against account's nonce on-chain |
| Stable/Tradable traits | GreenOrder | Implement Stable (stable_id = account Token), Tradable (pair_id from assets) |

### Step 5: Integrate GreenOrder into Executor stream

Connect HTTP-received GreenOrders and continuation scanner to the Executor's processing pipeline.

#### Relevant components

| Component | Modification |
|-----------|-------------|
| `bloom-offchain/src/execution_engine/mod.rs:202-254` | Add channel receiver for GreenOrder events to Executor struct. Add `Arc<RwLock<AccountIndex>>` field for success callback access. |
| `bloom-offchain/src/execution_engine/mod.rs:764-940` | In `poll_next()`, poll the GreenOrder channel alongside upstream; convert received GreenOrders to appropriate Event type and process through multi_book |
| `bloom-offchain/src/execution_engine/mod.rs:504-554` | Extend `on_execution_effects_success()` to handle GreenOrder effects: extract MptDelta and apply to AccountIndex |
| `bloom-cardano-agent/src/entity.rs:87-151` | GreenOrder flows through AnyOrder in EvolvingCardanoEntity - no direct modification needed if AnyOrder enum is extended |
| `bloom-cardano-agent/src/continuation_scanner.rs` (new) | Async task that periodically scans AccountIndex for accounts with pending partial fills (remainder > 0 in MPT). For each pending intent: generate Auth::Path proof from local MPT, construct GreenOrder, send to Executor channel. |

#### Requirements

| Requirement | Components | Description |
|-------------|------------|-------------|
| Event conversion | Executor | Convert GreenOrder to Event<AnyOrder, _, _, _, _, _> format |
| Pair routing | MultiPair book | Route GreenOrder to correct pair book based on leaving_asset/arriving_asset |
| AccountIndex in Executor | Executor struct | Add `account_index: Arc<RwLock<AccountIndex>>` field to Executor for use in success callback |
| MptDelta handling | on_execution_effects_success | For GreenOrder effects, lock AccountIndex and apply MptDelta (insert/update/delete) |
| Continuation scanner | New async task | Periodic task (e.g., every 5s) that iterates AccountIndex, finds pending remainders, generates continuation GreenOrders with Auth::Path |
| Scanner channel | bloom-cardano-agent | Channel from continuation scanner to Executor for injecting continuation GreenOrders |

### Step 6: Implement order execution logic

Implement BatchExec to translate GreenOrder execution into on-chain transaction consuming account UTxO.

#### Relevant components

| Component | Modification |
|-----------|-------------|
| `bloom-offchain-cardano/src/execution_engine/instances.rs` | Implement `BatchExec<ExecutionState, (EffectPreview<GreenOrder>, MptDelta), Ctx> for Magnet<Take<GreenOrder, FinalizedTxOut>>` following pattern at lines 98-184. Key differences: 1) Input is account UTxO 2) Redeemer is Delegate action with witness index 3) Must include witness script reference 4) Update account datum with new nonce 5) Adjust account value (subtract leaving_asset, add arriving_asset) 6) Use embedded MptSnapshot to compute new MPT root for account's store field 7) Return MptDelta alongside ExecutionEff for later application to AccountIndex |
| `bloom-offchain-cardano/src/execution_engine/instances.rs` | Add witness script handling - the account delegates to witness validator which validates the swap batch |
| `spectrum-offchain-cardano/src/deployment.rs` | Add `WitnessV1` variant for witness validator script |
| `bloom-offchain-cardano/src/orders/green.rs` | Add helper functions for MPT operations: compute intent digest (hash of intent data), insert/update intent in MPT, generate new MPT root |

#### Requirements

| Requirement | Components | Description |
|-------------|------------|-------------|
| Account UTxO as input | BatchExec | ScriptInputBlueprint references account UTxO, not a separate order UTxO |
| Delegate redeemer | BatchExec | Redeemer is `Delegate(witness_index)` pointing to witness validator |
| Witness reference script | BatchExec | Add witness validator as reference script input |
| Nonce update | BatchExec | Update account datum's nonce field to prevent replay |
| Multi-intent batching | BatchExec | Witness validates batch of intents; may need to group multiple GreenOrders from same account |
| **Partial fill - Sig auth** | BatchExec, MPT | For first execution (Auth::Sig): after partial fill, call `mpt.insert(intent_digest, updated_intent)` where updated_intent has reduced leaving_amount = remainder, proportionally reduced expected_arriving_amount and fee. Update account's store field with new MPT root hash. |
| **Partial fill - Path auth** | BatchExec, MPT | For continuing execution (Auth::Path): verify proof against current store root, then call `mpt.update(old_digest, new_digest)` with updated remainder values. Update account's store field with new MPT root hash. |
| **Full fill - Sig auth** | BatchExec, MPT | For first execution that completes fully (remainder == 0): no MPT update needed, account store unchanged |
| **Full fill - Path auth** | BatchExec, MPT | For continuing execution that completes: call `mpt.delete(intent_digest)` to remove from tree (or leave as completed marker). Update account's store field. |
| Intent digest computation | green.rs | Hash intent fields to create unique digest used as MPT key. Must match on-chain computation exactly. |
| Local MPT sync | AccountIndex | After successful tx submission, update local MPT copy to match new account state |

### Step 7: Wire everything together in bloom-cardano-agent

Connect all components in the agent's main entry point.

#### Relevant components

| Component | Modification |
|-----------|-------------|
| `bloom-cardano-agent/src/main.rs` | Initialize AccountIndex, start HTTP server for intents, create channel between HTTP handler and Executor, add account script hash to deployment config |
| `bloom-cardano-agent/src/config.rs` | Add configuration for intent HTTP endpoint address, account script hash |
| `bloom-cardano-agent/src/entity.rs` | Verify EvolvingCardanoEntity works with GreenOrder through AnyOrder enum |

#### Requirements

| Requirement | Components | Description |
|-------------|------------|-------------|
| Compilation | main.rs | All trait implementations complete; main.rs compiles |
| Account tracking | Event handlers | Account UTxOs parsed and stored on chain sync |
| Intent submission | HTTP server | Intents received, validated, converted to GreenOrders |
| Execution | Executor + BatchExec | GreenOrders matched and executed in transactions |

## Summary of new files and modifications

### New dependencies
- `mutree` crate (https://github.com/cfcosta/mutree) - Merkle Patricia Tree implementation for tracking partial fills

### New files
- `bloom-offchain-cardano/src/orders/green.rs` - Account, Intent, AuthorizedIntention, Auth, GreenOrder, MptSnapshot, MptDelta structs with trait impls and MPT helpers
- `bloom-cardano-agent/src/account_index.rs` - In-memory account state storage with per-account MPT instances, wrapped in `Arc<RwLock<>>` for shared access
- `bloom-cardano-agent/src/continuation_scanner.rs` - Async task scanning for pending partial fills and generating continuation GreenOrders
- `bloom-cardano-agent/src/intent_receiver.rs` - TCP listener receiving AuthedIntent from intent-relay, constructing GreenOrders

### Modified files
- `bloom-offchain-cardano/src/orders/mod.rs` - Add GreenOrder to AnyOrder enum
- `bloom-offchain-cardano/src/event_sink/context.rs` - Add account script context
- `bloom-offchain-cardano/src/event_sink/handler.rs` - Add account event handler (receives `Arc<RwLock<AccountIndex>>`)
- `bloom-offchain-cardano/src/execution_engine/instances.rs` - Add GreenOrder BatchExec impl returning `(EffectPreview, MptDelta)`
- `bloom-offchain/src/execution_engine/mod.rs` - Add `Arc<RwLock<AccountIndex>>` to Executor, extend `on_execution_effects_success()` for MptDelta handling, add GreenOrder channel receiver
- `spectrum-offchain-cardano/src/deployment.rs` - Add AccountV1, WitnessV1 script variants
- `bloom-cardano-agent/src/main.rs` - Wire up AccountIndex (create, wrap in Arc), account event handler, TCP intent listener, continuation scanner, pass AccountIndex to Executor
- `bloom-cardano-agent/src/config.rs` - Add `intent_listener_addr: SocketAddr` for TCP listener, account script hash config
- `intent-relay/src/intent.rs` - Add `account_id: [u8; 32]` field to AuthedIntent, update encode/decode
- `intent-relay/src/server.rs` - Add `account_id: HexString` to SubmitIntentRequest, update conversion to AuthedIntent
- `bloom-offchain-cardano/Cargo.toml` - Add mutree dependency

## Partial Execution Flow Summary

```
First execution (user-initiated):
1. User submits intent with Auth::Sig (signature-authenticated)
2. Engine matches intent, partial fill occurs (remainder > 0)
3. BatchExec computes:
   - remainder = leaving_amount - consumed
   - fee_remainder = remainder * fee_lovelace / leaving_amount
   - updated_intent with new amounts
4. MPT operation: mpt.insert(intent_digest, updated_intent)
5. Account datum updated: store = new_mpt_root, nonce incremented
6. Transaction submitted
7. On tx confirmation: AccountIndex updates local MPT copy

Automatic continuation (system-initiated):
8. System detects partial fill in local MPT (remainder > 0 stored)
9. System generates Auth::Path proof from local MPT
10. GreenOrder reconstructed with Auth::Path for remaining amount
11. Engine matches remaining amount against liquidity
12. If partial again: mpt.update(old_digest, new_digest)
13. If complete: mpt.delete(intent_digest)
14. Account datum updated with new store root
15. Repeat from step 8 until remainder == 0
```

Note: The off-chain system maintains a local copy of each account's MPT. When a partial fill occurs, the remainder is stored in the MPT and the system can automatically continue execution without user re-submission. The Auth::Path proof is generated locally from the MPT to authorize continuation.

## AccountIndex Access Pattern

### Where AccountIndex is stored
`AccountIndex` is created in `bloom-cardano-agent/src/main.rs` and wrapped in `Arc<RwLock<AccountIndex>>` for shared access across components.

### Access points

| Component | When | Operation | How to access |
|-----------|------|-----------|---------------|
| **Event handler** (bloom-offchain-cardano/event_sink/handler.rs) | On-chain account UTxO detected | `account_index.insert(account_id, (account_state, mpt))` | Handler receives `Arc<RwLock<AccountIndex>>` as parameter (similar to how `PairUpdateHandler` receives topic/index) |
| **TCP intent receiver** (bloom-cardano-agent/intent_receiver.rs) | Intent received from intent-relay | `account_index.get(account_id)` → retrieve Account + MPT snapshot | Receiver task holds `Arc<RwLock<AccountIndex>>` |
| **GreenOrder construction** (TCP receiver or continuation scanner) | Building GreenOrder for execution | Read current MPT state, create `MptSnapshot` with current root + relevant proofs | Same as above |
| **BatchExec** (bloom-offchain-cardano/execution_engine/instances.rs) | During transaction building | Compute new MPT root from embedded snapshot | **Does NOT access AccountIndex directly** - uses data embedded in GreenOrder |
| **Success callback** (bloom-offchain/execution_engine/mod.rs) | After tx confirmation | `account_index.apply_mpt_delta(account_id, delta)` | Extend Executor or add hook in success callback |
| **Continuation scanner** (new component) | Periodic scan for pending partial fills | `account_index.iter_pending_remainders()` → generate new GreenOrders | Separate async task with `Arc<RwLock<AccountIndex>>` |

### GreenOrder MPT data embedding

To avoid passing AccountIndex through all the generic Ctx bounds, GreenOrder embeds the necessary MPT data:

```rust
pub struct GreenOrder {
    pub account_id: Token,
    pub account_utxo: FinalizedTxOut,  // Current account UTxO
    pub authorized_intention: AuthorizedIntention,
    pub mpt_snapshot: MptSnapshot,     // Embedded MPT state for this execution
}

pub struct MptSnapshot {
    pub current_root: [u8; 32],
    pub pending_intent: Option<(IntentDigest, Intent)>,  // For Path auth continuation
    // For partial fill: pre-computed new state
    pub compute_new_root: Box<dyn Fn(Intent) -> [u8; 32] + Send + Sync>,
}
```

### MPT update flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ 1. TCP receiver gets AuthedIntent from intent-relay                         │
│    ↓                                                                        │
│ 2. Decode AuthedIntent, extract account_id                                  │
│    ↓                                                                        │
│ 3. Lock AccountIndex, lookup account by account_id                          │
│    ↓                                                                        │
│ 4. Verify signature against account's hot_cred                              │
│    ↓                                                                        │
│ 5. Create MptSnapshot from current MPT state                                │
│    ↓                                                                        │
│ 6. Construct GreenOrder with embedded MptSnapshot                           │
│    ↓                                                                        │
│ 7. Send GreenOrder to Executor (via channel)                                │
│    ↓                                                                        │
│ 8. Unlock AccountIndex                                                      │
└─────────────────────────────────────────────────────────────────────────────┘
                                    ↓
┌─────────────────────────────────────────────────────────────────────────────┐
│ 9. Executor matches GreenOrder in liquidity book                            │
│    ↓                                                                        │
│ 10. BatchExec::exec() called with GreenOrder                                │
│    ↓                                                                        │
│ 11. BatchExec uses mpt_snapshot to compute new_root for account datum       │
│    ↓                                                                        │
│ 12. BatchExec returns EffectPreview<GreenOrder> containing:                 │
│     - Updated/Eliminated effect                                             │
│     - MptDelta: (account_id, operation: Insert|Update|Delete, intent_data)  │
│    ↓                                                                        │
│ 13. Transaction submitted, pending_effects stored                           │
└─────────────────────────────────────────────────────────────────────────────┘
                                    ↓
┌─────────────────────────────────────────────────────────────────────────────┐
│ 14. Tx confirmed (feedback received)                                        │
│     ↓                                                                       │
│ 15. on_execution_effects_success() called                                   │
│     ↓                                                                       │
│ 16. For GreenOrder effects: extract MptDelta                                │
│     ↓                                                                       │
│ 17. Lock AccountIndex, apply MptDelta to account's MPT                      │
│     ↓                                                                       │
│ 18. Update account's stored UTxO reference to new one                       │
│     ↓                                                                       │
│ 19. Unlock AccountIndex                                                     │
└─────────────────────────────────────────────────────────────────────────────┘
```

### MptDelta structure

```rust
pub enum MptDelta {
    /// First execution partial fill: insert new intent remainder
    Insert { account_id: Token, intent_digest: [u8; 32], updated_intent: Intent },
    /// Continuation partial fill: update existing intent remainder
    Update { account_id: Token, old_digest: [u8; 32], new_digest: [u8; 32], updated_intent: Intent },
    /// Continuation complete: remove intent from tree
    Delete { account_id: Token, intent_digest: [u8; 32] },
    /// First execution complete: no MPT change
    None,
}
```

### Extended effect type for GreenOrder

```rust
pub type GreenOrderEffect = (ExecutionEff<...>, MptDelta);
// Or extend EffectPreview to carry additional metadata
```