use cml_chain::plutus::PlutusData;
use cml_crypto::Ed25519KeyHash;
use cml_multi_era::babbage::BabbageTransactionOutput;

use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::data::order::UniqueOrder;
use spectrum_offchain::data::Has;
use spectrum_offchain::ledger::TryFromLedger;

use crate::data::order::{ClassicalOrder, PoolNft};
use crate::data::pool::CFMMPoolAction::Deposit as DepositAction;
use crate::data::pool::{CFMMPoolAction, Lq, Rx, Ry};
use crate::data::{OnChainOrderId, PoolId};
use crate::deployment::ProtocolValidator::ConstFnPoolDeposit;
use crate::deployment::{
    test_address, DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator,
};

#[derive(Debug, Clone, Eq, PartialEq)]
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
}

pub type ClassicalOnChainDeposit = ClassicalOrder<OnChainOrderId, Deposit>;

impl<Ctx> RequiresValidator<Ctx> for ClassicalOnChainDeposit
where
    Ctx: Has<DeployedValidator<{ ConstFnPoolDeposit as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        ctx.get().erased()
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

impl<Ctx> TryFromLedger<BabbageTransactionOutput, Ctx> for ClassicalOnChainDeposit
where
    Ctx: Has<OutputRef> + Has<DeployedScriptInfo<{ ConstFnPoolDeposit as u8 }>>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &Ctx) -> Option<Self> {
        if test_address(repr.address(), ctx) {
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
            };

            return Some(ClassicalOrder {
                id: OnChainOrderId::from(ctx.select::<OutputRef>()),
                pool_id: PoolId::try_from(conf.pool_nft).ok()?,
                order: deposit,
            });
        }
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
        "d8799fd8799f581ca80022230c821a52e426d2fdb096e7d967b5ab25d350d469a7603dbf4b5350465f4144415f4e4654ffd8799f4040ffd8799f581c09f2d4e4a5c3662f4c1e6a7d9600e9605279dbdcedb22d4507cb6e7543535046ffd8799f581c74f47c99ac793c29280575b08fe20c7fb75543bff5b1581f7c162e7c4a5350465f4144415f4c51ff1a0016e360581ca104c691610dccf8e15f156bb96923cce7d61cc74370ed8e9111ef05d8799f581c874ea2c1b94eed88c1d09f057e96ffe0fa6311527e2003432a968794ff1a0014c828ff";
}
