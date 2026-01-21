use std::cmp::{max, Ordering};
use std::fmt::{Display, Formatter};

use bloom_offchain::execution_engine::liquidity_book::core::{Next, TerminalTake, Unit};
use bloom_offchain::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{
    AbsolutePrice, FeeAsset, InputAsset, OutputAsset, RelativePrice,
};
use bloom_offchain::execution_engine::liquidity_book::weight::Weighted;
use cml_chain::plutus::PlutusData;
use cml_crypto::{Ed25519KeyHash, RawBytesEncoding};
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::plutus_data::PlutusDataExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::{AssetClass, Token};
use spectrum_offchain::domain::{Stable, Tradable};
use spectrum_offchain_cardano::data::pair::{side_of, PairId};

/// Account datum parsed from on-chain UTxO.
/// Corresponds to AccountState in validators/account.ak
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// Magic bytes to recognize accounts
    pub magic: Vec<u8>,
    /// List of allowed delegatee script hashes
    pub allowlist: Vec<[u8; 28]>,
    /// Monotonically increasing nonce protecting against replay attacks
    pub nonce: Vec<i64>,
    /// Hot credentials for authorizing intents (two verification keys)
    pub hot_cred: ([u8; 32], [u8; 32]),
    /// Cold credential for direct spending
    pub cold_cred: Ed25519KeyHash,
    /// Local storage - hash of Merkle Patricia Tree root
    pub store: [u8; 32],
}

/// Intent (virtual order) structure.
/// Corresponds to Intention in validators/witness.ak
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Intent {
    /// Target nonce (index, value)
    pub target_nonce: (i64, i64),
    /// Asset being sold
    pub leaving_asset: AssetClass,
    /// Amount being sold
    pub leaving_amount: u64,
    /// Asset being bought
    pub arriving_asset: AssetClass,
    /// Expected amount to receive (minimum)
    pub expected_arriving_amount: u64,
    /// Fee in lovelace for execution
    pub fee_lovelace: u64,
    /// Operator verification key hash
    pub operator: Ed25519KeyHash,
}

impl Intent {
    /// Compute the digest of this intent for MPT operations.
    /// Must match on-chain computation exactly.
    pub fn digest(&self) -> [u8; 32] {
        use cml_crypto::blake2b256;
        let mut data = Vec::new();
        data.extend(&self.target_nonce.0.to_be_bytes());
        data.extend(&self.target_nonce.1.to_be_bytes());
        data.extend(&self.leaving_asset_bytes());
        data.extend(&self.leaving_amount.to_be_bytes());
        data.extend(&self.arriving_asset_bytes());
        data.extend(&self.expected_arriving_amount.to_be_bytes());
        data.extend(&self.fee_lovelace.to_be_bytes());
        data.extend(self.operator.to_raw_bytes());
        blake2b256(&data)
    }

    fn leaving_asset_bytes(&self) -> Vec<u8> {
        match self.leaving_asset {
            AssetClass::Native => vec![0u8; 56],
            AssetClass::Token(Token(policy, name)) => {
                let mut bytes = Vec::with_capacity(56);
                bytes.extend(policy.to_raw_bytes());
                bytes.extend(name.as_bytes());
                bytes.resize(56, 0);
                bytes
            }
        }
    }

    fn arriving_asset_bytes(&self) -> Vec<u8> {
        match self.arriving_asset {
            AssetClass::Native => vec![0u8; 56],
            AssetClass::Token(Token(policy, name)) => {
                let mut bytes = Vec::with_capacity(56);
                bytes.extend(policy.to_raw_bytes());
                bytes.extend(name.as_bytes());
                bytes.resize(56, 0);
                bytes
            }
        }
    }

