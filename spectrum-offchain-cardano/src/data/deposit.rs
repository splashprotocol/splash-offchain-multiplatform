use cml_chain::plutus::PlutusData;
use cml_chain::transaction::TransactionOutput;
use cml_crypto::Ed25519KeyHash;

use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::domain::order::UniqueOrder;
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;

use crate::data::order::{ClassicalOrder, OrderType, PoolNft};
use crate::data::pool::CFMMPoolAction::Deposit as DepositAction;
use crate::data::pool::{CFMMPoolAction, Lq, Rx, Ry};
use crate::data::{OnChainOrderId, PoolId};
use crate::deployment::ProtocolValidator::{
    BalanceFnPoolDeposit, ConstFnFeeSwitchPoolDeposit, ConstFnPoolDeposit, RoyaltyPoolV1Deposit,
    RoyaltyPoolV2Deposit, StableFnPoolT2TDeposit,
};
use crate::deployment::{
    test_address, DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Deposit {
    pub pool_nft: PoolId,
    pub token_x: TaggedAssetClass<Rx>,
    pub token_x_amount: TaggedAmount<Rx>,
    pub token_y: TaggedAssetClass<Ry>,
    pub token_y_amount: TaggedAmount<Ry>,
    pub token_lq: TaggedAssetClass<Lq>,
    pub ex_fee: u64,
    pub reward_pkh: Ed25519KeyHash,
    pub reward_stake_pkh: Option<Ed25519KeyHash>,
    pub collateral_ada: u64,
    pub order_type: OrderType,
}

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepositOrderValidation {
    pub min_collateral_ada: u64,
}

pub type ClassicalOnChainDeposit = ClassicalOrder<OnChainOrderId, Deposit>;

impl<Ctx> RequiresValidator<Ctx> for ClassicalOnChainDeposit
where
    Ctx: Has<DeployedValidator<{ ConstFnFeeSwitchPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2TDeposit as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1Deposit as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2Deposit as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.order.order_type {
            OrderType::ConstFnFeeSwitch => {
                let validator: DeployedValidator<{ ConstFnFeeSwitchPoolDeposit as u8 }> = ctx.get();
                validator.erased()
            }
            OrderType::BalanceFn => {
                let validator: DeployedValidator<{ BalanceFnPoolDeposit as u8 }> = ctx.get();
                validator.erased()
            }
            OrderType::ConstFn => {
                let validator: DeployedValidator<{ ConstFnPoolDeposit as u8 }> = ctx.get();
                validator.erased()
            }
            OrderType::StableFn => {
                let validator: DeployedValidator<{ StableFnPoolT2TDeposit as u8 }> = ctx.get();
                validator.erased()
            }
            OrderType::RoyaltyConstFnV1 => {
                let validator: DeployedValidator<{ RoyaltyPoolV1Deposit as u8 }> = ctx.get();
                validator.erased()
            }
            OrderType::RoyaltyConstFnV2 => {
                let validator: DeployedValidator<{ RoyaltyPoolV2Deposit as u8 }> = ctx.get();
                validator.erased()
            }
        }
    }
}

impl Into<CFMMPoolAction> for ClassicalOnChainDeposit {
    fn into(self) -> CFMMPoolAction {
        DepositAction
    }
}

impl UniqueOrder for ClassicalOnChainDeposit {
    type TOrderId = OnChainOrderId;
    fn get_self_ref(&self) -> Self::TOrderId {
        self.id
    }
}

