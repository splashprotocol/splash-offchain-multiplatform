use std::collections::HashMap;

use cml_chain::plutus::PlutusData;
use cml_crypto::ScriptHash;
use log::info;
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
        info!(
            "going to get index of {} in {}",
            input,
            self.0
                .iter()
                .map(|res| format!("ORef({}) -> idx {},", res.0, res.1))
                .collect::<Vec<String>>()
                .iter()
                .fold(String::new(), |mut acc, to_add| {
                    acc.push_str(to_add);
                    acc
                })
        );
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
        info!("Computing redeemer");
        let res = match self {
            DelayedRedeemer::Ready(pd) => {
                info!("Redeemer is ready");
                pd
            }
            DelayedRedeemer::Delayed(closure) => {
                info!("Redeemer is delayed - computing");
                let res = closure(inputs_ordering);
                info!("Redeemer is delayed - computed");
                res
            }
        };
        info!("After computing redeemer");
        res
    }
}

pub fn ready_redeemer(r: PlutusData) -> DelayedRedeemer {
    DelayedRedeemer::Ready(r)
}

pub fn delayed_redeemer(f: impl FnOnce(&TxInputsOrdering) -> PlutusData + 'static) -> DelayedRedeemer {
    DelayedRedeemer::Delayed(Box::new(f))
}
