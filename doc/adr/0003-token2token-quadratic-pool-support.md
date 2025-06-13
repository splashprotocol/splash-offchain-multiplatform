# 3. Token2Token quadratic pool support

Date: 2025-06-24

## Status

Accepted

## Context

Context

The Snek Cardano agent currently supports only Quadratic Pools for ADA ↔ Token pairs.
The implementation assumed that the first asset in the pool is always ADA.

There is now a requirement to extend support to Token ↔ Token pairs, by introducing a new pool type: QuadraticPoolT2T.

## Decision

- Relaxed the "first token must be ADA" constraint.
- Agent was updated to support any token as the first asset of the pool.
- Added full support for QuadraticPoolT2T (Token ↔ Token) pairs.

## Consequences

- The engine now supports both:
    - QuadraticPool v1 (ADA ↔ Token)
    - QuadraticPoolT2T (Token ↔ Token)
  
- Code consumers must be aware that:
  - The "first token = ADA" assumption no longer holds (only for QuadraticPoolT2T).

- Pairs can now be Token-Token.
