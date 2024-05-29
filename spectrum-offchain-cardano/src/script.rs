use std::collections::HashMap;

use cml_chain::plutus::PlutusData;
use cml_crypto::ScriptHash;

use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::OutputRef;

pub struct ScriptWitness {
    pub hash: ScriptHash,
    pub cost: DelayedScriptCost,
}

pub struct TxInputsOrdering(HashMap<OutputRef, usize>);

impl TxInputsOrdering {
    pub fn new(ordering: HashMap<OutputRef, usize>) -> TxInputsOrdering {
        Self(ordering)
    }

    pub fn index_of(&self, input: &OutputRef) -> usize {
        *self
            .0
            .get(input)
            .expect("Input must be present in final transaction")
    }
}

pub struct ScriptContextPreview {
    pub self_index: usize,
}

pub enum DelayedScriptCost {
    Ready(ExUnits),
    Delayed(Box<dyn FnOnce(&ScriptContextPreview) -> ExUnits>),
}

impl DelayedScriptCost {
    pub fn compute(self, ctx: &ScriptContextPreview) -> ExUnits {
        match self {
            DelayedScriptCost::Ready(cost) => cost,
            DelayedScriptCost::Delayed(closure) => closure(ctx),
        }
    }
}

pub fn ready_cost(r: ExUnits) -> DelayedScriptCost {
    DelayedScriptCost::Ready(r)
}

pub fn delayed_cost(f: impl FnOnce(&ScriptContextPreview) -> ExUnits + 'static) -> DelayedScriptCost {
    DelayedScriptCost::Delayed(Box::new(f))
}

pub enum DelayedRedeemer {
    Ready(PlutusData),
    Delayed(Box<dyn FnOnce(&TxInputsOrdering) -> PlutusData>),
}

impl DelayedRedeemer {
    pub fn compute(self, inputs_ordering: &TxInputsOrdering) -> PlutusData {
        match self {
            DelayedRedeemer::Ready(pd) => pd,
            DelayedRedeemer::Delayed(closure) => closure(inputs_ordering),
        }
    }
}

pub fn ready_redeemer(r: PlutusData) -> DelayedRedeemer {
    DelayedRedeemer::Ready(r)
}

pub fn delayed_redeemer(f: impl FnOnce(&TxInputsOrdering) -> PlutusData + 'static) -> DelayedRedeemer {
    DelayedRedeemer::Delayed(Box::new(f))
}
