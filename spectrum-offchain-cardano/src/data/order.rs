use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};

use cml_chain::builders::tx_builder::{SignedTxBuilder, TransactionBuilderConfig};
use cml_chain::plutus::PlutusData;
use cml_chain::transaction::TransactionOutput;

use spectrum_cardano_lib::plutus_data::RequiresRedeemer;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::backlog::data::{OrderWeight, Weighted};
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::{SpecializedOrder, UniqueOrder};
use spectrum_offchain::executor::{RunOrder, RunOrderError};
use spectrum_offchain::ledger::TryFromLedger;

use crate::data::limit_swap::ClassicalOnChainLimitSwap;
use crate::data::pool::CFMMPool;
use crate::data::{OnChain, OnChainOrderId, PoolId};

pub struct Base;

pub struct Quote;

pub struct PoolNft;

#[derive(Debug, Clone)]
pub struct ClassicalOrder<Id, Ord> {
    pub id: Id,
    pub pool_id: PoolId,
    pub order: Ord,
}

pub enum ClassicalOrderAction {
    Apply,
    Refund,
}

#[derive(Debug, Clone)]
pub enum ClassicalOnChainOrder {
    Swap(OnChain<ClassicalOnChainLimitSwap>),
}

impl PartialEq for ClassicalOnChainOrder {
    fn eq(&self, other: &Self) -> bool {
        <Self as UniqueOrder>::get_self_ref(self).eq(&<Self as UniqueOrder>::get_self_ref(other))
    }
}

impl Eq for ClassicalOnChainOrder {}

impl Hash for ClassicalOnChainOrder {
    fn hash<H: Hasher>(&self, state: &mut H) {
        <Self as UniqueOrder>::get_self_ref(self).hash(state)
    }
}

impl Display for ClassicalOnChainOrder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClassicalLimitSwap")
    }
}

impl Weighted for ClassicalOnChainOrder {
    fn weight(&self) -> OrderWeight {
        todo!()
    }
}

impl SpecializedOrder for ClassicalOnChainOrder {
    type TOrderId = OnChainOrderId;
    type TPoolId = PoolId;
    fn get_self_ref(&self) -> Self::TOrderId {
        match self {
            ClassicalOnChainOrder::Swap(limit_swap) => limit_swap.value.id,
        }
    }
    fn get_pool_ref(&self) -> Self::TPoolId {
        match self {
            ClassicalOnChainOrder::Swap(limit_swap) => limit_swap.value.pool_id,
        }
    }
}

impl RequiresRedeemer<ClassicalOrderAction> for ClassicalOnChainOrder {
    fn redeemer(action: ClassicalOrderAction) -> PlutusData {
        todo!()
    }
}

impl TryFromLedger<TransactionOutput, OutputRef> for ClassicalOnChainOrder {
    fn try_from_ledger(repr: TransactionOutput, ctx: OutputRef) -> Option<Self> {
        ClassicalOnChainLimitSwap::try_from_ledger(repr.clone(), ctx).map(|r| {
            ClassicalOnChainOrder::Swap(OnChain {
                value: r,
                source: repr,
            })
        })
    }
}

impl RunOrder<ClassicalOnChainOrder, TransactionBuilderConfig, SignedTxBuilder> for OnChain<CFMMPool> {
    fn try_run(
        self,
        order: ClassicalOnChainOrder,
        ctx: TransactionBuilderConfig,
    ) -> Result<(SignedTxBuilder, Predicted<Self>), RunOrderError<ClassicalOnChainOrder>> {
        match order {
            ClassicalOnChainOrder::Swap(limit_swap) => <Self as RunOrder<
                OnChain<ClassicalOnChainLimitSwap>,
                TransactionBuilderConfig,
                SignedTxBuilder,
            >>::try_run(self, limit_swap, ctx)
            .map_err(|err| err.map(|inner| ClassicalOnChainOrder::Swap(inner))),
        }
    }
}
