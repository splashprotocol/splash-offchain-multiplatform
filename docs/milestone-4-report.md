# Milestone 4 Completion Report

## Scope

Milestone 4 requested:

1. a Decentralized Execution Assessment SDK;
2. tools inside the SDK that can recognize common MEV patterns such as
   front-running and sandwich attacks;
3. evidence that these tools can support avoiding unfair parties.

This report covers the SDK implementation, the example application, and the
auditor-facing evidence.

## Repository and Revision Context

- GitHub repository:
  `https://github.com/splashprotocol/splash-offchain-multiplatform`
- note on repository naming:
  this checkout's `origin` still uses the legacy path
  `https://github.com/splashprotocol/spectrum-offchain-multiplatform.git`,
  which GitHub redirects to the canonical browser repository above
- working branch used for this report:
  `bromel777/dea-sdk-milestone-4-plan`
- implementation revision validated by this report:
  `5a486fbf1a4409b7670f3aa4a7b35eb71a287d9a`
- branch source browser root:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/tree/bromel777/dea-sdk-milestone-4-plan`

Milestone-4 evidence in this report was validated from the branch and revision
listed above. The published branch contains the report and documentation commits
for milestone submission. The deterministic code-verification rerun in this
report targets the validated implementation revision above, because later
branch-level updates are documentation-only.

## Delivered Outputs

### 1. DEA SDK crate

The repository now contains a dedicated reusable SDK crate:

- crate root:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/tree/bromel777/dea-sdk-milestone-4-plan/dea-sdk`
- crate manifest:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/Cargo.toml`

Primary modules:

- shared config:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/src/config.rs`
- normalized domain:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/src/domain.rs`
- actor attribution:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/src/attribution.rs`
- projection helpers:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/src/projection.rs`
- front-run detector:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/src/detectors/front_run.rs`
- sandwich detector:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/src/detectors/sandwich.rs`
- actor fairness aggregation:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/src/aggregation.rs`
- replay facade:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/src/replay.rs`

### 2. Example application

The repository now contains a small runnable example application that replays
fixture observations through the SDK and prints structured findings:

- example app root:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/tree/bromel777/dea-sdk-milestone-4-plan/dea-sdk-example`
- example binary:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk-example/src/main.rs`
- example README:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk-example/README.md`

### 3. Design and implementation plans

The milestone design and execution plans are also stored in the repository:

- design:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/docs/plans/2026-06-18-dea-sdk-design.md`
- implementation plan:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/docs/plans/2026-06-18-dea-sdk.md`

## What the SDK Does

The SDK consumes normalized mempool and ledger observations and emits:

- front-run candidates;
- front-run findings;
- sandwich candidates;
- sandwich findings;
- actor fairness profiles.

The actor fairness profile is the key bridge to the milestone acceptance
criterion. It does not treat one suspicious transaction sequence as enough to
exclude or down-rank a party. Instead it accumulates repeated suspicious
behavior across:

- multiple victim opportunities;
- multiple observations;
- multiple distinct days;
- multiple confirmed findings.

This is a deliberate protection against false positives caused by distributed
mempool propagation.

## Acceptance Criteria Mapping

### Tools within the SDK can identify specific off-chain behaviour patterns

Satisfied by:

- front-run candidate and finding detection in:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/src/detectors/front_run.rs`
- sandwich candidate and finding detection in:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/src/detectors/sandwich.rs`
- normalized replay input and report output in:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/src/replay.rs`

Patterns currently demonstrated:

- front-running on a single pool with mempool ordering and ledger-order harm
  evidence;
- sandwich attacks with same-actor bracketing, same-pool ordering, victim harm,
  and unwind logic;
- same-block ordering support through `(slot, tx_index)` handling.

### Tools can prevent the selection of unfair parties

Satisfied by:

- actor fairness aggregation in:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/src/aggregation.rs`
- profile output schema in:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/src/domain.rs`

Operational meaning:

- one suspicious transaction sequence can create evidence;
- one suspicious transaction sequence cannot by itself create an actor-level
  exclusion or down-rank recommendation;
- repeated suspicious behavior is required before an actor leaves `low` risk;
- consuming applications can use `ActorFairnessProfile` to observe, down-rank,
  or exclude parties with sustained suspicious behavior.

## Auditor Evidence

### Test and replay coverage

The milestone currently includes:

- unit tests for detector and aggregation logic:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/tree/bromel777/dea-sdk-milestone-4-plan/dea-sdk/src`
- fixture-driven integration tests:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/tree/bromel777/dea-sdk-milestone-4-plan/dea-sdk/tests`
- example-app smoke tests:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk-example/tests/cli_smoke.rs`

