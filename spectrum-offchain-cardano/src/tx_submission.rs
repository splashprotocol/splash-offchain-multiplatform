use std::collections::HashSet;
use std::fmt::{Display, Formatter};
use std::ops::Deref;

use async_stream::stream;
use cml_core::serialization::Serialize;
use futures::channel::{mpsc, oneshot};
use futures::{SinkExt, Stream, StreamExt};
use log::{trace, warn};
use pallas_network::miniprotocols::localtxsubmission;
use pallas_network::miniprotocols::localtxsubmission::cardano_node_errors::{
    AlonzoUtxoPredFailure, AlonzoUtxowPredFailure, ApplyTxError, BabbageUtxoPredFailure,
    BabbageUtxowPredFailure, ShelleyLedgerPredFailure, ShelleyUtxowPredFailure, TxInput,
};
use pallas_network::miniprotocols::localtxsubmission::Response;
use pallas_network::multiplexer;

use cardano_submit_api::client::{Error, LocalTxSubmissionClient};
use pallas_primitives::conway::Value;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::network::Network;
use spectrum_offchain::tx_hash::CanonicalHash;

use crate::node::NodeConfig;

pub struct TxSubmissionAgent<'a, const ERA: u16, TxAdapter, Tx> {
    client: LocalTxSubmissionClient<'a, ERA, Tx>,
    mailbox: mpsc::Receiver<SubmitTx<TxAdapter>>,
    node_config: NodeConfig<'a>,
}

impl<'a, const ERA: u16, TxAdapter, Tx> TxSubmissionAgent<'a, ERA, TxAdapter, Tx> {
    pub async fn new(
        node_config: NodeConfig<'a>,
        buffer_size: usize,
    ) -> Result<(Self, TxSubmissionChannel<ERA, TxAdapter>), Error> {
        let tx_submission_client = LocalTxSubmissionClient::init(node_config.path, node_config.magic).await?;
        let (snd, recv) = mpsc::channel(buffer_size);
        let agent = Self {
            client: tx_submission_client,
            mailbox: recv,
            node_config,
        };
        Ok((agent, TxSubmissionChannel(snd)))
    }

    pub fn recover(&mut self) {
        self.client.unsafe_reset();
    }

    pub async fn restarted(self) -> Result<Self, Error> {
        let TxSubmissionAgent {
            client,
            mailbox,
            node_config,
        } = self;
        client.close().await;
        let new_tx_submission_client =
            LocalTxSubmissionClient::init(node_config.path, node_config.magic).await?;
        Ok(Self {
            client: new_tx_submission_client,
            mailbox,
            node_config,
        })
    }
}

#[derive(Clone)]
pub struct TxSubmissionChannel<const ERA: u16, Tx>(mpsc::Sender<SubmitTx<Tx>>);

pub struct SubmitTx<Tx>(Tx, oneshot::Sender<SubmissionResult>);

#[derive(Debug, Clone)]
pub enum SubmissionResult {
    Ok,
    TxRejected { errors: RejectReasons },
}

impl From<SubmissionResult> for Result<(), RejectReasons> {
    fn from(value: SubmissionResult) -> Self {
        match value {
            SubmissionResult::Ok => Ok(()),
            SubmissionResult::TxRejected { errors } => Err(errors.into()),
        }
    }
}

const MAX_SUBMIT_ATTEMPTS: usize = 3;

pub fn tx_submission_agent_stream<'a, const ERA: u16, TxAdapter, Tx>(
    mut agent: TxSubmissionAgent<'a, ERA, TxAdapter, Tx>,
) -> impl Stream<Item = ()> + 'a
where
    TxAdapter: Deref<Target = Tx> + CanonicalHash + 'a,
    TxAdapter::Hash: Display,
    Tx: Serialize + Clone + 'a,
{
    stream! {
        loop {
            let SubmitTx(tx, on_resp) = agent.mailbox.select_next_some().await;
            let mut attempts_done = 0;
            let tx_hash = tx.canonical_hash();
            loop {
                match agent.client.submit_tx((*tx).clone()).await {
                    Ok(Response::Accepted) => on_resp.send(SubmissionResult::Ok).expect("Responder was dropped"),
                    Ok(Response::Rejected(errors)) => {
                        trace!("TX {} was rejected due to error: {:?}", tx_hash, errors);
                        let errors: Vec<ProtocolRejectReason> = errors.into_iter().flat_map(|ApplyTxError { node_errors }|
                            node_errors.into_iter().map(ProtocolRejectReason::from)).collect();
                        on_resp.send(SubmissionResult::TxRejected{errors: errors.into()}).expect("Responder was dropped");
                    },
                    Err(Error::TxSubmissionProtocol(err)) => {
                        trace!("Failed to submit TX {}: {}", tx_hash, hex::encode(tx.to_cbor_bytes()));
                        match err {
                            localtxsubmission::Error::ChannelError(multiplexer::Error::Decoding(_)) => {
                                warn!("TX {} was likely rejected, reason unknown. Trying to recover.", tx_hash);
                                agent.recover();
                                on_resp.send(SubmissionResult::TxRejected{errors: vec![].into()}).expect("Responder was dropped");
                            }
                            retryable_err => {
                                trace!("Failed to submit TX {}: protocol returned error: {}", tx_hash, retryable_err);
                                if attempts_done < MAX_SUBMIT_ATTEMPTS {
                                    trace!("Retrying");
                                    attempts_done += 1;
                                } else {
                                    trace!("Restarting TxSubmissionProtocol");
                                    agent = agent.restarted().await.expect("Failed to restart TxSubmissionProtocol");
                                }
                                continue;
                            },
                        };
                    },
                    Err(err) => panic!("Cannot submit TX {} due to {}", tx_hash, err),
                }
                break;
            }
        }
    }
}