    /// Create an updated intent after partial fill
    pub fn with_remainder(&self, remainder: u64, original_leaving: u64) -> Self {
        let ratio = remainder as u128 * 1_000_000 / original_leaving as u128;
        let new_expected = (self.expected_arriving_amount as u128 * ratio / 1_000_000) as u64;
        let new_fee = (self.fee_lovelace as u128 * ratio / 1_000_000) as u64;
        Intent {
            target_nonce: self.target_nonce,
            leaving_asset: self.leaving_asset,
            leaving_amount: remainder,
            arriving_asset: self.arriving_asset,
            expected_arriving_amount: new_expected,
            fee_lovelace: new_fee,
            operator: self.operator,
        }
    }

    /// Calculate price as RelativePrice (output/input)
    pub fn price(&self) -> RelativePrice {
        RelativePrice::new(self.expected_arriving_amount as u128, self.leaving_amount as u128)
    }
}

/// Authorization method for intents
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    /// First execution - signature-authenticated
    Sig {
        signature: Vec<u8>,
        prefix: Vec<u8>,
        postfix: Vec<u8>,
    },
    /// Continuation - MPT proof-authenticated
    Path {
        /// Merkle proof from the account's store
        proof: Vec<u8>,
        /// The original intent digest this continuation is for
        original_digest: [u8; 32],
    },
}

/// Authorized intention with remainder tracking
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedIntention {
    pub intent: Intent,
    /// Remaining amount to fill (equals leaving_amount initially, decreases with partial fills)
    pub remainder: u64,
    pub auth: Auth,
}

impl AuthorizedIntention {
    /// Create new authorized intention from signed intent
    pub fn new_signed(intent: Intent, signature: Vec<u8>, prefix: Vec<u8>, postfix: Vec<u8>) -> Self {
        Self {
            remainder: intent.leaving_amount,
            intent,
            auth: Auth::Sig {
                signature,
                prefix,
                postfix,
            },
        }
    }

    /// Create continuation from existing partial fill
    pub fn new_continuation(intent: Intent, remainder: u64, proof: Vec<u8>, original_digest: [u8; 32]) -> Self {
        Self {
            intent,
            remainder,
            auth: Auth::Path {
                proof,
                original_digest,
            },
        }
    }

    /// Whether this is a first-time execution (Sig auth)
    pub fn is_first_execution(&self) -> bool {
        matches!(self.auth, Auth::Sig { .. })
    }
}

/// Snapshot of MPT state for embedding in GreenOrder.
/// Allows BatchExec to compute new MPT root without accessing AccountIndex.
#[derive(Debug, Clone)]
pub struct MptSnapshot {
    /// Current MPT root hash (from account's store field)
    pub current_root: [u8; 32],
    /// For Path auth: the intent data being continued
    pub pending_intent: Option<(Intent, [u8; 32])>,
}

impl MptSnapshot {
    pub fn empty() -> Self {
        Self {
            current_root: [0u8; 32],
            pending_intent: None,
        }
    }

    pub fn new(current_root: [u8; 32]) -> Self {
        Self {
            current_root,
            pending_intent: None,
        }
    }

    pub fn with_pending(current_root: [u8; 32], intent: Intent, digest: [u8; 32]) -> Self {
        Self {
            current_root,
            pending_intent: Some((intent, digest)),
        }
    }
}

/// Delta to apply to AccountIndex MPT after successful execution
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MptDelta {
    /// First execution partial fill: insert new intent remainder
    Insert {
        account_id: Token,
        intent_digest: [u8; 32],
        updated_intent: Intent,
    },
    /// Continuation partial fill: update existing intent remainder
    Update {
        account_id: Token,
        old_digest: [u8; 32],
        new_digest: [u8; 32],
        updated_intent: Intent,
    },
    /// Continuation complete: remove intent from tree
    Delete {
        account_id: Token,
        intent_digest: [u8; 32],
    },
    /// First execution complete: no MPT change needed
    None,
}