Verified commands:

```bash
cargo test -p dea-sdk -p dea-sdk-example
cargo check -p dea-sdk -p dea-sdk-example
cargo run -q -p dea-sdk-example -- dea-sdk/tests/fixtures/front_run_basic.json
cargo run -q -p dea-sdk-example -- dea-sdk/tests/fixtures/sandwich_basic.json
cargo run -q -p dea-sdk-example -- dea-sdk/tests/fixtures/actor_profile_basic.json
```

Latest validated results:

- `dea-sdk` unit tests: `21 passed`
- fixture tests:
  - `front_run_fixture`: pass
  - `sandwich_fixture`: pass
  - `actor_profile_fixture`: pass
- `dea-sdk-example` smoke tests: `2 passed`
- `cargo check`: pass

### What the fixtures demonstrate

#### Front-run fixture

Fixture:

- `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/tests/fixtures/front_run_basic.json`

Expected and validated behavior:

- `1` front-run candidate
- `1` confirmed front-run finding
- harm kind:
  `worsePrice`
- actor profile:
  - actor id: `attacker-1`
  - risk band: `low`

This shows that a valid single front-run pattern is detected, but a single
sequence still remains low risk at actor level.

#### Sandwich fixture

Fixture:

- `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/tests/fixtures/sandwich_basic.json`

Expected and validated behavior:

- `1` sandwich candidate
- `1` confirmed sandwich finding
- attacker actor:
  `attacker-2`
- actor profile:
  - risk band: `low`

This shows that a valid sandwich is detected structurally and economically, but
one isolated sequence still does not escalate to actor-level punishment.

#### Actor-profile fixture

Fixture:

- `https://github.com/splashprotocol/splash-offchain-multiplatform/blob/bromel777/dea-sdk-milestone-4-plan/dea-sdk/tests/fixtures/actor_profile_basic.json`

Expected and validated behavior:

- repeated actor:
  `repeat-attacker`
- isolated actor:
  `isolated-attacker`

Validated actor outcomes from the example replay:

- `isolated-attacker`
  - `frontRunCount = 1`
  - `distinctVictimCount = 1`
  - `distinctDayCount = 1`
  - `riskBand = low`
- `repeat-attacker`
  - `frontRunCount = 2`
  - `distinctVictimCount = 2`
  - `distinctDayCount = 2`
  - `riskBand = medium`

This is the critical milestone-4 proof:

- the SDK does **not** decide from one suspicious sequence;
- it escalates only when repeated suspicious behavior appears across multiple
  independent observations.

## Example Local Reproduction

### Prerequisites

- Rust toolchain installed
- repository checked out on branch:
  `bromel777/dea-sdk-milestone-4-plan`

For a deterministic rerun of the exact implementation state validated by this
report, auditors should run:

```bash
git checkout bromel777/dea-sdk-milestone-4-plan
git checkout 5a486fbf1a4409b7670f3aa4a7b35eb71a287d9a
```

### Commands

Run the full milestone-4 verification:

```bash
cargo test -p dea-sdk -p dea-sdk-example
cargo check -p dea-sdk -p dea-sdk-example
```

Replay the example fixtures:

```bash
cargo run -q -p dea-sdk-example -- dea-sdk/tests/fixtures/front_run_basic.json
cargo run -q -p dea-sdk-example -- dea-sdk/tests/fixtures/sandwich_basic.json
cargo run -q -p dea-sdk-example -- dea-sdk/tests/fixtures/actor_profile_basic.json
```

### What auditors should expect

- `front_run_basic.json`:
  one front-run finding, actor remains `low`
- `sandwich_basic.json`:
  one sandwich finding, actor remains `low`
- `actor_profile_basic.json`:
  repeated actor escalates above isolated actor, while isolated actor remains
  `low`

## Evidence of Milestone Completion

Open-source repository links showcasing implementation, documentation, and
examples:

- SDK implementation:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/tree/bromel777/dea-sdk-milestone-4-plan/dea-sdk`
- example app:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/tree/bromel777/dea-sdk-milestone-4-plan/dea-sdk-example`
- design and plan docs:
  `https://github.com/splashprotocol/splash-offchain-multiplatform/tree/bromel777/dea-sdk-milestone-4-plan/docs/plans`