use cml_chain::{
    assets::AssetName,
    certs::StakeCredential,
    plutus::{ConstrPlutusData, PlutusData},
    utils::BigInteger,
    PolicyId,
};
use cml_crypto::RawBytesEncoding;
use spectrum_cardano_lib::plutus_data::make_constr_pd_indefinite_arr;

pub fn make_redeem_ve_witness_redeemer(
    stake_credential: Option<StakeCredential>,
    voting_escrow_input_ix: u32,
    ve_factory_input_ix: u32,
    (ve_ident_policy_id, ve_ident_name): (PolicyId, AssetName),
    (ve_factory_policy_id, ve_factory_name): (PolicyId, AssetName),
    splash_token_policy: PolicyId,
    mint_composition_token_policy: PolicyId,
) -> PlutusData {
    let stake_cred_pd = if let Some(sc) = stake_credential {
        make_constr_pd_indefinite_arr(vec![make_constr_pd_indefinite_arr(vec![
            make_constr_pd_indefinite_arr(vec![PlutusData::new_bytes(sc.to_raw_bytes().to_vec())]),
        ])])
    } else {
        PlutusData::new_constr_plutus_data(ConstrPlutusData::new(1, vec![]))
    };
    let ve_ix = PlutusData::new_integer(BigInteger::from(voting_escrow_input_ix));
    let ve_fac_ix = PlutusData::new_integer(BigInteger::from(ve_factory_input_ix));
    let ve_ident_asset_pd = make_constr_pd_indefinite_arr(vec![
        PlutusData::new_bytes(ve_ident_policy_id.to_raw_bytes().to_vec()),
        PlutusData::new_bytes(ve_ident_name.to_raw_bytes().to_vec()),
    ]);
    let ve_factory_asset_pd = make_constr_pd_indefinite_arr(vec![
        PlutusData::new_bytes(ve_factory_policy_id.to_raw_bytes().to_vec()),
        PlutusData::new_bytes(ve_factory_name.to_raw_bytes().to_vec()),
    ]);
    let splash_token_policy_pd = PlutusData::new_bytes(splash_token_policy.to_raw_bytes().to_vec());
    let mint_composition_token_policy_pd =
        PlutusData::new_bytes(mint_composition_token_policy.to_raw_bytes().to_vec());
    make_constr_pd_indefinite_arr(vec![
        stake_cred_pd,
        ve_ix,
        ve_fac_ix,
        ve_ident_asset_pd,
        ve_factory_asset_pd,
        splash_token_policy_pd,
        mint_composition_token_policy_pd,
    ])
}
