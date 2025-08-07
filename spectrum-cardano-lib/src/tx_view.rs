use crate::hash::hash_transaction_canonical;
use crate::transaction::TransactionOutputExtension;
use crate::OutputRef;
use cbor_event::Serialize;
use cml_chain::{
    transaction::{ConwayFormatTxOut, Transaction, TransactionInput, TransactionOutput},
    LenEncoding,
};
use cml_core::{serialization::CBORReadLen, DeserializeFailure, Slot};
use cml_crypto::{Ed25519KeyHash, TransactionHash};
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use spectrum_offchain::persistent_index::PersistentIndex;

/// A Tx view giving access to its mandatory fields, inputs are partially resolved.
#[derive(Debug, Clone)]
pub struct TxView {
    pub hash: TransactionHash,
    pub inputs: Vec<TransactionInput>,
    pub outputs: Vec<TransactionOutput>,
    pub signers: Vec<Ed25519KeyHash>,
}

impl From<Either<BabbageTransaction, Transaction>> for TxView {
    fn from(tx: Either<BabbageTransaction, Transaction>) -> Self {
        match tx {
            Either::Left(tx) => Self {
                hash: hash_transaction_canonical(&tx.body),
                inputs: tx.body.inputs.into(),
                outputs: tx
                    .body
                    .outputs
                    .into_iter()
                    .map(|out| {
                        TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
                            address: out.address().clone(),
                            amount: out.value().clone(),
                            datum_option: out.datum(),
                            script_reference: None,
                            encodings: None,
                        })
                    })
                    .collect(),
                signers: tx
                    .witness_set
                    .vkeywitnesses
                    .into_iter()
                    .flat_map(|witnesses| witnesses.into_iter().map(|witness| witness.vkey.hash()))
                    .collect(),
            },
            Either::Right(tx) => Self {
                hash: hash_transaction_canonical(&tx.body),
                inputs: tx.body.inputs.into(),
                outputs: tx.body.outputs,
                signers: tx
                    .witness_set
                    .vkeywitnesses
                    .into_iter()
                    .flat_map(|witnesses| witnesses.into_iter().map(|witness| witness.vkey.hash()))
                    .collect(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimedOutput {
    pub output: TransactionOutput,
    pub slot: Slot,
}

impl cml_crypto::Serialize for TimedOutput {
    fn serialize<'a, W: std::io::Write + Sized>(
        &self,
        serializer: &'a mut cbor_event::se::Serializer<W>,
        force_canonical: bool,
    ) -> cbor_event::Result<&'a mut cbor_event::se::Serializer<W>> {
        serializer.write_array_sz(LenEncoding::default().to_len_sz(2, force_canonical))?;
        self.output.serialize(serializer, force_canonical)?;
        self.slot.serialize(serializer)?;
        LenEncoding::default().end(serializer, force_canonical)
    }
}

impl cml_crypto::Deserialize for TimedOutput {
    fn deserialize<R: std::io::BufRead + std::io::Seek>(
        raw: &mut cbor_event::de::Deserializer<R>,
    ) -> Result<Self, cml_core::DeserializeError>
    where
        Self: Sized,
    {
        use cbor_event::Deserialize;

        let len = raw.array_sz()?;
        let mut read_len = CBORReadLen::new(len);
        read_len.read_elems(2)?;
        let output = TransactionOutput::deserialize(raw)?;
        let slot = Slot::deserialize(raw)?;
        match len {
            cbor_event::LenSz::Len(_, _) => read_len.finish()?,
            cbor_event::LenSz::Indefinite => match raw.special()? {
                cbor_event::Special::Break => read_len.finish()?,
                _ => return Err(DeserializeFailure::EndingBreakMissing.into()),
            },
        }
        Ok(Self { output, slot })
    }
}

/// A Tx view giving access to its mandatory fields, inputs are partially resolved.
#[derive(Debug, Clone)]
pub struct TxViewPartiallyResolved {
    pub hash: TransactionHash,
    pub inputs: Vec<(TransactionInput, Option<TimedOutput>)>,
    pub outputs: Vec<TransactionOutput>,
    pub signers: Vec<Ed25519KeyHash>,
    pub slot: u64,
}

impl TxViewPartiallyResolved {
    pub async fn resolve<Index: PersistentIndex<OutputRef, TimedOutput>>(
        tx: TxView,
        index: &Index,
        slot: Slot,
    ) -> Self {
        Self {
            hash: tx.hash,
            inputs: try_resolve_inputs(tx.inputs, index).await,
            outputs: tx.outputs,
            signers: tx.signers,
            slot,
        }
    }
}

async fn try_resolve_inputs<Index: PersistentIndex<OutputRef, TimedOutput>>(
    inputs: Vec<TransactionInput>,
    index: &Index,
) -> Vec<(TransactionInput, Option<TimedOutput>)> {
    let mut processed_inputs = vec![];
    for input in inputs {
        let maybe_output = index.get(OutputRef::new(input.transaction_id, input.index)).await;
        processed_inputs.push((input, maybe_output));
    }
    processed_inputs
}

#[cfg(test)]
mod tests {
    use cml_chain::{address::Address, transaction::TransactionOutput, Deserialize, Serialize, Value};

    use crate::tx_view::TimedOutput;

    #[test]
    fn test_timed_output_roundtrip() {
        let t = TimedOutput {
            output: TransactionOutput::new(
                Address::from_bech32("addr_test1wp8v2rexyjaxyppmaezyfz7fkwy059ewpde7l9xr4vhcp9qvrkvl0")
                    .unwrap(),
                Value::from(12345),
                None,
                None,
            ),
            slot: 1234567,
        };
        let bytes = t.to_cbor_bytes();
        assert_eq!(TimedOutput::from_cbor_bytes(&bytes).unwrap(), t);
    }
}