impl TryFrom<RejectReasons> for HashSet<OutputRef> {
    type Error = &'static str;
    fn try_from(value: RejectReasons) -> Result<Self, Self::Error> {
        let mut outputs = HashSet::new();

        for reason in value.0 {
            if let ProtocolRejectReason::BadInputsUtxo(inputs) = reason {
                let o = inputs.into_iter().map(|TxInput { tx_hash, index }| {
                    let tx_hash = *tx_hash;
                    OutputRef::new(tx_hash.into(), index)
                });
                outputs.extend(o);
            }
        }

        if !outputs.is_empty() {
            Ok(outputs)
        } else {
            Err("No missing inputs")
        }
    }
}

#[async_trait::async_trait]
impl<const ERA: u16, Tx> Network<Tx, RejectReasons> for TxSubmissionChannel<ERA, Tx>
where
    Tx: Send,
{
    async fn submit_tx(&mut self, tx: Tx) -> Result<(), RejectReasons> {
        let (snd, recv) = oneshot::channel();
        self.0.send(SubmitTx(tx, snd)).await.unwrap();
        recv.await.expect("Channel closed").into()
    }
}

#[derive(Debug, Clone, derive_more::Display, derive_more::From)]
#[display(fmt = "RejectReasons: {:?}", "_0")]
pub struct RejectReasons(pub Vec<ProtocolRejectReason>);

#[derive(Debug, Clone, derive_more::Display)]
pub enum ProtocolRejectReason {
    /// Missing/bad TX inputs
    #[display(fmt = "ProtocolRejectReason::BadInputsUtxo: {:?}", "_0")]
    BadInputsUtxo(Vec<TxInput>),
    #[display(
        fmt = "ProtocolRejectReason::ValueNotConserved: Consumed {:?}, produced {:?}",
        consumed,
        produced
    )]
    ValueNotConserved { consumed: Value, produced: Value },
    #[display(fmt = "ProtocolRejectReason::MissingVKeyWitnesses: {:?}", "_0")]
    MissingVKeyWitnesses(Vec<pallas_crypto::hash::Hash<28>>),
    #[display(fmt = "ProtocolRejectReason::MissingScriptWitnesses: {:?}", "_0")]
    MissingScriptWitnesses(Vec<pallas_crypto::hash::Hash<28>>),
    /// A generic error that cannot be clearly identified.
    Unknown(String),
}

