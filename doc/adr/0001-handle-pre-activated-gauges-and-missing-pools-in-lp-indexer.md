# 1. Handle Pre-Activated Gauges and Missing Pools in LP Indexer

Date: 2025-05-21

## Status
Accepted

## Context
The current implementation of the LP indexer does not correctly handle two specific edge cases:

1. **Pre-activated gauges** — where a gauge activation event is received *before* the corresponding gauge entity is actually created.
2. **Gauges with non-existent pools** — where the gauge references a pool that does not exist in the system.

These scenarios lead to incomplete or incorrect processing of gauge activation events, resulting in inconsistencies in downstream systems (e.g., missing farm activation events).

## Decision
To address these issues, the following changes will be implemented:

1. **New RocksDB Key Format for Pre-Activated Gauges**  
   A new key pattern will be introduced in RocksDB to track pre-activated gauges:
   ```
   "pre-activated" || gauge_id
   ```
   When a gauge activation event is received but the gauge entity does not yet exist, this key will be written to the database.

2. **Deferred Activation Handling**  
   Upon the creation of a gauge entity, the system will check if a corresponding `"pre-activated" || gauge_id` key exists in RocksDB.  
   - If it does, a `FarmActivation` event will be emitted.
   - The key will then be removed to prevent reprocessing.

3. **Strict Validation of Pool Existence**  
   If a gauge references a pool that does not exist at the time of processing, it will be **explicitly rejected**. This ensures that only valid and complete gauge definitions are processed by the system.

## Consequences

- **Improved Resilience**: The system can now reliably handle out-of-order event delivery for gauge activation and creation.
- **Data Integrity**: Invalid gauges pointing to non-existent pools will be rejected, preventing pollution of the data model.
- **Slight Storage Overhead**: Temporary keys for pre-activated gauges will be stored in RocksDB until processed.
- **Increased Complexity**: Adds a small layer of logic for conditional activation based on key presence in RocksDB.