/// Green Order - a virtual order backed by an on-chain Account.
#[derive(Debug, Clone)]
pub struct GreenOrder {
    /// Account identifier (NFT token)
    pub account_id: Token,
    /// The authorized intention with remainder tracking
    pub authorized_intention: AuthorizedIntention,
    /// Account's current UTxO reference
    pub account_utxo: FinalizedTxOut,
    /// Snapshot of MPT state for computing new root
    pub mpt_snapshot: MptSnapshot,
    /// Execution budget (from fee_lovelace, decreases with execution)
    pub execution_budget: u64,
    /// Maximum cost per execution step
    pub max_cost_per_ex_step: u64,
    /// Marginal execution cost hint
    pub marginal_cost: ExUnits,
    /// Minimum output per execution step
    pub min_marginal_output: u64,
}

impl GreenOrder {
    /// Create a new green order from account and authorized intention
    pub fn new(
        account_id: Token,
        authorized_intention: AuthorizedIntention,
        account_utxo: FinalizedTxOut,
        mpt_snapshot: MptSnapshot,
        max_cost_per_ex_step: u64,
        marginal_cost: ExUnits,
        min_marginal_output: u64,
    ) -> Self {
        let execution_budget = authorized_intention.intent.fee_lovelace;
        Self {
            account_id,
            authorized_intention,
            account_utxo,
            mpt_snapshot,
            execution_budget,
            max_cost_per_ex_step,
            marginal_cost,
            min_marginal_output,
        }
    }

    /// Get the underlying intent
    pub fn intent(&self) -> &Intent {
        &self.authorized_intention.intent
    }

    /// Get current remainder
    pub fn remainder(&self) -> u64 {
        self.authorized_intention.remainder
    }

    /// Input asset (what user is selling)
    pub fn input_asset(&self) -> AssetClass {
        self.intent().leaving_asset
    }

    /// Output asset (what user is buying)
    pub fn output_asset(&self) -> AssetClass {
        self.intent().arriving_asset
    }

    /// Accumulated output so far
    pub fn accumulated_output(&self) -> u64 {
        let original = self.intent().leaving_amount;
        let consumed = original - self.remainder();
        // Calculate proportional output based on price
        let price = self.intent().price();
        (consumed as u128 * *price.numer() / *price.denom()) as u64
    }
}

impl Display for GreenOrder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "GreenOrder({}, {}, in={} {}, out={}, budget={}, remainder={})",
            self.account_id,
            self.side(),
            self.remainder(),
            self.input_asset(),
            self.output_asset(),
            self.execution_budget,
            self.remainder()
        )
    }
}

impl PartialEq for GreenOrder {
    fn eq(&self, other: &Self) -> bool {
        self.account_id == other.account_id
            && self.authorized_intention == other.authorized_intention
    }
}

impl Eq for GreenOrder {}

impl PartialOrd for GreenOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GreenOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        let cmp_by_price = self.price().cmp(&other.price());
        let cmp_by_price = if matches!(self.side(), Side::Bid) {
            cmp_by_price.reverse()
        } else {
            cmp_by_price
        };
        cmp_by_price
            .then(self.weight().cmp(&other.weight()))
            .then(self.stable_id().cmp(&other.stable_id()))
    }
}

impl TakerBehaviour for GreenOrder {
    fn with_updated_time(self, _: u64) -> Next<Self, Unit> {
        Next::Succ(self)
    }

    fn with_applied_trade(
        mut self,
        removed_input: InputAsset<u64>,
        added_output: OutputAsset<u64>,
    ) -> Next<Self, TerminalTake> {
        // Update remainder
        self.authorized_intention.remainder = self.authorized_intention.remainder.saturating_sub(removed_input);

        // Calculate proportional fee reduction
        let original_leaving = self.intent().leaving_amount;
        if self.authorized_intention.remainder == 0 {
            // Fully filled
            Next::Term(TerminalTake {
                remaining_input: 0,
                accumulated_output: self.accumulated_output() + added_output,
                remaining_fee: 0,
                remaining_budget: self.execution_budget,
            })
        } else {
            // Partially filled - update the intent with new amounts
            let updated_intent = self.intent().with_remainder(self.authorized_intention.remainder, original_leaving);
            self.authorized_intention.intent = updated_intent;
            Next::Succ(self)
        }
    }

