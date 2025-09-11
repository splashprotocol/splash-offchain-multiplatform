# 4. Market Taker Trait Refactoring for MultiStep and OneShot Orders

Date: 2023-07-12

## Status
Accepted

## Context
The original implementation of the `MarketTaker` trait included a `min_marginal_output` method that did not depend on added output:

```rust
pub trait MarketTaker {
    // ... other methods ...

    /// Minimal amount of output per execution step.
    fn min_marginal_output(&self) -> OutputAsset<u64>;
}
```

This PR introduces the concepts of **MultiStep** and **OneShot** orders, which have different requirements for calculating the minimum output:

1. For **MultiStep** orders, the minimum output is a fixed value per execution step.
2. For **OneShot** orders, the minimum output needs to be calculated based on the removed input.

## Decision
To address these issues, the following changes were implemented:

1. **Remove `min_marginal_output` from the `MarketTaker` trait**
   The method was removed from the base trait since it had different semantics depending on the order type.

2. **Create a new `MultiStepMarketTaker` trait**
   A new trait was introduced that extends `MarketTaker` and adds the parameter-less `min_marginal_output` method:
   ```rust
   pub trait MultiStepMarketTaker: MarketTaker {
       /// Minimal amount of output per execution step.
       fn min_marginal_output(&self) -> OutputAsset<u64>;
   }
   ```

3. **Create a new `OneShotMarketTaker` trait**
   A new trait was introduced for one-shot orders that extends `MarketTaker`:
   ```rust
   pub trait OneShotMarketTaker: MarketTaker {
       /// Minimal amount of output based on the removed input.
       fn min_marginal_output(&self, removed_input: InputAsset<u64>) -> OutputAsset<u64>;
   }
   ```

4. **Update all implementations**
   All implementations of `MarketTaker` were updated to also implement `MultiStepMarketTaker` where appropriate.

## Consequences

### Positive
- **Improved Code Clarity**: The code now clearly separates the different behaviors required for different types of market takers.