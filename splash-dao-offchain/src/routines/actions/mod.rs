mod inflation;
mod voting_escrow;
mod wpoll;

use cml_chain::address::{Address, EnterpriseAddress};
use cml_chain::builders::input_builder::InputBuilderResult;
use cml_chain::builders::mint_builder::MintBuilderResult;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{SignedTxBuilder, TransactionBuilder, TransactionUnspentOutput};
use cml_chain::builders::withdrawal_builder::WithdrawalBuilderResult;
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::{ExUnits, RedeemerTag};
use cml_chain::transaction::TransactionInput;
use cml_chain::utils::BigInteger;
use cml_chain::Coin;
use cml_crypto::{blake2b256, RawBytesEncoding, ScriptHash};
use serde::Serialize;
use spectrum_offchain::domain::event::{Predicted, Traced};

use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::IntoLedger;
use uplc::PlutusData;
use uplc_pallas_primitives::Fragment;

use crate::collect_utxos::collect_tagged_utxos;
use crate::constants::time::MAX_TIME_DRIFT_MILLIS;
use crate::create_change_output::{ChangeOutputCreator, CreateChangeOutput};
use crate::deployment::IssuedAsset;
use crate::entities::offchain::{
    ExtendVotingEscrowOffChainOrder, RedeemVotingEscrowOffChainOrder, WPollVoteOffChainOrder,
};
use crate::entities::onchain::extend_voting_escrow_order::ExtendVotingEscrowOrderBundle;
use crate::entities::onchain::funding_box::FundingBox;
use crate::entities::onchain::make_voting_escrow_order::MakeVotingEscrowOrderBundle;
use crate::entities::onchain::redeem_voting_escrow::RedeemVotingEscrowOrderBundle;
use crate::entities::onchain::voting_escrow_factory::VEFactorySnapshot;
use crate::entities::onchain::wpoll_vote_order::{WPollVoteOnchainOrder, WPollVoteOrderBundle};
use crate::funding::AvailableFundingBoxes;
use crate::protocol_config::OperatorCreds;

use super::{
    FundingBoxChanges, InflationBoxSnapshot, PermManagerSnapshot, PollFactorySnapshot, Slot,
    SmartFarmSnapshot, VotingEscrowSnapshot, WeightingPollSnapshot,
};

#[async_trait::async_trait]
pub trait InflationActions<Bearer> {
    async fn distribute_inflation(
        &self,
        weighting_poll: Bundled<WeightingPollSnapshot, Bearer>,
        farm: Bundled<SmartFarmSnapshot, Bearer>,
        perm_manager: Bundled<PermManagerSnapshot, Bearer>,
        current_slot: Slot,
        farm_weight: u64,
        funding_boxes: AvailableFundingBoxes,
    ) -> (
        SignedTxBuilder,
        Traced<Predicted<Bundled<WeightingPollSnapshot, Bearer>>>,
        Traced<Predicted<Bundled<SmartFarmSnapshot, Bearer>>>,
        FundingBoxChanges,
    );
}

#[async_trait::async_trait]
pub trait WPollActions<Bearer> {
    async fn create_wpoll(
        &self,
        inflation_box: Bundled<InflationBoxSnapshot, Bearer>,
        factory: Bundled<PollFactorySnapshot, Bearer>,
        current_slot: Slot,
        funding_boxes: AvailableFundingBoxes,
    ) -> (
        SignedTxBuilder,
        Traced<Predicted<Bundled<InflationBoxSnapshot, Bearer>>>,
        Traced<Predicted<Bundled<PollFactorySnapshot, Bearer>>>,
        Traced<Predicted<Bundled<WeightingPollSnapshot, Bearer>>>,
        FundingBoxChanges,
    );

    async fn eliminate_wpoll(
        &self,
        weighting_poll: Bundled<WeightingPollSnapshot, Bearer>,
        funding_boxes: AvailableFundingBoxes,
        current_slot: Slot,
    ) -> (SignedTxBuilder, FundingBoxChanges);

    async fn execute_order(
        &self,
        weighting_poll: Bundled<WeightingPollSnapshot, Bearer>,
        voting_escrow: Bundled<VotingEscrowSnapshot, Bearer>,
        onchain_order: WPollVoteOrderBundle<Bearer>,
        offchain_order: WPollVoteOffChainOrder,
        current_slot: Slot,
    ) -> Result<
        (
            SignedTxBuilder,
            Traced<Predicted<Bundled<WeightingPollSnapshot, Bearer>>>,
            Traced<Predicted<Bundled<VotingEscrowSnapshot, Bearer>>>,
        ),
        ExecuteOrderError,
    >;
}