    fn with_budget_corrected(mut self, delta: i64) -> (i64, Self) {
        let budget_remainder = self.execution_budget as i64;
        let corrected_remainder = budget_remainder + delta;
        let updated_budget_remainder = max(corrected_remainder, 0);
        let real_delta = updated_budget_remainder - budget_remainder;
        self.execution_budget = updated_budget_remainder as u64;
        (real_delta, self)
    }

    fn with_fee_charged(mut self, fee: u64) -> Self {
        self.authorized_intention.intent.fee_lovelace =
            self.authorized_intention.intent.fee_lovelace.saturating_sub(fee);
        self
    }

    fn with_output_added(self, _added_output: u64) -> Self {
        // Output is tracked through remainder calculation
        self
    }

    fn try_terminate(self) -> Next<Self, TerminalTake> {
        if self.execution_budget < self.max_cost_per_ex_step {
            Next::Term(TerminalTake {
                remaining_input: self.remainder(),
                accumulated_output: self.accumulated_output(),
                remaining_fee: self.intent().fee_lovelace,
                remaining_budget: self.execution_budget,
            })
        } else {
            Next::Succ(self)
        }
    }
}

impl MarketTaker for GreenOrder {
    type U = ExUnits;

    fn side(&self) -> Side {
        side_of(self.input_asset(), self.output_asset())
    }

    fn input(&self) -> InputAsset<u64> {
        self.remainder()
    }

    fn output(&self) -> OutputAsset<u64> {
        self.accumulated_output()
    }

    fn price(&self) -> AbsolutePrice {
        AbsolutePrice::from_price(self.side(), self.intent().price())
    }

    fn operator_fee(&self, input_consumed: InputAsset<u64>) -> FeeAsset<u64> {
        let total_fee = self.intent().fee_lovelace;
        let total_input = self.remainder();
        if total_input > 0 {
            total_fee.saturating_mul(input_consumed) / total_input
        } else {
            0
        }
    }

    fn fee(&self) -> FeeAsset<u64> {
        self.intent().fee_lovelace
    }

    fn budget(&self) -> FeeAsset<u64> {
        self.execution_budget
    }

    fn consumable_budget(&self) -> FeeAsset<u64> {
        self.max_cost_per_ex_step
    }

    fn marginal_cost_hint(&self) -> ExUnits {
        self.marginal_cost
    }

    fn min_marginal_output(&self) -> OutputAsset<u64> {
        self.min_marginal_output
    }

    fn time_bounds(&self) -> TimeBounds<u64> {
        TimeBounds::None
    }
}

impl Stable for GreenOrder {
    type StableId = Token;

    fn stable_id(&self) -> Self::StableId {
        self.account_id
    }

    fn is_quasi_permanent(&self) -> bool {
        false
    }
}

impl Tradable for GreenOrder {
    type PairId = PairId;

    fn pair_id(&self) -> Self::PairId {
        PairId::canonical(self.input_asset(), self.output_asset())
    }
}


// ============================================================================
// Account TryFromLedger implementation
// ============================================================================

/// Protocol validator variant for Account
pub const ACCOUNT_V1: u8 = 100; // Placeholder - should be added to ProtocolValidator enum

impl TryFromPData for Account {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let cpd = data.into_constr_pd()?;
        if cpd.alternative != 0 {
            return None;
        }
        let mut fields = cpd.fields.into_iter();

        // magic
        let magic = fields.next()?.into_bytes()?;

