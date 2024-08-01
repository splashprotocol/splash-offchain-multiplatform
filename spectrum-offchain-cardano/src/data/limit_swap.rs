use cml_chain::plutus::PlutusData;
use cml_chain::Coin;
use cml_crypto::Ed25519KeyHash;
use cml_multi_era::babbage::BabbageTransactionOutput;
use num_rational::Ratio;

use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::data::order::UniqueOrder;
use spectrum_offchain::data::Has;
use spectrum_offchain::ledger::TryFromLedger;

use crate::constants::MIN_SAFE_LOVELACE_VALUE;
use crate::data::order::{Base, ClassicalOrder, OrderType, PoolNft, Quote};
use crate::data::pool::CFMMPoolAction;
use crate::data::pool::CFMMPoolAction::Swap;
use crate::data::{ExecutorFeePerToken, OnChainOrderId, PoolId};
use crate::deployment::ProtocolValidator::ConstFnFeeSwitchPoolSwap;
use crate::deployment::{
    test_address, DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator,
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LimitSwap {
    pub base_asset: TaggedAssetClass<Base>,
    pub base_amount: TaggedAmount<Base>,
    pub quote_asset: TaggedAssetClass<Quote>,
    pub ada_deposit: Coin,
    pub min_expected_quote_amount: TaggedAmount<Quote>,
    pub fee: ExecutorFeePerToken,
    pub redeemer_pkh: Ed25519KeyHash,
    pub redeemer_stake_pkh: Option<Ed25519KeyHash>,
}

pub type ClassicalOnChainLimitSwap = ClassicalOrder<OnChainOrderId, LimitSwap>;

impl<Ctx> RequiresValidator<Ctx> for ClassicalOnChainLimitSwap
where
    Ctx: Has<DeployedValidator<{ ConstFnFeeSwitchPoolSwap as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        ctx.get().erased()
    }
}

impl Into<CFMMPoolAction> for ClassicalOnChainLimitSwap {
    fn into(self) -> CFMMPoolAction {
        Swap
    }
}

impl UniqueOrder for ClassicalOnChainLimitSwap {
    type TOrderId = OnChainOrderId;
    fn get_self_ref(&self) -> Self::TOrderId {
        self.id
    }
}

impl<Ctx> TryFromLedger<BabbageTransactionOutput, Ctx> for ClassicalOnChainLimitSwap
where
    Ctx: Has<OutputRef> + Has<DeployedScriptInfo<{ ConstFnFeeSwitchPoolSwap as u8 }>>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &Ctx) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let value = repr.value().clone();
            let conf = OnChainLimitSwapConfig::try_from_pd(repr.datum()?.into_pd()?)?;
            let real_base_input = value.amount_of(conf.base.untag()).unwrap_or(0);
            let (min_base, ada_deposit) = if conf.base.is_native() {
                let min = conf.base_amount.untag()
                    + ((conf.min_quote_amount.untag() as u128) * conf.ex_fee_per_token_num
                        / conf.ex_fee_per_token_denom) as u64;
                let ada = real_base_input - conf.base_amount.untag();
                (min, ada)
            } else {
                (conf.base_amount.untag(), value.coin)
            };
            if real_base_input < min_base || ada_deposit < MIN_SAFE_LOVELACE_VALUE {
                return None;
            }
            let swap = LimitSwap {
                base_asset: conf.base,
                base_amount: conf.base_amount,
                quote_asset: conf.quote,
                min_expected_quote_amount: conf.min_quote_amount,
                ada_deposit,
                fee: ExecutorFeePerToken::new(
                    Ratio::new(conf.ex_fee_per_token_num, conf.ex_fee_per_token_denom),
                    AssetClass::Native,
                ),
                redeemer_pkh: conf.redeemer_pkh,
                redeemer_stake_pkh: conf.redeemer_stake_pkh,
            };
            return Some(ClassicalOrder {
                id: OnChainOrderId::from(ctx.select::<OutputRef>()),
                pool_id: PoolId::try_from(conf.pool_nft).ok()?,
                order: swap,
            });
        }
        None
    }
}

pub struct OnChainLimitSwapConfig {
    pub base: TaggedAssetClass<Base>,
    pub base_amount: TaggedAmount<Base>,
    pub quote: TaggedAssetClass<Quote>,
    pub min_quote_amount: TaggedAmount<Quote>,
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub ex_fee_per_token_num: u128,
    pub ex_fee_per_token_denom: u128,
    pub redeemer_pkh: Ed25519KeyHash,
    pub redeemer_stake_pkh: Option<Ed25519KeyHash>,
}

impl TryFromPData for OnChainLimitSwapConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let stake_pkh: Option<Ed25519KeyHash> = cpd
            .take_field(7)
            .clone()
            .and_then(|pd| pd.into_constr_pd())
            .and_then(|mut cpd_spkh| cpd_spkh.take_field(0))
            .and_then(|pd| pd.into_bytes())
            .and_then(|bytes| <[u8; 28]>::try_from(bytes).ok())
            .map(|bytes| Ed25519KeyHash::from(bytes));

        Some(OnChainLimitSwapConfig {
            base: TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?,
            base_amount: TaggedAmount::try_from_pd(cpd.take_field(8)?)?,
            quote: TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?,
            min_quote_amount: TaggedAmount::try_from_pd(cpd.take_field(9)?)?,
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?,
            ex_fee_per_token_num: cpd.take_field(4)?.into_u64()?.into(),
            ex_fee_per_token_denom: cpd.take_field(5)?.into_u64()?.into(),
            redeemer_pkh: Ed25519KeyHash::from(<[u8; 28]>::try_from(cpd.take_field(6)?.into_bytes()?).ok()?),
            redeemer_stake_pkh: stake_pkh,
        })
    }
}

#[cfg(test)]
mod tests {
    use cml_chain::plutus::PlutusData;
    use cml_chain::Deserialize;

    use spectrum_cardano_lib::types::TryFromPData;

    use crate::data::limit_swap::OnChainLimitSwapConfig;

    #[test]
    fn parse_swap_datum_mainnet() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM_SAMPLE).unwrap()).unwrap();
        let maybe_conf = OnChainLimitSwapConfig::try_from_pd(pd);
        assert!(maybe_conf.is_some())
    }

    const DATUM_SAMPLE: &str =
        "d8799fd8799f581c95a427e384527065f2f8946f5e86320d0117839a5e98ea2c0b55fb004448554e54ffd8799f\
        4040ffd8799f581ce08fbaa73db55294b3b31f2a365be5c4b38211a47880f0ef6b17a1604c48554e545f4144415\
        f4e4654ff1903e51b00148f1c351223aa1b8ac7230489e80000581c022835b77a25d6bf00f8cbf7e4744e0065ec\
        77383500221ed4f32514d8799f581c3cc6ea3784eecc03bc736d90e368abb40f873c48d1fc74133afae5a5ff1b0\
        0000003882d614c1a9a800dc5ff";
}