#[async_trait::async_trait]
pub trait VoteEscrowActions<Bearer> {
    async fn make_voting_escrow(
        &self,
        make_voting_escrow_order: MakeVotingEscrowOrderBundle<Bearer>,
        ve_factory: Bundled<VEFactorySnapshot, Bearer>,
        current_slot: Slot,
    ) -> Result<
        (
            SignedTxBuilder,
            Traced<Predicted<Bundled<VEFactorySnapshot, Bearer>>>,
            Traced<Predicted<Bundled<VotingEscrowSnapshot, Bearer>>>,
        ),
        MakeVotingEscrowError,
    >;

    async fn extend_voting_escrow(
        &self,
        extend_voting_escrow_onchain_order: ExtendVotingEscrowOrderBundle<Bearer>,
        extend_voting_escrow_offchain_order: ExtendVotingEscrowOffChainOrder,
        voting_escrow: Bundled<VotingEscrowSnapshot, Bearer>,
        ve_factory: Bundled<VEFactorySnapshot, Bearer>,
        current_slot: Slot,
    ) -> Result<
        (
            SignedTxBuilder,
            Traced<Predicted<Bundled<VEFactorySnapshot, Bearer>>>,
            Traced<Predicted<Bundled<VotingEscrowSnapshot, Bearer>>>,
        ),
        ExtendVotingEscrowError,
    >;

    async fn redeem_voting_escrow(
        &self,
        onchain_order: RedeemVotingEscrowOrderBundle<Bearer>,
        offchain_order: RedeemVotingEscrowOffChainOrder,
        voting_escrow: Bundled<VotingEscrowSnapshot, Bearer>,
        ve_factory: Bundled<VEFactorySnapshot, Bearer>,
        current_slot: Slot,
    ) -> Result<
        (
            SignedTxBuilder,
            Traced<Predicted<Bundled<VEFactorySnapshot, Bearer>>>,
        ),
        RedeemVotingEscrowError,
    >;
}

/// 1/5 of MAX_TIME_DRIFT
const TX_TTL_SLOT: u64 = MAX_TIME_DRIFT_MILLIS / 1000 / 5;

#[derive(derive_more::From)]
pub struct CardanoInflationActions<Ctx> {
    ctx: Ctx,
}

pub(crate) fn compute_identifier_token_asset_name(output_ref: OutputRef) -> cml_chain::assets::AssetName {
    use cml_chain::Serialize;
    let mut bytes = output_ref.tx_hash().to_raw_bytes().to_vec();
    bytes.extend_from_slice(
        &cml_chain::plutus::PlutusData::new_integer(BigInteger::from(output_ref.index())).to_cbor_bytes(),
    );
    let token_name = blake2b256(bytes.as_ref());
    cml_chain::assets::AssetName::new(token_name.to_vec()).unwrap()
}

pub(crate) fn script_address(script_hash: ScriptHash, network_id: NetworkId) -> Address {
    EnterpriseAddress::new(u8::from(network_id), StakeCredential::new_script(script_hash)).to_address()
}

/// Here we calculate `cbor.serialise(i)` from Aiken script. The exact calculation that is
/// performed is found here: https://github.com/aiken-lang/aiken/blob/2bb2f11090ace3c7f36ed75b0e1d5b101d0c9a8a/crates/uplc/src/machine/runtime.rs#L1032
fn cbor_serialise_integer(i: u32) -> Vec<u8> {
    let i = uplc_pallas_codec::utils::Int::from(i as i64);

    PlutusData::BigInt(uplc_pallas_primitives::alonzo::BigInt::Int(i))
        .encode_fragment()
        .unwrap()
}

/// Computes index_tn(epoch) from aiken script
pub fn compute_epoch_asset_name(epoch: u32) -> cml_chain::assets::AssetName {
    let bytes = cbor_serialise_integer(epoch);

    let token_name = blake2b256(bytes.as_ref());
    cml_chain::assets::AssetName::new(token_name.to_vec()).unwrap()
}

