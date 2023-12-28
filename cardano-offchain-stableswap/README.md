# StableSwap math

This repository contains core StableSwap's math functions fo off-chain operators.
These functions allow to calculate the required output values and constants for substitution into the pool datum.

## Functions for AMM actions

All functions are suitable for pools containing `n = 2, 3, 4, ...` tradable tokens.

* `liquidity_action` is universal function for AMM operations that change the number of liquidity tokens in the pool,
  i.e. _**deposit**_ arbitrary tokens out of `n` in arbitrary amount or _**redeem**_ the required number of liquidity
  tokens to receive desired tokens out of `n` in the desired amount.
* `redeem_uniform` allows to **_redeem_** a certain number of liquidity tokens in order to receive all `n` tradable
  tokens in an amount corresponding to the current ratio in the pool.
* `swap` is responsible for trading assets `i` to `j`, where both assets belong to `n`.