        // allowlist - List<ScriptHash>
        let allowlist_data = fields.next()?.into_vec()?;
        let allowlist: Vec<[u8; 28]> = allowlist_data
            .into_iter()
            .filter_map(|pd| {
                let bytes = pd.into_bytes()?;
                if bytes.len() == 28 {
                    let mut arr = [0u8; 28];
                    arr.copy_from_slice(&bytes);
                    Some(arr)
                } else {
                    None
                }
            })
            .collect();

        // nonce - List<Int>
        let nonce_data = fields.next()?.into_vec()?;
        let nonce: Vec<i64> = nonce_data
            .into_iter()
            .filter_map(|pd| pd.into_i128().and_then(|v| i64::try_from(v).ok()))
            .collect();

        // hot_cred - (VerificationKey, VerificationKey)
        let hot_cred_data = fields.next()?.into_constr_pd()?;
        let mut hot_fields = hot_cred_data.fields.into_iter();
        let hot_key1_bytes = hot_fields.next()?.into_bytes()?;
        let hot_key2_bytes = hot_fields.next()?.into_bytes()?;
        let mut hot_key1 = [0u8; 32];
        let mut hot_key2 = [0u8; 32];
        if hot_key1_bytes.len() == 32 && hot_key2_bytes.len() == 32 {
            hot_key1.copy_from_slice(&hot_key1_bytes);
            hot_key2.copy_from_slice(&hot_key2_bytes);
        } else {
            return None;
        }

        // cold_cred - VerificationKeyHash
        let cold_cred_bytes = fields.next()?.into_bytes()?;
        let cold_cred = Ed25519KeyHash::from_raw_bytes(&cold_cred_bytes).ok()?;

        // store - ByteArray (32 bytes - MPT root hash)
        let store_bytes = fields.next()?.into_bytes()?;
        let mut store = [0u8; 32];
        if store_bytes.len() == 32 {
            store.copy_from_slice(&store_bytes);
        } else if store_bytes.is_empty() {
            // Empty store means null hash
            store = [0u8; 32];
        } else {
            return None;
        }

        Some(Account {
            magic,
            allowlist,
            nonce,
            hot_cred: (hot_key1, hot_key2),
            cold_cred,
            store,
        })
    }
}

// ============================================================================
// BatchExec for GreenOrder is implemented in execution_engine/instances.rs
// ============================================================================

use cml_chain::plutus::ConstrPlutusData;

/// Delegate redeemer for account script: Delegate(witness_index)
/// witness_index is the index in the transaction's reference inputs where the witness script is
pub fn delegate_redeemer(witness_index: u64) -> PlutusData {
    PlutusData::ConstrPlutusData(ConstrPlutusData {
        alternative: 1, // Delegate constructor
        fields: vec![PlutusData::Integer(witness_index.into())],
        encodings: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intent_with_remainder() {
        let intent = Intent {
            target_nonce: (0, 1),
            leaving_asset: AssetClass::Native,
            leaving_amount: 1000,
            arriving_asset: AssetClass::Native,
            expected_arriving_amount: 500,
            fee_lovelace: 100,
            operator: Ed25519KeyHash::from_raw_bytes(&[0u8; 28]).unwrap(),
        };

        // 50% filled, 500 remainder
        let updated = intent.with_remainder(500, 1000);
        assert_eq!(updated.leaving_amount, 500);
        assert_eq!(updated.expected_arriving_amount, 250);
        assert_eq!(updated.fee_lovelace, 50);
    }

    #[test]
    fn test_intent_digest_deterministic() {
        let intent = Intent {
            target_nonce: (0, 1),
            leaving_asset: AssetClass::Native,
            leaving_amount: 1000,
            arriving_asset: AssetClass::Native,
            expected_arriving_amount: 500,
            fee_lovelace: 100,
            operator: Ed25519KeyHash::from_raw_bytes(&[0u8; 28]).unwrap(),
        };

        let digest1 = intent.digest();
        let digest2 = intent.digest();
        assert_eq!(digest1, digest2);
    }
}