/// Computes farm_name(farm_id: Int) from aiken script
pub fn compute_farm_name(farm_id: u32) -> cml_chain::assets::AssetName {
    let bytes = cbor_serialise_integer(farm_id);

    cml_chain::assets::AssetName::try_from(bytes).unwrap()
}

pub fn select_funding_boxes<Ctx>(
    target: Coin,
    required_tokens: Vec<IssuedAsset>,
    AvailableFundingBoxes { confirmed, predicted }: AvailableFundingBoxes,
    ctx: &Ctx,
) -> (Vec<InputBuilderResult>, AvailableFundingBoxes)
where
    Ctx: Has<OperatorCreds> + Clone,
{
    #[derive(Clone, Copy)]
    enum T {
        Predicted,
        Confirmed,
    }

    let boxes: Vec<_> = confirmed
        .into_iter()
        .map(|f| (T::Confirmed, f))
        .chain(predicted.into_iter().map(|f| (T::Predicted, f)))
        .collect();
    let mut all_utxos = vec![];
    for (t, funding_box) in &boxes {
        let output = funding_box.clone().into_ledger(ctx.clone());
        let output_ref: OutputRef = funding_box.id.into();
        let input = TransactionInput::from(output_ref);
        all_utxos.push((t, TransactionUnspentOutput::new(input, output)));
    }
    let input_results = collect_tagged_utxos(all_utxos, target, required_tokens, None);

    let mut predicted = vec![];
    let mut confirmed = vec![];

    for (t, i) in &input_results {
        let output_ref = OutputRef::new(i.input.transaction_id, i.input.index);
        let id = boxes
            .iter()
            .find_map(|(_, f)| {
                let funding_output_ref: OutputRef = f.id.into();
                if funding_output_ref == output_ref {
                    Some(f.id)
                } else {
                    None
                }
            })
            .unwrap();
        let funding_box = FundingBox {
            value: i.utxo_info.value().clone(),
            id,
        };
        match t {
            T::Predicted => {
                predicted.push(funding_box);
            }
            T::Confirmed => {
                confirmed.push(funding_box);
            }
        }
    }
    let inputs = input_results.into_iter().map(|(_, i)| i).collect();
    (inputs, AvailableFundingBoxes { predicted, confirmed })
}

#[derive(Clone, Debug, Serialize)]
pub enum ExecuteOrderError {
    BadOrMissingInput,
    WeightingExceedsAvailableVotingPower {
        order_weighting_power: u64,
        voting_escrow_weighting_power: u64,
    },
    InVotingPower,
    Witness(WitnessError),
    Other(String),
}

#[derive(Clone, Debug, Serialize)]
pub enum MakeVotingEscrowError {
    NonAcceptedAsset,
    InsufficientAdaInOrder,
    VEFactoryDatumNotPresent,
    LocktimeExceedsLimit,
    Other(String),
}

#[derive(Clone, Debug, Serialize)]
pub enum ExtendVotingEscrowError {
    NonAcceptedAsset,
    InsufficientAdaInOrder,
    VEFactoryDatumNotPresent,
    LocktimeExceedsLimit,
    Witness(WitnessError),
    Other(String),
}

#[derive(Clone, Debug, Serialize)]
pub enum RedeemVotingEscrowError {
    InsufficientAdaInOrder,
    VEFactoryDatumNotPresent,
    VEStillLocked,
    OwnerStakeCredentialMissingInRedeemer,
    Witness(WitnessError),
    Other(String),
}

#[derive(Clone, Debug, Serialize)]
pub enum WitnessError {
    OwnerAuthFailure,
    VotingEscrowIneligibleToVote {
        last_wp_epoch: i32,
        current_epoch: i32,
    },
    VEVersionMismatchWithOffchainOrder {
        voting_escrow_input_version: u32,
        order_version: u32,
    },
    VEVersionMismatchWithOnchainProxy {
        voting_escrow_output_version: u32,
        proxy_version: u32,
    },
    CannotDecodeRedeemer,
}

pub(crate) struct DaoTxBlueprint {
    reference_inputs: Vec<TransactionUnspentOutput>,
    sorted_inputs: Vec<(InputBuilderResult, ExUnits)>,
    outputs: Vec<SingleOutputBuilderResult>,
    sorted_mints: Vec<(
        MintBuilderResult,
        crate::create_change_output::Token,
        bool,
        ExUnits,
    )>,
    withdrawal: Option<(WithdrawalBuilderResult, ExUnits)>,
    fee_buffer: u64,
    operator_address: Address,
}

