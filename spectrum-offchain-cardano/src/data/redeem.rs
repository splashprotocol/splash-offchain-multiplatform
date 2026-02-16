use cml_chain::plutus::PlutusData;
use cml_chain::transaction::TransactionOutput;
use cml_crypto::Ed25519KeyHash;
use cml_multi_era::babbage::BabbageTransactionOutput;

use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::domain::order::UniqueOrder;
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;

use crate::data::order::{ClassicalOrder, OrderType, PoolNft};
use crate::data::pool::CFMMPoolAction::Redeem as RedeemAction;
use crate::data::pool::{CFMMPoolAction, Lq, Rx, Ry};
use crate::data::{OnChainOrderId, PoolId};
use crate::deployment::ProtocolValidator::{
    BalanceFnPoolRedeem, ConstFnFeeSwitchPoolRedeem, ConstFnPoolRedeem, RoyaltyPoolV1Redeem,
    RoyaltyPoolV2Redeem, StableFnPoolT2TRedeem,
};
use crate::deployment::{
    test_address, DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Redeem {
    pub pool_nft: PoolId,
    pub token_x: TaggedAssetClass<Rx>,
    pub token_y: TaggedAssetClass<Ry>,
    pub token_lq: TaggedAssetClass<Lq>,
    pub token_lq_amount: TaggedAmount<Lq>,
    pub ex_fee: u64,
    pub reward_pkh: Ed25519KeyHash,
    pub reward_stake_pkh: Option<Ed25519KeyHash>,
    pub collateral_ada: u64,
    pub order_type: OrderType,
}

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedeemOrderValidation {
    pub min_collateral_ada: u64,
}

pub type ClassicalOnChainRedeem = ClassicalOrder<OnChainOrderId, Redeem>;

impl<Ctx> RequiresValidator<Ctx> for ClassicalOnChainRedeem
where
    Ctx: Has<DeployedValidator<{ ConstFnFeeSwitchPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2TRedeem as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1Redeem as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2Redeem as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.order.order_type {
            OrderType::ConstFnFeeSwitch => {
                let validator: DeployedValidator<{ ConstFnFeeSwitchPoolRedeem as u8 }> = ctx.get();
                validator.erased()
            }
            OrderType::BalanceFn => {
                let validator: DeployedValidator<{ BalanceFnPoolRedeem as u8 }> = ctx.get();
                validator.erased()
            }
            OrderType::ConstFn => {
                let validator: DeployedValidator<{ ConstFnPoolRedeem as u8 }> = ctx.get();
                validator.erased()
            }
            OrderType::StableFn => {
                let validator: DeployedValidator<{ StableFnPoolT2TRedeem as u8 }> = ctx.get();
                validator.erased()
            }
            OrderType::RoyaltyConstFnV1 => {
                let validator: DeployedValidator<{ RoyaltyPoolV1Redeem as u8 }> = ctx.get();
                validator.erased()
            }
            OrderType::RoyaltyConstFnV2 => {
                let validator: DeployedValidator<{ RoyaltyPoolV2Redeem as u8 }> = ctx.get();
                validator.erased()
            }
        }
    }
}

impl Into<CFMMPoolAction> for ClassicalOnChainRedeem {
    fn into(self) -> CFMMPoolAction {
        RedeemAction
    }
}

impl UniqueOrder for ClassicalOnChainRedeem {
    type TOrderId = OnChainOrderId;
    fn get_self_ref(&self) -> Self::TOrderId {
        self.id
    }
}

struct OnChainRedeemConfig {
    pool_nft: TaggedAssetClass<PoolNft>,
    token_x: TaggedAssetClass<Rx>,
    token_y: TaggedAssetClass<Ry>,
    token_lq: TaggedAssetClass<Lq>,
    ex_fee: u64,
    reward_pkh: Ed25519KeyHash,
    reward_stake_pkh: Option<Ed25519KeyHash>,
}

