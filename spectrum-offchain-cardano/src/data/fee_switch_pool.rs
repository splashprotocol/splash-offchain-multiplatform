use crate::constants::{FEE_DEN, MAX_LQ_CAP};
use crate::data::deposit::ClassicalOnChainDeposit;
use crate::data::limit_swap::ClassicalOnChainLimitSwap;
use crate::data::operation_output::{DepositOutput, RedeemOutput, SwapOutput};
use crate::data::order::{Base, ClassicalOrder, PoolNft, Quote};
use crate::data::pool::{ApplyOrder, CFMMPoolAction, ImmutablePoolUtxo, Lq, Rx, Ry, Slippage};
use crate::data::redeem::ClassicalOnChainRedeem;
use crate::data::{PoolId, PoolStateVer, PoolVer};
use cml_chain::assets::MultiAsset;
use cml_chain::plutus::PlutusData;
use cml_chain::plutus::PlutusData::Integer;
use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, TransactionOutput};
use cml_chain::utils::BigInt;
use cml_chain::{PolicyId, PolicyIdList, Value};
use cml_core::serialization::Serialize;
use cml_crypto::typed_bytes::ByteArray;
use cml_crypto::ScriptHash;
use cml_multi_era::babbage::BabbageTransactionOutput;
use isahc::http::Version;
use log::info;
use num_rational::Ratio;
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension, RequiresRedeemer,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::data::{EntitySnapshot, Has, Stable};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};
use type_equalities::IsEqual;

use crate::data::pool::PoolOps;
use crate::fees::FeeExtension;
use crate::pool_math::cfmm_math::{output_amount, reward_lp, shares_amount};

#[derive(Debug, Copy, Clone)]
pub struct FeeSwitchPool {
    pub id: PoolId,
    pub state_ver: PoolStateVer,
    pub reserves_x: TaggedAmount<Rx>,
    pub reserves_y: TaggedAmount<Ry>,
    pub liquidity: TaggedAmount<Lq>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee: Ratio<u64>,
    pub treasury_fee: Ratio<u64>,
    pub treasury_x: TaggedAmount<Rx>,
    pub treasury_y: TaggedAmount<Ry>,
    pub lq_lower_bound: TaggedAmount<Lq>,
    pub ver: PoolVer,
}

impl FeeSwitchPool {
    fn update_treasury(self, prev_datum: Option<DatumOption>) -> Option<DatumOption> {
        match prev_datum {
            None => None,
            Some(DatumOption::Hash { .. }) => None,
            Some(DatumOption::Datum { datum, .. }) => {
                let new_treasury_x = Integer(BigInt::from(self.treasury_x.untag()));
                let new_treasury_y = Integer(BigInt::from(self.treasury_y.untag()));

                let mut cpd = datum.into_constr_pd()?;

                cpd.update_field_unsafe(6, new_treasury_x)?;
                cpd.update_field_unsafe(7, new_treasury_y)?;

                Some(DatumOption::new_datum(PlutusData::ConstrPlutusData(cpd)))
            }
        }
    }
}

impl Stable for FeeSwitchPool {
    type StableId = PoolId;

    fn stable_id(&self) -> Self::StableId {
        self.id
    }
}

impl EntitySnapshot for FeeSwitchPool {
    type Version = PoolStateVer;
    fn version(&self) -> Self::Version {
        self.state_ver
    }

    fn update_version(&mut self, new_version: PoolStateVer) {
        self.state_ver = new_version;
    }
}

impl PoolOps for FeeSwitchPool {
    fn get_asset_x(&self) -> TaggedAssetClass<Rx> {
        self.asset_x
    }

    fn get_reserves_x(&self) -> TaggedAmount<Rx> {
        self.reserves_x - self.reserves_x
    }

    fn get_reserves_y(&self) -> TaggedAmount<Ry> {
        self.reserves_y - self.treasury_y
    }

    fn set_reserves_x(mut self, new_value: TaggedAmount<Rx>) -> () {
        self.reserves_x = new_value
    }

    fn set_reserves_y(mut self, new_value: TaggedAmount<Ry>) -> () {
        self.reserves_y = new_value
    }

    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote> {
        output_amount(
            self.asset_x,
            self.reserves_x,
            self.reserves_y,
            base_asset,
            base_amount,
            self.lp_fee,
            self.lp_fee,
        )
    }