pub(crate) struct BlueprintEstimates {
    estimated_fee: u64,
    change_output: SingleOutputBuilderResult,
    tx_builder: TransactionBuilder,
}

impl DaoTxBlueprint {
    fn compute_estimated_fee_and_change_output(&self) -> BlueprintEstimates {
        let mut txb = constant_tx_builder();
        let mut change_output_creator = ChangeOutputCreator::default();

        for ref_input in &self.reference_inputs {
            txb.add_reference_input(ref_input.clone());
        }

        for (ix, (input, ex_units)) in self.sorted_inputs.iter().enumerate() {
            change_output_creator.add_input(input);
            txb.add_input(input.clone()).unwrap();
            txb.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, ix as u64),
                ex_units.clone(),
            );
        }

        for (ix, (mint, minted_token, is_mint, ex_units)) in self.sorted_mints.iter().enumerate() {
            if *is_mint {
                change_output_creator.mint_token(minted_token.clone());
            } else {
                change_output_creator.burn_token(minted_token.clone());
            }
            txb.add_mint(mint.clone()).unwrap();
            txb.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Mint, ix as u64),
                ex_units.clone(),
            );
        }

        for output in &self.outputs {
            change_output_creator.add_output(output);
            txb.add_output(output.clone()).unwrap();
        }

        if let Some((withdrawal, ex_units)) = &self.withdrawal {
            txb.add_withdrawal(withdrawal.clone());
            txb.set_exunits(RedeemerWitnessKey::new(RedeemerTag::Reward, 0), ex_units.clone());
        }

        let estimated_fee = txb.min_fee(true).unwrap() + self.fee_buffer;
        let change_output =
            change_output_creator.create_change_output(estimated_fee, self.operator_address.clone());

        BlueprintEstimates {
            estimated_fee,
            change_output,
            tx_builder: txb,
        }
    }

    fn build(&self, actual_fee: u64, change_output: Option<SingleOutputBuilderResult>) -> TransactionBuilder {
        let mut txb = constant_tx_builder();
        let mut change_output_creator = ChangeOutputCreator::default();

        for ref_input in &self.reference_inputs {
            txb.add_reference_input(ref_input.clone());
        }

        for (ix, (input, ex_units)) in self.sorted_inputs.iter().enumerate() {
            change_output_creator.add_input(input);
            txb.add_input(input.clone()).unwrap();
            txb.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, ix as u64),
                ex_units.clone(),
            );
        }

        for (ix, (mint, minted_token, is_mint, ex_units)) in self.sorted_mints.iter().enumerate() {
            if *is_mint {
                change_output_creator.mint_token(minted_token.clone());
            } else {
                change_output_creator.burn_token(minted_token.clone());
            }
            txb.add_mint(mint.clone()).unwrap();
            txb.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Mint, ix as u64),
                ex_units.clone(),
            );
        }

        for output in &self.outputs {
            change_output_creator.add_output(output);
            txb.add_output(output.clone()).unwrap();
        }

        if let Some((withdrawal, ex_units)) = &self.withdrawal {
            txb.add_withdrawal(withdrawal.clone());
            txb.set_exunits(RedeemerWitnessKey::new(RedeemerTag::Reward, 0), ex_units.clone());
        }

        if let Some(change_output) = change_output {
            txb.add_output(change_output).unwrap();
        }
        txb.set_fee(actual_fee);
        txb
    }
}

#[cfg(test)]
mod tests {
    use cml_chain::{
        address::{BaseAddress, EnterpriseAddress},
        certs::{Credential, StakeCredential},
        transaction::{DatumOption, Transaction},
        Deserialize, Serialize,
    };
    use cml_crypto::{Bip32PrivateKey, ScriptHash};

    use crate::entities::onchain::weighting_poll::compute_mint_wp_auth_token_validator;

    #[test]
    fn test_parametrised_validator() {
        let splash_policy = create_dummy_policy_id(0);
        let farm_auth_policy = create_dummy_policy_id(1);
        let factory_auth_policy = create_dummy_policy_id(2);
        let inflation_box_auth_policy = create_dummy_policy_id(3);
        let zeroth_epoch_start = 100;
        let _ = compute_mint_wp_auth_token_validator(
            splash_policy,
            farm_auth_policy,
            factory_auth_policy,
            inflation_box_auth_policy,
            zeroth_epoch_start,
        );
    }