impl<Ctx> TryFromLedger<TransactionOutput, Ctx> for ClassicalOnChainRedeem
where
    Ctx: Has<OutputRef>
        + Has<DeployedScriptInfo<{ ConstFnFeeSwitchPoolRedeem as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolRedeem as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolRedeem as u8 }>>
        + Has<DeployedScriptInfo<{ StableFnPoolT2TRedeem as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1Redeem as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV2Redeem as u8 }>>
        + Has<RedeemOrderValidation>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &Ctx) -> Option<Self> {
        let is_const_fee_switch_pool_deposit =
            test_address::<{ ConstFnFeeSwitchPoolRedeem as u8 }, Ctx>(repr.address(), ctx);
        let is_const_pool_redeem = test_address::<{ ConstFnPoolRedeem as u8 }, Ctx>(repr.address(), ctx);
        let is_balance_pool_redeem = test_address::<{ BalanceFnPoolRedeem as u8 }, Ctx>(repr.address(), ctx);
        let is_stable_pool_redeem = test_address::<{ StableFnPoolT2TRedeem as u8 }, Ctx>(repr.address(), ctx);
        let is_royalty_v1_pool_redeem =
            test_address::<{ RoyaltyPoolV1Redeem as u8 }, Ctx>(repr.address(), ctx);
        let is_royalty_v2_pool_redeem =
            test_address::<{ RoyaltyPoolV2Redeem as u8 }, Ctx>(repr.address(), ctx);
        if is_const_fee_switch_pool_deposit
            || is_balance_pool_redeem
            || is_const_pool_redeem
            || is_stable_pool_redeem
            || is_royalty_v1_pool_redeem
            || is_royalty_v2_pool_redeem
        {
            let order_type = if is_const_fee_switch_pool_deposit {
                OrderType::ConstFnFeeSwitch
            } else if is_balance_pool_redeem {
                OrderType::BalanceFn
            } else if is_const_pool_redeem {
                OrderType::ConstFn
            } else if is_royalty_v1_pool_redeem {
                OrderType::RoyaltyConstFnV1
            } else if is_royalty_v2_pool_redeem {
                OrderType::RoyaltyConstFnV2
            } else {
                OrderType::StableFn
            };
            let value = repr.value().clone();
            let conf = OnChainRedeemConfig::try_from_pd(repr.datum().clone()?.into_pd()?)?;
            let token_lq_amount = TaggedAmount::new(value.amount_of(conf.token_lq.untag())?);
            let collateral_ada = value.amount_of(AssetClass::Native)? - conf.ex_fee;
            let redeem = Redeem {
                pool_nft: PoolId::try_from(conf.pool_nft).ok()?,
                token_x: conf.token_x,
                token_y: conf.token_y,
                token_lq: conf.token_lq,
                token_lq_amount,
                ex_fee: conf.ex_fee,
                reward_pkh: conf.reward_pkh,
                reward_stake_pkh: conf.reward_stake_pkh,
                collateral_ada,
                order_type,
            };

            let bounds = ctx.select::<RedeemOrderValidation>();

            if collateral_ada >= bounds.min_collateral_ada {
                return Some(ClassicalOrder {
                    id: OnChainOrderId::from(ctx.select::<OutputRef>()),
                    pool_id: PoolId::try_from(conf.pool_nft).ok()?,
                    order: redeem,
                });
            }
        }
        None
    }
}

impl TryFromPData for OnChainRedeemConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let stake_pkh = cpd
            .take_field(6)
            .clone()
            .and_then(|pd| pd.into_constr_pd())
            .and_then(|mut cpd_spkh| cpd_spkh.take_field(0))
            .and_then(|pd| pd.into_bytes())
            .and_then(|bytes| <[u8; 28]>::try_from(bytes).ok())
            .map(|bytes| Ed25519KeyHash::from(bytes));

        Some(OnChainRedeemConfig {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?,
            token_x: TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?,
            token_y: TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?,
            token_lq: TaggedAssetClass::try_from_pd(cpd.take_field(3)?)?,
            ex_fee: cpd.take_field(4)?.into_u64()?,
            reward_pkh: Ed25519KeyHash::from(<[u8; 28]>::try_from(cpd.take_field(5)?.into_bytes()?).ok()?),
            reward_stake_pkh: stake_pkh,
        })
    }
}

#[cfg(test)]
mod tests {
    use cml_chain::plutus::PlutusData;
    use cml_core::serialization::Deserialize;

    use spectrum_cardano_lib::types::TryFromPData;

    use crate::data::redeem::OnChainRedeemConfig;

    #[test]
    fn parse_deposit_datum_mainnet() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM_SAMPLE).unwrap()).unwrap();
        let maybe_conf = OnChainRedeemConfig::try_from_pd(pd);
        assert!(maybe_conf.is_some())
    }

    const DATUM_SAMPLE: &str =
        "d8799fd8799f581cbb461a9afa6e60962c72d520b476f60f5b24554614531ef1fe34236853436f726e75636f706961735f4144415f4e4654ffd8799f4040ffd8799f581cb6a7467ea1deb012808ef4e87b5ff371e85f7142d7b356a40d9b42a0581e436f726e75636f70696173205b76696120436861696e506f72742e696f5dffd8799f581ce6cdb6e0e98a136df23bbea57ab39417c82302947779be2d9acedf0a52436f726e75636f706961735f4144415f4c51ff1a0016e360581cf197ea0891ce786a9a41b59255bf0efa6c2fb47d0d0babdfed7a294cd8799f581c0a391e83011b5bcfdc7435e9b50fbff6a8bdeb9e7ad8706f7b2673dbffff";
}
