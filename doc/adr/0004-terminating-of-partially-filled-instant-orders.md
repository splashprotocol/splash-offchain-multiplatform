# 4. Termination of Partially Filled Instant Orders

**Date:** 2025-07-29

## Status

Accepted

## Context

The current implementation of the instant order contract expects that a terminal state of the order (i.e., the final output address matches `conf.redeemer_address` in the datum) will consume the entire tradable input amount.

However, in reality, the processing flow may reach a terminal state without consuming all the tradable input, which leads to a script execution error.

Instant orders are generally created with a budget sufficient for only a single execution step. This is acceptable when the liquidity in the target pool is sufficient to fulfill the order in one attempt. However, in scenarios where pool liquidity is insufficient, the order may be only partially filled. If such an order reaches a terminal state without consuming all the tradable input, it becomes invalid. Attempting to include it in a transaction results in immediate script failure. As a result, the entire transaction is invalidated — including other valid orders batched in the same transaction.

## Decision

Introduce a new interpretation of the terminal state for instant orders:

> If the `input_amount` of a terminal order is ≥ 0 (i.e., the remaining budget is insufficient to satisfy the next execution step), we consider the order complete and remove it from the execution engine (EE) state.

## Consequences

- Prevents transaction-wide failures caused by improperly terminated partially filled orders.
- Ensures valid orders can still be processed even when other orders are only partially filled and cannot proceed further.
- May require updates to analytics and frontend to distinguish between fully and partially filled but actually finalized orders.