    fn reward_lp(
        &self,
        in_x_amount: u64,
        in_y_amount: u64,
    ) -> (TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>) {
        reward_lp(
            self.reserves_x,
            self.reserves_y,
            self.liquidity,
            in_x_amount,
            in_y_amount,
        )
    }

    fn shares_amount(self, burned_lq: TaggedAmount<Lq>) -> (TaggedAmount<Rx>, TaggedAmount<Ry>) {
        shares_amount(self.reserves_x, self.reserves_y, self.liquidity, burned_lq)
    }
}

impl ApplyOrder<ClassicalOnChainLimitSwap> for FeeSwitchPool {
    type OrderApplicationResult = SwapOutput;

    fn apply_order(
        mut self,
        ClassicalOrder { id, pool_id, order }: ClassicalOnChainLimitSwap,
    ) -> Result<(Self, SwapOutput), Slippage<ClassicalOnChainLimitSwap>> {
        let quote_amount = self.output_amount(order.base_asset, order.base_amount);
        if quote_amount < order.min_expected_quote_amount {
            return Err(Slippage(ClassicalOrder { id, pool_id, order }));
        }
        // Adjust pool value.
        if order.quote_asset.untag() == self.asset_x.untag() {
            let x_from_swap_without_fee = self.reserves_x.untag()
                - (self.reserves_y.untag() * self.reserves_x.untag())
                    / (self.reserves_y.untag() + order.base_amount.untag());
            let lq_fee = (x_from_swap_without_fee * (10000 - self.lp_fee.numer()) / self.lp_fee.denom());
            let additional_treasury_x = (((lq_fee as u128) * (*self.treasury_fee.numer() as u128))
                / (*self.treasury_fee.denom() as u128)) as u64;
            self.reserves_x = self.reserves_x - quote_amount.retag();
            self.reserves_y = self.reserves_y + order.base_amount.retag();
            self.treasury_x = self.treasury_x + TaggedAmount::new(additional_treasury_x);
        } else {
            let y_from_swap_without_fee = self.reserves_y.untag()
                - (self.reserves_x.untag() * self.reserves_y.untag())
                    / (self.reserves_x.untag() + order.base_amount.untag());
            let lq_fee = (y_from_swap_without_fee * (10000 - self.lp_fee.numer()) / self.lp_fee.denom());
            let additional_treasury_y = (((lq_fee as u128) * (*self.treasury_fee.numer() as u128))
                / (*self.treasury_fee.denom() as u128)) as u64;
            self.reserves_y = self.reserves_y - quote_amount.retag();
            self.reserves_x = self.reserves_x + order.base_amount.retag();
            self.treasury_y = self.treasury_y + TaggedAmount::new(additional_treasury_y);
        }
        // Prepare user output.
        let batcher_fee = order.fee.value().linear_fee(quote_amount.untag());
        if (batcher_fee > order.ada_deposit) {
            //incorrect error
            return Err(Slippage(ClassicalOrder { id, pool_id, order }));
        }
        let ada_residue = order.ada_deposit - batcher_fee;
        let swap_output = SwapOutput {
            quote_asset: order.quote_asset,
            quote_amount,
            ada_residue,
            redeemer_pkh: order.redeemer_pkh,
            redeemer_stake_pkh: order.redeemer_stake_pkh,
        };
        // Prepare batcher fee.
        Ok((self, swap_output))
    }
}

impl ApplyOrder<ClassicalOnChainDeposit> for FeeSwitchPool {
    type OrderApplicationResult = DepositOutput;