    #[test]
    fn test_blaze_deployment() {
        let tx_hex = "84a8008282582024da0b6a8037df5b61747b08c061b802054a86e260c2558b0766144cb75b8b6f0182582024da0b6a8037df5b61747b08c061b802054a86e260c2558b0766144cb75b8b6f070183a300581d7038c1745a6f8a6921427f160be2d73bc77d91a7c7703ab2afd29788b701821a02faf080a1581cc72c38ca9933f1fc4b614d1930113a4fd02f3448cd283ed66e189209a142613401028201d818582dd8799f1b00238d7ea4c68000581e581c7bf3980a45756eabfb799fd1998f633176f6d2a2e34de887ddb4e8dbffa300581d7043d7d73cb48e5d7b481ac9197dcb643a941c7685e5ae751eaa96b11601821a00989680a1581c43d7d73cb48e5d7b481ac9197dcb643a941c7685e5ae751eaa96b116a1491b00238d7ea4c6800001028201d818581e581c7bf3980a45756eabfb799fd1998f633176f6d2a2e34de887ddb4e8db825839002e8ec2b01750544eb86070b3085acef5d6983b4d6e1f5cf26e3ec0b84d3464d094658b76fa83cb87317c43440a16e2376b0a826673413daf1b0000000195da142c021a0004d06909a1581c43d7d73cb48e5d7b481ac9197dcb643a941c7685e5ae751eaa96b116a1491b00238d7ea4c68000010b5820fde88b63cd7a03131fac6aab2270fcbef844f274b4884b0b11b53689a17b5d160d8182582024da0b6a8037df5b61747b08c061b802054a86e260c2558b0766144cb75b8b6f0710825839002e8ec2b01750544eb86070b3085acef5d6983b4d6e1f5cf26e3ec0b84d3464d094658b76fa83cb87317c43440a16e2376b0a826673413daf1b00000001967a1ad6128182582059f16714bec4899e2041493f0aa042b2f48f90fc98d1daf9c0e6d900b6989e5c02a3008182582000000000000000000000000000000000000000000000000000000000000000005840000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000582840000d87980821a00d59f801b00000002540be400840100d8799f01ff821a00d59f801b00000002540be4000681590b98590b9501000033323232323232322322322253232323233300b3002300c375400a264a666018646464a66601e600c60206ea80044c8c8c8c8c94ccc050c02cc054dd5007899191919299980c1807980c9baa00113232323232533301d3016301e375400226464646464646464a66604a6038604c6ea80044c8c8c8c94ccc0a4c088c0a8dd500089929998150080a9998150038a9998150020a99981500188008a5014a0294052819b8f300a302e302b37540026eb8c040c0acdd50060b180698151baa0033375e602660526ea8008c07ccc0accdd2a4004660566ea40612f5c06605698103d87a80004bd7019baf374c6602c6eacc034c0a0dd5000a4500374ca66604c603890000a5eb7bdb1804c8c8cc0040052f5bded8c044a66605800226605a66ec0dd480d1ba60034bd6f7b630099191919299981699baf3300c01e0024c0103d8798000133031337606ea4078dd30038028a99981699b8f01e002133031337606ea4078dd300380189981899bb037520046e98004cc01801800cdd598170019bae302c0023030002302e00133330044bd6f7b6300032400400a6054604e6ea800458cc038dd6180498131baa01748008cdd79ba633010012014374c666600297adef6c60003480080088888c8cc004004014894ccc0ac0044cc0b0cdd81ba9005375000897adef6c60132323232533302c3375e6600e01200498103d8798000133030337606ea4024dd40040028a99981619b8f009002133030337606ea4024dd400400189981819bb037520046ea0004cc01801800cdd698168019bae302b002302f002302d00122533302333720004002298103d8798000153330233371e0040022980103d87a800014c103d87b80003001375066e00dd6980618111baa003480088dd9800a99980f180a980f9baa00113232323253330253028002149858dd7181300098130011bad3024001302037540022c6044603e6ea800458c004c078dd51801980f1baa0042302130223022001301032533301b3011301c37540022900009bad3020301d375400264a666036602260386ea80045300103d87a80001323300100137566042603c6ea8008894ccc080004530103d87a8000132323253330203371e91101a400375c604200626030660486ea00052f5c026600a00a0046eb4c084008c090008c088004cc020dd59800980e1baa3001301c375400402c4603e6040002603a60346ea800458cc004dd61801980c9baa00a375a603860326ea8048c0040048894ccc06c0085300103d87a800013232533301a3011003130123301e0024bd70099980280280099b8000348004c07c00cc0740088c0680044c8c8cc004004cc008008cc00c01401c894ccc068004528899299980c19b88375a603a00490000998018018008a50301d00122533301900114bd7009980d180c180d80099801001180e0009119299980b1806180b9baa00114bd6f7b63009bab301b301837540026600600400244646600200200644a666032002298103d87a8000132323253330193371e00c6eb8c06800c4c044cc074dd3000a5eb804cc014014008dd5980d001180e801180d800998009bab30163017301730173017301337540089110022323300100100322533301700114bd6f7b630099191919299980c19b8f007002100313301c337606ea4008dd3000998030030019bab3019003375c602e004603600460320026eb8c050c044dd50008b1809980a001180900098071baa00614984d958c94ccc030c00c0044c8c94ccc044c05000852616375a6024002601c6ea801c54ccc030c00800454ccc03cc038dd50038a4c2c2c60186ea80184cc88c894ccc03cc8c8c94ccc048c020c04cdd5000899191919299980b1806980b9baa0011323232323232533301c3013301d375400226464a66603c602e603e6ea80044c8c8c8c94ccc088cdc780b9bae30273024375400a2a666044008200229405281919192999812180d98129baa00313232533302633300230010070064a226660046466002002600400e44a666056002297ae013232533302a33302a3371e6eb8c06000922011cececc92aeaaac1f5b665f567b01baec8bc2771804b4c21716a87a4e3004a09444cc0b8dd38011980200200089980200200098178011bac302d0010074a229408c8cc004004008894ccc0ac00452f5c0264666444646600200200644a6660620022006264660666e9ccc0ccdd4803198199ba9375c6060002660666ea0dd69818800a5eb80cc00c00cc0d4008c0cc004dd718150009bab302b00133003003302f002302d00122232333001001004002222533302d0021001132323232323330080083035007533302f0061337120026660180140080042940dd69819981a0011bae30320013032002375c60600026eb0c0bc0084c8c94ccc098c074c09cdd500089919192999814981118151baa001132323232533302d3023302e375400226464a666064606a0042646464a66606401c2a6660640162a666064004200229405280a5032330010013758606e6070607060706070607060706070607060686ea8088894ccc0d8004528099299981a198028049bae303900214a22660060060026072002666060660026eb0c068c0c8dd50038012504a244646600200200644a66606c00229404c94ccc0d0cdc79bae303900200414a226600600600260720022c6eb8c0cc004c8cc004004c94ccc0bcc094c0c0dd50008a5eb7bdb1804dd5981a18189baa0013300c00f375c606660606ea8008894ccc0c800452f5c02660666060606800266004004606a0022c6034605c6ea8050dd6180c98169baa00232533302b3022302c3754004264646464a666064606a00426464931980300111bae001330050032375c0022c6eb0c0cc004c0cc008dd6181880098169baa0021622323300100100322533303100114984c8cc00c00cc0d4008c00cc0cc004c0b8c0acdd50008b180a98151baa002301d3330043756602260526ea8004071220101a400301030283754605660506ea800458cc02cdd6180798139baa015375a6054604e6ea8010c8cdd79ba633001006023374c660026eacc03cc09cdd50048119119198008008019129998158008a5eb7bdb1804c8c8c8c94ccc0b0cdc7803801080189981819bb037520046e98004cc01801800cdd598168019bae302b002302f002302d001222325333027301d302837540022900009bad302c3029375400264a66604e603a60506ea8004530103d87a8000132330010013756605a60546ea8008894ccc0b0004530103d87a80001323232533302c3371e00e6eb8c0b400c4c090cc0c0dd4000a5eb804cc014014008dd6981680118180011817000998020018011119198008008019129998148008a60103d87a8000132323253330293371e00c6eb8c0a800c4c084cc0b4dd3000a5eb804cc014014008dd5981500118168011815800980598119baa0153756601460446ea8010dd5980498109baa0083375e00c601860406ea800858c028c07cdd50009810980f1baa0011633001007375a6012603a6ea803cc0040048894ccc07c008530103d87a800013232533301e301500313016330220024bd70099980280280099b8000348004c08c00cc084008c018c068dd50009800980c9baa301c3019375400446038603a0022c6644646600200200644a6660380022980103d87a800013232533301b3375e6012603a6ea80080144c04ccc07c0092f5c02660080080026040004603c0026eb0c00cc05cdd5002980d180b9baa00437586002602c6ea80108c064c068c0680048c06000458c058c05c008c054004c044dd50008a4c26caca66601a6008601c6ea80044c8c8c8c94ccc050c05c0084c926325333012300900115333015301437540042930b0a999809180400089919299980b980d0010a4c2c6eb4c060004c050dd50010b18091baa0011630150013015002375a6026002601e6ea800458dd7003180818069baa005370e90011b8748000dd2a40006e1d2004375c0026eb80055cd2ab9d5573caae7d5d02ba157449811e581c65bb79f9ec437c70413430b7aba049d891b5483a0041deaae98758dd004c011e581cc72c38ca9933f1fc4b614d1930113a4fd02f3448cd283ed66e1892090001f5f6";
        let tx = Transaction::from_cbor_bytes(&hex::decode(tx_hex).unwrap()).unwrap();

        for input in tx.body.inputs {
            println!("input: {}#{}", input.transaction_id.to_hex(), input.index);
        }

        for output in &tx.body.outputs {
            println!(
                "output addr: {}, hex: {}",
                output.address().to_bech32(None).unwrap(),
                output.address().to_hex()
            );
        }
        println!("\n\n");

        let ff_datum = tx.body.outputs[0].datum().unwrap();
        if let DatumOption::Datum {
            datum,
            datum_bytes_encoding,
            ..
        } = ff_datum
        {
            println!("farm_factory datum CBOR: {}", hex::encode(datum.to_cbor_bytes()));
            dbg!(datum);
        } else {
            panic!("");
        }

        println!("\n\n");

        let mut m = tx.body.mint.unwrap();
        for e in m.entries() {
            let sh = e.key();
            let cred = StakeCredential::new_script(*sh);
            let e_addr = EnterpriseAddress::new(0, cred);
            let addr = e_addr.to_address();
            println!("mint script_hash: {}", sh.to_hex());
            println!("mint addr: {}", addr.to_bech32(None).unwrap());
            println!("mint addr (hex): {}", addr.to_hex());
            let vals = e.get();
        }
    }

