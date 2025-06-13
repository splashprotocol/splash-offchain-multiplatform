# 2. Record architecture decisions

Date: 2025-06-24

## Status

Accepted

## Context

The Splash execution engine for Cardano currently supports multiple CFMM (Constant Function Market Maker) pool types:

- **Classical AMM Pool** — various versions of traditional constant-product pools
- **Single Royalty Pool** — a pool type where a portion of the swap amount is redirected to a single royalty address

To accommodate a new requirement, we need to support **Double Royalty Pools**. These pools introduce a second royalty receiver, enabling royalty routing to two distinct addresses for each swap operation.

## Decision

To support the Double Royalty Pool:

- We refactored the internal shared structure used by all CFMM pool types. It is now split into more specific categories:
  - `Classic`
  - `FeeSwitch`
  - `Royalty`

  This decomposition reduces the complexity of the shared logic and makes room for pool-specific behaviors.

- We extended the existing Royalty pool implementation to support a second royalty output:
  - Added two new fields:
    - `second_royalty_x`
    - `second_royalty_y`
  - These fields represent the additional royalty amounts to be distributed from each token leg of the pool.

- The output calculation logic was updated to:
  - Correctly incorporate `second_royalty_x` and `second_royalty_y` into the royalty and fee computation logic
  - Ensure backwards compatibility with existing pool types

## Consequences

- The execution engine now supports a new pool type: **Double Royalty Pool**
- The system is more modular and better equipped for future pool type extensions
- Slight increase in execution complexity for swap operations involving double royalty