impl<Ctx> TryFromLedger<TransactionOutput, Ctx> for ClassicalOnChainDeposit
where
    Ctx: Has<OutputRef>
        + Has<DeployedScriptInfo<{ ConstFnFeeSwitchPoolDeposit as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolDeposit as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolDeposit as u8 }>>
        + Has<DeployedScriptInfo<{ StableFnPoolT2TDeposit as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1Deposit as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV2Deposit as u8 }>>
        + Has<DepositOrderValidation>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &Ctx) -> Option<Self> {
        let is_const_fee_switch_pool_deposit =
            test_address::<{ ConstFnFeeSwitchPoolDeposit as u8 }, Ctx>(repr.address(), ctx);
        let is_const_fn_pool_deposit = test_address::<{ ConstFnPoolDeposit as u8 }, Ctx>(repr.address(), ctx);
        let is_balance_fn_pool_deposit =
            test_address::<{ BalanceFnPoolDeposit as u8 }, Ctx>(repr.address(), ctx);
        let is_stable_fn_pool_deposit =
            test_address::<{ StableFnPoolT2TDeposit as u8 }, Ctx>(repr.address(), ctx);
        let is_royalty_v1_fn_pool_deposit =
            test_address::<{ RoyaltyPoolV1Deposit as u8 }, Ctx>(repr.address(), ctx);
        let is_royalty_v2_fn_pool_deposit =
            test_address::<{ RoyaltyPoolV2Deposit as u8 }, Ctx>(repr.address(), ctx);
        if is_const_fee_switch_pool_deposit
            || is_balance_fn_pool_deposit
            || is_const_fn_pool_deposit
            || is_stable_fn_pool_deposit
            || is_royalty_v1_fn_pool_deposit
            || is_royalty_v2_fn_pool_deposit
        {
            let order_type = if is_const_fee_switch_pool_deposit {
                OrderType::ConstFnFeeSwitch
            } else if is_balance_fn_pool_deposit {
                OrderType::BalanceFn
            } else if is_const_fn_pool_deposit {
                OrderType::ConstFn
            } else if is_stable_fn_pool_deposit {
                OrderType::StableFn
            } else if is_royalty_v1_fn_pool_deposit {
                OrderType::RoyaltyConstFnV1
            } else {
                OrderType::RoyaltyConstFnV2
            };
            let value = repr.value().clone();
            let conf = OnChainDepositConfig::try_from_pd(repr.clone().into_datum()?.into_pd()?)?;
            let token_x_amount = TaggedAmount::new(value.amount_of(conf.token_x.untag()).unwrap_or(0));
            let token_y_amount = TaggedAmount::new(value.amount_of(conf.token_y.untag()).unwrap_or(0));
            let deposit = Deposit {
                pool_nft: PoolId::try_from(conf.pool_nft).ok()?,
                token_x: conf.token_x,
                token_x_amount,
                token_y: conf.token_y,
                token_y_amount,
                token_lq: conf.token_lq,
                ex_fee: conf.ex_fee,
                reward_pkh: conf.reward_pkh,
                reward_stake_pkh: conf.reward_stake_pkh,
                collateral_ada: conf.collateral_ada,
                order_type,
            };

            let bounds = ctx.select::<DepositOrderValidation>();

            if conf.collateral_ada >= bounds.min_collateral_ada {
                return Some(ClassicalOrder {
                    id: OnChainOrderId::from(ctx.select::<OutputRef>()),
                    pool_id: PoolId::try_from(conf.pool_nft).ok()?,
                    order: deposit,
                });
            }
        };
        None
    }
}

struct OnChainDepositConfig {
    pool_nft: TaggedAssetClass<PoolNft>,
    token_x: TaggedAssetClass<Rx>,
    token_y: TaggedAssetClass<Ry>,
    token_lq: TaggedAssetClass<Lq>,
    ex_fee: u64,
    reward_pkh: Ed25519KeyHash,
    reward_stake_pkh: Option<Ed25519KeyHash>,
    collateral_ada: u64,
}

impl TryFromPData for OnChainDepositConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let stake_pkh: Option<Ed25519KeyHash> = cpd
            .take_field(6)
            .clone()
            .and_then(|pd| pd.into_constr_pd())
            .and_then(|mut cpd_spkh| cpd_spkh.take_field(0))
            .and_then(|pd| pd.into_bytes())
            .and_then(|bytes| <[u8; 28]>::try_from(bytes).ok())
            .map(|bytes| Ed25519KeyHash::from(bytes));

        Some(OnChainDepositConfig {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?,
            token_x: TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?,
            token_y: TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?,
            token_lq: TaggedAssetClass::try_from_pd(cpd.take_field(3)?)?,
            ex_fee: cpd.take_field(4)?.into_u64()?,
            reward_pkh: Ed25519KeyHash::from(<[u8; 28]>::try_from(cpd.take_field(5)?.into_bytes()?).ok()?),
            reward_stake_pkh: stake_pkh,
            collateral_ada: cpd.take_field(7)?.into_u64()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use cml_chain::plutus::PlutusData;
    use cml_core::serialization::Deserialize;

    use spectrum_cardano_lib::types::TryFromPData;

    use crate::data::deposit::OnChainDepositConfig;

    #[test]
    fn parse_deposit_datum_mainnet() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM_SAMPLE).unwrap()).unwrap();
        let maybe_conf = OnChainDepositConfig::try_from_pd(pd);
        assert!(maybe_conf.is_some())
    }

    const DATUM_SAMPLE: &str =
        "d8799fd8799f581c6a875653bb9e387d3dbd030c4195e027c333fd075b105302457e2470436e6674ffd8799f4040ffd8799f581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26457465737443ffd8799f581cf716e211496e520e79d6a1573a3de09d3f2eb36f883178083cf30e7d426c71ff1a001e8480581c8d4be10d934b60a22f267699ea3f7ebdade1f8e535d1bd0ef7ce18b6d87a801a001e8480ff";
}
