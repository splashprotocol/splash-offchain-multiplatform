use std::collections::HashMap;

use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::TransactionBuilder;
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::{PlutusData, RedeemerTag};
use cml_chain::transaction::TransactionOutput;
use cml_chain::Value;

use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, OutputRef};
use spectrum_offchain_cardano::deployment::DeployedValidatorErased;

pub struct TxInputsOrdering(HashMap<OutputRef, usize>);

impl TxInputsOrdering {
    pub fn index_of(&self, input: &OutputRef) -> usize {
        *self
            .0
            .get(input)
            .expect("Input must be present in final transaction")
    }
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

pub struct ScriptInputBlueprint {
    pub reference: OutputRef,
    pub utxo: TransactionOutput,
    pub script: DeployedValidatorErased,
    pub redeemer: DelayedRedeemer,
}

pub fn ready_redeemer(r: PlutusData) -> DelayedRedeemer {
    DelayedRedeemer::Ready(r)
}

pub fn delayed_redeemer(f: impl FnOnce(&TxInputsOrdering) -> PlutusData + 'static) -> DelayedRedeemer {
    DelayedRedeemer::Delayed(Box::new(f))
}

/// Blueprint of DEX transaction.
/// Accumulates information that is later used to create real transaction
/// that executes a batch of DEX operations.
pub struct TxBlueprint {
    pub script_io: Vec<(ScriptInputBlueprint, TransactionOutput)>,
}

impl TxBlueprint {
    pub fn new() -> Self {
        Self {
            script_io: Vec::new(),
        }
    }

    pub fn add_io(&mut self, input: ScriptInputBlueprint, output: TransactionOutput) {
        self.script_io.push((input, output));
    }

    pub fn apply_to_builder(self, mut txb: TransactionBuilder) -> TransactionBuilder {
        let mut sorted_io = self.script_io;
        sorted_io.sort_by(|(left_in, _), (right_in, _)| left_in.reference.cmp(&right_in.reference));
        let enumerated_io = sorted_io.into_iter().enumerate().collect::<Vec<_>>();
        let inputs_ordering = TxInputsOrdering(HashMap::from_iter(
            enumerated_io.iter().map(|(ix, (i, _))| (i.reference, *ix)),
        ));
        for (
            ix,
            (
                ScriptInputBlueprint {
                    reference,
                    utxo,
                    script,
                    redeemer,
                },
                output,
            ),
        ) in enumerated_io
        {
            let cml_script = PartialPlutusWitness::new(
                PlutusScriptWitness::Ref(script.hash),
                redeemer.compute(&inputs_ordering),
            );
            let input = SingleInputBuilder::new(reference.into(), utxo)
                .plutus_script_inline_datum(cml_script, Vec::new())
                .unwrap();
            let output = SingleOutputBuilderResult::new(output);
            txb.add_reference_input(script.reference_utxo);
            txb.add_input(input).expect("add_input ok");
            txb.add_output(output).expect("add_output ok");
            txb.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, ix as u64),
                script.ex_budget.into(),
            );
        }
        txb
    }
}

pub struct ExecutionState {
    pub tx_blueprint: TxBlueprint,
    pub execution_budget_acc: Value,
}

impl ExecutionState {
    pub fn new() -> Self {
        Self {
            tx_blueprint: TxBlueprint::new(),
            execution_budget_acc: Value::zero(),
        }
    }

    pub fn add_ex_budget(&mut self, ac: AssetClass, amount: u64) {
        self.execution_budget_acc.add_unsafe(ac.into(), amount);
    }
}
