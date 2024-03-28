use std::collections::{HashMap, HashSet};

use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{TransactionBuilder, TransactionUnspentOutput};
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::{PlutusData, RedeemerTag};
use cml_chain::transaction::{TransactionInput, TransactionOutput};
use cml_chain::Value;

use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, OutputRef};
use spectrum_offchain_cardano::deployment::{DeployedValidatorErased, ScriptWitness};

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
    pub script: ScriptWitness,
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
    pub reference_inputs: HashSet<(TransactionInput, TransactionOutput)>,
}

impl TxBlueprint {
    pub fn new() -> Self {
        Self {
            script_io: Vec::new(),
            reference_inputs: HashSet::new(),
        }
    }

    pub fn add_io(&mut self, input: ScriptInputBlueprint, output: TransactionOutput) {
        self.script_io.push((input, output));
    }

    pub fn add_ref_input(&mut self, utxo: TransactionUnspentOutput) {
        self.reference_inputs.insert((utxo.input, utxo.output));
    }

    pub fn apply_to_builder(self, mut txb: TransactionBuilder) -> TransactionBuilder {
        let mut sorted_io = self.script_io;
        sorted_io.sort_by(|(left_in, _), (right_in, _)| left_in.reference.cmp(&right_in.reference));
        let enumerated_io = sorted_io.into_iter().enumerate().collect::<Vec<_>>();
        let inputs_ordering = TxInputsOrdering(HashMap::from_iter(
            enumerated_io.iter().map(|(ix, (i, _))| (i.reference, *ix)),
        ));
        for (ref_in, ref_utxo) in self.reference_inputs {
            txb.add_reference_input(TransactionUnspentOutput::new(ref_in, ref_utxo));
        }
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

#[cfg(test)]
mod test {
    use cml_chain::plutus::PlutusV2Script;
    use cml_core::serialization::Deserialize;

    #[test]
    fn hash_script_cml() {
        let script = PlutusV2Script::from_cbor_bytes(&*hex::decode(SCRIPT).unwrap()).unwrap();
        let sh = script.hash();
        println!("{}", sh)
    }
    const SCRIPT: &str = "59041459041101000033232323232323232322222323253330093232533300b003132323300100100222533301100114a02646464a66602266ebc0380045288998028028011808801180a80118098009bab301030113011301130113011301130090011323232533300e3370e900118068008991919299980899b8748000c0400044c8c8c8c8c94ccc0594ccc05802c400852808008a503375e601860260046034603660366036603660366036603660366036602602266ebcc020c048c020c048008c020c048004c060dd6180c180c980c9808804980b80098078008b19191980080080111299980b0008a60103d87a80001323253330153375e6018602600400c266e952000330190024bd70099802002000980d001180c0009bac3007300e0063014001300c001163001300b0072301230130013322323300100100322533301200114a026464a66602266e3c008014528899802002000980b0011bae3014001375860206022602260226022602260226022602260120026eb8c040c044c044c044c044c044c044c044c044c044c044c02401cc004c0200108c03c004526136563370e900118049baa003323232533300a3370e90000008991919191919191919191919191919191919191919191919299981298140010991919191924c646600200200c44a6660560022930991980180198178011bae302d0013253330263370e9000000899191919299981698180010991924c64a66605866e1d20000011323253330313034002132498c94ccc0bccdc3a400000226464a666068606e0042649318150008b181a80098168010a99981799b87480080044c8c8c8c8c8c94ccc0e0c0ec00852616375a607200260720046eb4c0dc004c0dc008dd6981a80098168010b18168008b181900098150018a99981619b874800800454ccc0bcc0a800c5261616302a002302300316302e001302e002302c00130240091630240083253330253370e9000000899191919299981618178010a4c2c6eb4c0b4004c0b4008dd6981580098118060b1811805980d806180d0098b1bac30260013026002375c60480026048004604400260440046eb4c080004c080008c078004c078008c070004c070008dd6980d000980d0011bad30180013018002375a602c002602c004602800260280046eb8c048004c048008dd7180800098040030b1804002919299980519b87480000044c8c8c8c94ccc044c05000852616375c602400260240046eb8c040004c02000858c0200048c94ccc024cdc3a400000226464a66601c60220042930b1bae300f0013007002153330093370e900100089919299980718088010a4c2c6eb8c03c004c01c00858c01c0048c014dd5000918019baa0015734aae7555cf2ab9f5740ae855d126126d8799fd87a9f581ce7feddaece029040c973d5bf806fa9497314c0a63dfdc47fc47ac557ffff0001";
}