impl From<ShelleyLedgerPredFailure> for ProtocolRejectReason {
    fn from(value: ShelleyLedgerPredFailure) -> Self {
        match value {
            ShelleyLedgerPredFailure::UtxowFailure(babbage_failure) => match babbage_failure {
                BabbageUtxowPredFailure::AlonzoInBabbageUtxowPredFailure(alonzo_fail) => match alonzo_fail {
                    AlonzoUtxowPredFailure::ShelleyInAlonzoUtxowPredfailure(shelley_fail) => {
                        match shelley_fail {
                            ShelleyUtxowPredFailure::MissingVKeyWitnessesUTXOW(m) => {
                                ProtocolRejectReason::MissingVKeyWitnesses(m)
                            }
                            ShelleyUtxowPredFailure::MissingScriptWitnessesUTXOW(m) => {
                                ProtocolRejectReason::MissingScriptWitnesses(m)
                            }
                            ShelleyUtxowPredFailure::InvalidWitnessesUTXOW => unimplemented!(),
                            ShelleyUtxowPredFailure::ScriptWitnessNotValidatingUTXOW(_) => {
                                unimplemented!()
                            }
                            ShelleyUtxowPredFailure::UtxoFailure => unimplemented!(),
                            ShelleyUtxowPredFailure::MIRInsufficientGenesisSigsUTXOW => {
                                unimplemented!()
                            }
                            ShelleyUtxowPredFailure::MissingTxBodyMetadataHash => {
                                unimplemented!()
                            }
                            ShelleyUtxowPredFailure::MissingTxMetadata => unimplemented!(),
                            ShelleyUtxowPredFailure::ConflictingMetadataHash => {
                                unimplemented!()
                            }
                            ShelleyUtxowPredFailure::InvalidMetadata => unimplemented!(),
                            ShelleyUtxowPredFailure::ExtraneousScriptWitnessesUTXOW(_) => {
                                unimplemented!()
                            }
                        }
                    }
                    AlonzoUtxowPredFailure::MissingRedeemers => unimplemented!(),
                    AlonzoUtxowPredFailure::MissingRequiredDatums => unimplemented!(),
                    AlonzoUtxowPredFailure::NotAllowedSupplementalDatums => unimplemented!(),
                    AlonzoUtxowPredFailure::PPViewHashesDontMatch => unimplemented!(),
                    AlonzoUtxowPredFailure::MissingRequiredSigners(_) => unimplemented!(),
                    AlonzoUtxowPredFailure::UnspendableUtxoNoDatumHash => unimplemented!(),
                    AlonzoUtxowPredFailure::ExtraRedeemers => unimplemented!(),
                },
                BabbageUtxowPredFailure::UtxoFailure(utxo_fail) => match utxo_fail {
                    BabbageUtxoPredFailure::AlonzoInBabbageUtxoPredFailure(alonzo_fail) => {
                        match alonzo_fail {
                            AlonzoUtxoPredFailure::BadInputsUtxo(inputs) => {
                                ProtocolRejectReason::BadInputsUtxo(inputs)
                            }
                            AlonzoUtxoPredFailure::ValueNotConservedUTxO {
                                consumed_value,
                                produced_value,
                            } => ProtocolRejectReason::ValueNotConserved {
                                consumed: consumed_value,
                                produced: produced_value,
                            },
                            AlonzoUtxoPredFailure::OutsideValidityIntervalUTxO => unimplemented!(),
                            AlonzoUtxoPredFailure::MaxTxSizeUTxO => unimplemented!(),
                            AlonzoUtxoPredFailure::InputSetEmptyUTxO => unimplemented!(),
                            AlonzoUtxoPredFailure::FeeTooSmallUTxO => unimplemented!(),
                            AlonzoUtxoPredFailure::WrongNetwork => unimplemented!(),
                            AlonzoUtxoPredFailure::WrongNetworkWithdrawal => unimplemented!(),
                            AlonzoUtxoPredFailure::OutputTooSmallUTxO => unimplemented!(),
                            AlonzoUtxoPredFailure::UtxosFailure(failure) => unimplemented!(),
                            AlonzoUtxoPredFailure::OutputBootAddrAttrsTooBig => unimplemented!(),
                            AlonzoUtxoPredFailure::TriesToForgeADA => unimplemented!(),
                            AlonzoUtxoPredFailure::OutputTooBigUTxO => unimplemented!(),
                            AlonzoUtxoPredFailure::InsufficientCollateral => unimplemented!(),
                            AlonzoUtxoPredFailure::ScriptsNotPaidUTxO => unimplemented!(),
                            AlonzoUtxoPredFailure::ExUnitsTooBigUTxO => unimplemented!(),
                            AlonzoUtxoPredFailure::CollateralContainsNonADA => unimplemented!(),
                            AlonzoUtxoPredFailure::WrongNetworkInTxBody => unimplemented!(),
                            AlonzoUtxoPredFailure::OutsideForecast => unimplemented!(),
                            AlonzoUtxoPredFailure::TooManyCollateralInputs => unimplemented!(),
                            AlonzoUtxoPredFailure::NoCollateralInputs => unimplemented!(),
                        }
                    }
                    BabbageUtxoPredFailure::IncorrectTotalCollateralField => unimplemented!(),
                    BabbageUtxoPredFailure::BabbageOutputTooSmallUTxO => unimplemented!(),
                    BabbageUtxoPredFailure::BabbageNonDisjointRefInputs => unimplemented!(),
                },
                BabbageUtxowPredFailure::MalformedScriptWitnesses => unimplemented!(),
                BabbageUtxowPredFailure::MalformedReferenceScripts => unimplemented!(),
            },
            ShelleyLedgerPredFailure::DelegsFailure => unimplemented!(),
        }
    }
}