    #[test]
    fn test_hex_encode() {
        let orig: Vec<u8> = vec![
            102, 97, 114, 109, 48, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0,
        ];
        let trunc: Vec<u8> = vec![102, 97, 114, 109, 48];
        let tx_input: Vec<u8> = vec![
            34, 137, 63, 115, 4, 79, 41, 24, 87, 53, 80, 108, 164, 159, 251, 51, 50, 147, 63, 63, 20, 189,
            145, 172, 75, 137, 165, 134, 131, 202, 199, 225,
        ];
        println!("orig: {}", hex::encode(&tx_input));
    }

    fn create_dummy_policy_id(val: u8) -> ScriptHash {
        let bytes: [u8; 28] = std::iter::repeat(val)
            .take(28)
            .collect::<Vec<u8>>()
            .try_into()
            .unwrap();
        ScriptHash::from(bytes)
    }

    #[test]
    fn witness_reward_addr() {
        let weighting_witness_script_hash_hex = "635fcf57b7e16a5f23f18620722688cf2ab227093ef42e49cecf61d1";
        let weighting_witness_script_hash = ScriptHash::from_hex(weighting_witness_script_hash_hex).unwrap();
        let staking_address =
            cml_chain::address::RewardAddress::new(0, Credential::new_script(weighting_witness_script_hash));
        println!("{}", staking_address.to_address().to_bech32(None).unwrap());

        let addr = BaseAddress::new(
            0,
            StakeCredential::new_script(weighting_witness_script_hash),
            StakeCredential::new_script(weighting_witness_script_hash),
        )
        .to_address();
        println!("BASE ADDRESS: {}", addr.to_bech32(None).unwrap());
    }
}
