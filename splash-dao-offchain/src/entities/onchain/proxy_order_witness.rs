use cml_chain::plutus::utils::ConstrPlutusDataEncoding;
use cml_chain::plutus::ConstrPlutusData;
use cml_chain::utils::BigInteger;
use cml_chain::LenEncoding;
use cml_chain::{certs::StakeCredential, plutus::PlutusData};
use cml_crypto::RawBytesEncoding;
use cml_crypto::ScriptHash;
use spectrum_cardano_lib::{
    plutus_data::{make_constr_pd_indefinite_arr, IntoPlutusData},
    OutputRef,
};

pub struct WitnessAction {
    pub proxy_order_input_ix: u32,
    pub proxy_order_output_reference: OutputRef,
    pub proxy_order_script_hash: ScriptHash,
    /// Expected redeemer that is applied to the proxy order
    pub proxy_order_redeemer: PlutusData,
    pub proxy_order_datum: PlutusData,
    /// Following field is Some(_) iff we're witnessing redeem VE TX.
    pub owner_redemption: Option<OwnerRedemptionUTxO>,
}

/// Contains infomation necessary to complete redemption of an owner's voting escrow.
pub struct OwnerRedemptionUTxO {
    /// Output index of owner's UTxO in redeem TX.
    pub owner_output_ix: u32,
    /// Owner's stake credential. Needed to properly address returned assets to owner.
    pub owner_stake_credential: StakeCredential,
}

impl IntoPlutusData for WitnessAction {
    fn into_pd(self) -> PlutusData {
        let WitnessAction {
            proxy_order_input_ix,
            proxy_order_output_reference,
            proxy_order_script_hash,
            proxy_order_redeemer,
            proxy_order_datum,
            owner_redemption,
        } = self;

        // Note: witness is a Plutus V3 script, so `OutputReference` has a different representation.
        let order_input_ix_pd = PlutusData::new_integer(BigInteger::from(proxy_order_input_ix));
        let tx_hash_pd =
            PlutusData::new_bytes(proxy_order_output_reference.tx_hash().to_raw_bytes().to_vec());
        let output_ref_ix =
            PlutusData::new_integer(BigInteger::from(self.proxy_order_output_reference.index()));
        let output_ref_pd = make_constr_pd_indefinite_arr(vec![tx_hash_pd, output_ref_ix]);
        let script_hash_pd = PlutusData::new_bytes(proxy_order_script_hash.to_raw_bytes().to_vec());

        let owner_redemption_pd = if let Some(owner_redemption) = owner_redemption {
            let OwnerRedemptionUTxO {
                owner_output_ix,
                owner_stake_credential,
            } = owner_redemption;
            let owner_output_ix_pd = PlutusData::new_integer(BigInteger::from(owner_output_ix));
            let stake_cred_bytes = PlutusData::new_bytes(owner_stake_credential.to_raw_bytes().to_vec());
            let stake_cred =
                make_constr_pd_indefinite_arr(vec![make_constr_pd_indefinite_arr(vec![stake_cred_bytes])]);
            make_constr_pd_indefinite_arr(vec![make_constr_pd_indefinite_arr(vec![
                owner_output_ix_pd,
                stake_cred,
            ])])
        } else {
            PlutusData::new_constr_plutus_data(ConstrPlutusData::new(1, vec![]))
        };

        make_constr_pd_indefinite_arr(vec![
            order_input_ix_pd,
            output_ref_pd,
            script_hash_pd,
            proxy_order_redeemer,
            proxy_order_datum,
            owner_redemption_pd,
        ])
    }
}