    fn apply_order(
        mut self,
        ClassicalOrder { id, pool_id, order }: ClassicalOnChainDeposit,
    ) -> Result<(Self, DepositOutput), Slippage<ClassicalOnChainDeposit>> {
        let net_x = if order.token_x.is_native() {
            order.token_x_amount.untag() - order.ex_fee - order.collateral_ada
        } else {
            order.token_x_amount.untag()
        };

        let net_y = if order.token_y.is_native() {
            order.token_y_amount.untag() - order.ex_fee - order.collateral_ada
        } else {
            order.token_y_amount.untag()
        };

        let (unlocked_lq, change_x, change_y) = self.reward_lp(net_x, net_y);

        self.reserves_x = self.reserves_x + TaggedAmount::new(net_x) - change_x;
        self.reserves_y = self.reserves_y + TaggedAmount::new(net_y) - change_y;
        self.liquidity = self.liquidity + unlocked_lq;

        let deposit_output = DepositOutput {
            token_x_asset: order.token_x,
            token_x_charge_amount: change_x,
            token_y_asset: order.token_y,
            token_y_charge_amount: change_y,
            token_lq_asset: order.token_lq,
            token_lq_amount: unlocked_lq,
            ada_residue: order.collateral_ada,
            redeemer_pkh: order.reward_pkh,
            redeemer_stake_pkh: order.reward_stake_pkh,
        };

        Ok((self, deposit_output))
    }
}

impl ApplyOrder<ClassicalOnChainRedeem> for FeeSwitchPool {
    type OrderApplicationResult = RedeemOutput;

    fn apply_order(
        mut self,
        ClassicalOrder { order, .. }: ClassicalOnChainRedeem,
    ) -> Result<(Self, RedeemOutput), Slippage<ClassicalOnChainRedeem>> {
        let (x_amount, y_amount) = self.clone().shares_amount(order.token_lq_amount);

        self.reserves_x = self.reserves_x - x_amount;
        self.reserves_y = self.reserves_y - y_amount;
        self.liquidity = self.liquidity + order.token_lq_amount;

        let redeem_output = RedeemOutput {
            token_x_asset: order.token_x,
            token_x_amount: x_amount,
            token_y_asset: order.token_y,
            token_y_amount: y_amount,
            ada_residue: order.collateral_ada,
            redeemer_pkh: order.reward_pkh,
            redeemer_stake_pkh: order.reward_stake_pkh,
        };

        Ok((self, redeem_output))
    }
}

impl Has<PoolStateVer> for FeeSwitchPool {
    fn get_labeled<U: IsEqual<PoolStateVer>>(&self) -> PoolStateVer {
        self.state_ver
    }
}

impl Has<PoolVer> for FeeSwitchPool {
    fn get_labeled<U: IsEqual<PoolVer>>(&self) -> PoolVer {
        self.ver
    }
}

impl RequiresRedeemer<CFMMPoolAction> for FeeSwitchPool {
    fn redeemer(action: CFMMPoolAction) -> PlutusData {
        action.to_plutus_data()
    }
}

impl TryFromLedger<BabbageTransactionOutput, OutputRef> for FeeSwitchPool {
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
        // BabbageTransactionOutput::from_cbor_bytes(&*hex::decode(SWAP_SAMPLE).unwrap()).unwrap();

