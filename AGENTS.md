# Repository Notes

## Domain Overview

This repository contains the Bloom/Splash and Snek.fun Cardano execution engines.

- Splash supports multiple pool families under `spectrum-offchain-cardano/src/data/cfmm_pool/` and currently routes DEX swap execution through the limit order implementation at `bloom-offchain-cardano/src/orders/limit.rs`.
- Snek.fun supports quadratic bonding pools in `spectrum-offchain-cardano/src/data/quadratic_pool.rs` and instant orders in `bloom-offchain-cardano/src/orders/instant.rs`.
- Snek.fun instant orders use an ad-hoc commission path implemented in `bloom-offchain-cardano/src/orders/adhoc.rs`. The matcher sees a virtual reduced input amount, while on-chain execution spends the full input amount and sends the fee remainder to the batcher/operator wallet.
- Graduation, also called bonding, connects Snek.fun pools to Splash pools. A graduation transaction can be identified when a transaction consumes a Snek.fun quadratic pool input and produces a Splash pool output. The external migration script lives in `/Users/aleksandr/IdeaProjects/snekfun-amm-pool-launcher/index.ts`.
- Splash pools may also be created directly by users through the interface. Business logic that applies only to graduated pools must not apply to directly created Splash pools.

## Current Business Requirement Context

Business wants a 1% fee from swaps on Splash pools that graduated from Snek.fun pools. The intended behavior is the same virtual-input approach used by Snek.fun instant orders, but scoped to Splash limit orders and only when the matched Splash pool is known to be graduated from Snek.fun.