        if let Some(pool_ver) = PoolVer::try_from_pool_address(repr.address()) {
            let value = repr.value();
            let pd = repr.clone().into_datum()?.into_pd()?;
            let conf = crate::data::fee_switch_pool::FeeSwitchPoolConfig::try_from_pd(pd.clone())?;
            let treasury_x = TaggedAmount::new(conf.treasury_x);
            let treasury_y = TaggedAmount::new(conf.treasury_y);
            let reserves_x = TaggedAmount::new(value.amount_of(conf.asset_x.into())?) - treasury_x;
            let reserves_y = TaggedAmount::new(value.amount_of(conf.asset_y.into())?) - treasury_y;
            let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
            let liquidity = TaggedAmount::new(MAX_LQ_CAP - liquidity_neg);
            return Some(FeeSwitchPool {
                id: PoolId::try_from(conf.pool_nft).ok()?,
                state_ver: PoolStateVer::from(ctx),
                reserves_x,
                reserves_y,
                liquidity,
                asset_x: conf.asset_x,
                asset_y: conf.asset_y,
                asset_lq: conf.asset_lq,
                lp_fee: Ratio::new_raw(conf.lp_fee_num, FEE_DEN),
                treasury_fee: Ratio::new_raw(conf.treasury_fee_num, FEE_DEN),
                treasury_x,
                treasury_y,
                lq_lower_bound: conf.lq_lower_bound,
                ver: pool_ver,
            });
        }
        None
    }
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for FeeSwitchPool {
    fn into_ledger(self, immut_pool: ImmutablePoolUtxo) -> TransactionOutput {
        let mut ma = MultiAsset::new();
        let coins = if self.asset_x.is_native() {
            let (policy, name) = self.asset_y.untag().into_token().unwrap();
            ma.set(
                policy,
                name.into(),
                (self.reserves_y.untag() + self.treasury_y.untag()),
            );
            self.reserves_x.untag() + self.treasury_x.untag()
        } else if self.asset_y.is_native() {
            let (policy, name) = self.asset_x.untag().into_token().unwrap();
            ma.set(
                policy,
                name.into(),
                (self.reserves_x.untag() + self.treasury_x.untag()),
            );
            self.reserves_y.untag() + self.treasury_y.untag()
        } else {
            let (policy_x, name_x) = self.asset_x.untag().into_token().unwrap();
            ma.set(
                policy_x,
                name_x.into(),
                (self.reserves_x.untag() + self.treasury_x.untag()),
            );
            let (policy_y, name_y) = self.asset_y.untag().into_token().unwrap();
            ma.set(
                policy_y,
                name_y.into(),
                (self.reserves_y.untag() + self.treasury_y.untag()),
            );
            immut_pool.value
        };
        let (policy_lq, name_lq) = self.asset_lq.untag().into_token().unwrap();
        let (nft_lq, name_nft) = self.id.0;
        ma.set(policy_lq, name_lq.into(), MAX_LQ_CAP - self.liquidity.untag());
        ma.set(nft_lq, name_nft.into(), 1);

        TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
            address: immut_pool.address,
            amount: Value::new(coins, ma),
            datum_option: self.update_treasury(immut_pool.datum_option),
            script_reference: immut_pool.script_reference,
            encodings: None,
        })
    }
}

pub struct FeeSwitchPoolConfig {
    pool_nft: TaggedAssetClass<PoolNft>,
    asset_x: TaggedAssetClass<Rx>,
    asset_y: TaggedAssetClass<Ry>,
    asset_lq: TaggedAssetClass<Lq>,
    lp_fee_num: u64,
    treasury_fee_num: u64,
    treasury_x: u64,
    treasury_y: u64,
    lq_lower_bound: TaggedAmount<Lq>,
}

impl TryFromPData for FeeSwitchPoolConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(Self {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?,
            asset_x: TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?,
            asset_y: TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?,
            asset_lq: TaggedAssetClass::try_from_pd(cpd.take_field(3)?)?,
            lp_fee_num: cpd.take_field(4)?.into_u64()?,
            treasury_fee_num: cpd.take_field(5)?.into_u64()?,
            treasury_x: cpd.take_field(6)?.into_u64()?,
            treasury_y: cpd.take_field(7)?.into_u64()?,
            lq_lower_bound: TaggedAmount::new(cpd.take_field(9).and_then(|pd| pd.into_u64()).unwrap_or(0)),
        })
    }
}

mod tests {
    use crate::data::fee_switch_pool::FeeSwitchPoolConfig;
    use crate::data::limit_swap::OnChainLimitSwapConfig;
    use cml_chain::plutus::PlutusData;
    use cml_core::serialization::Deserialize;
    use spectrum_cardano_lib::types::TryFromPData;

    const DATUM_SAMPLE: &str =
        "d8799fd8799f581c6aaa652b39f5723afc85bba38401a4cbfd5b2f7aa3771504257ac8a74d74657374425f4144415f4e4654ffd8799f4040ffd8799f581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26457465737442ffd8799f581c635f44ae5df86be9e80fd0c57a5ec699a146d9d9034516ffd72febef4c74657374425f4144415f4c51ff19270b010000801b00000002540be400581c2618e94cdb06792f05ae9b1ec78b0231f4b7f4215b1b4cf52e6342deff";

    #[test]
    fn parse_fee_switch_datum_mainnet() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM_SAMPLE).unwrap()).unwrap();
        let maybe_conf = FeeSwitchPoolConfig::try_from_pd(pd);
        assert!(maybe_conf.is_some())
    }
}
