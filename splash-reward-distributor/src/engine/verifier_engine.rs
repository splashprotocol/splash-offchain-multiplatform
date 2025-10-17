use std::future::Future;
use std::pin::Pin;
use std::{future::poll_fn, marker::PhantomData, task::Poll};

use async_primitives::beacon::{Beacon, Once};
use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use cml_chain::transaction::Transaction;
use futures::Stream;
use futures::StreamExt;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::{deployment::ProtocolValidator, protocol_config::SplashPolicy};
use splash_yf_offchain::{events::OnChainEvent, settings::MinLovelacePerHarvest};
use tokio::sync::oneshot;
use tokio::time::Sleep;

use crate::engine::verifier::{
    AuthorizedExecutors, LocalVerifier, TxCosignRequest, VerifierHandleLedgerEvent,
};

pub struct VerifierEngine<U, R, Verifier, Ctx> {
    ledger_event_stream: U,
    cosign_request_stream: R,
    local_verifier: Verifier,
    state_synced: Beacon,
    blocker: Option<Once>,
    initial_tx_ttl_delay: Option<Pin<Box<Sleep>>>,
    pd: PhantomData<Ctx>,
}

impl<U, R, Verifier, Ctx> VerifierEngine<U, R, Verifier, Ctx> {
    pub fn new(
        ledger_event_stream: U,
        cosign_request_stream: R,
        local_verifier: Verifier,
        state_synced: Beacon,
        initial_tx_ttl_delay: Sleep,
    ) -> Self {
        Self {
            ledger_event_stream,
            cosign_request_stream,
            local_verifier,
            state_synced,
            blocker: None,
            initial_tx_ttl_delay: Some(Box::pin(initial_tx_ttl_delay)),
            pd: PhantomData,
        }
    }
}

impl<GaugeId, StateId, Bearer, U, R, Verifier, Ctx> VerifierEngine<U, R, Verifier, Ctx>
where
    GaugeId: Copy + Unpin + Send + 'static,
    StateId: Copy + Unpin + Send + 'static,
    Bearer: Unpin + Send + 'static,
    U: Stream<
            Item = (
                BlockEvents<OnChainEvent<GaugeId, StateId, Bearer>>,
                TransactionHandle,
            ),
        > + Unpin,
    R: Stream<Item = (TxCosignRequest, oneshot::Sender<Option<Transaction>>)> + Unpin,
    Verifier:
        VerifierHandleLedgerEvent + LocalVerifier<TxCosignRequest, Transaction, Ctx> + Unpin + Send + 'static,
    Ctx: Has<MinLovelacePerHarvest>
        + Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::BufferWallet as u8 }>>
        + Has<NetworkId>
        + Has<SplashPolicy>
        + Has<AuthorizedExecutors>
        + Sync,
{
    pub async fn run(mut self, ctx: Ctx) {
        enum T<GaugeId, StateId, Bearer> {
            Ledger(
                BlockEvents<OnChainEvent<GaugeId, StateId, Bearer>>,
                TransactionHandle,
            ),
            Cosign(TxCosignRequest, oneshot::Sender<Option<Transaction>>),
        }
        loop {
            // Use poll_fn to manually poll both streams with bias toward ledger events (note that
            // tokio::select! isn't appropriate here because the macro `.awaits` on multiple futures
            // at the top level. This doesn't work for us because we need to inspect Option<..>
            // values.
            let result = poll_fn(|cx| {
                // First try ledger events (biased)
                if let Poll::Ready(Some((event, tx))) = self.ledger_event_stream.poll_next_unpin(cx) {
                    return Poll::Ready(Some(T::Ledger(event, tx)));
                }

                // Wait until initial tx TTL delay is resolved (CompleteDataLoss stressor).
                if let Some(mut initial_tx_ttl_delay) = self.initial_tx_ttl_delay.take() {
                    if Future::poll(Pin::new(&mut initial_tx_ttl_delay), cx).is_pending() {
                        self.initial_tx_ttl_delay = Some(initial_tx_ttl_delay);
                        return Poll::Ready(None);
                    }
                }

                // Wait until blockers are resolved.
                if let Some(mut blocker) = self.blocker.take() {
                    if Future::poll(Pin::new(&mut blocker), cx).is_pending() {
                        self.blocker = Some(blocker);
                        return Poll::Ready(None);
                    }
                }

                if !self.state_synced.read() {
                    self.blocker = Some(self.state_synced.once(true));
                    return Poll::Ready(None);
                }

                // Then try cosign requests
                if let Poll::Ready(Some((tx, sender))) = self.cosign_request_stream.poll_next_unpin(cx) {
                    return Poll::Ready(Some(T::Cosign(tx, sender)));
                }

                Poll::Pending
            })
            .await;

            match result {
                Some(T::Ledger(event, tx)) => {
                    process_ledger_event((event, tx), &mut self.local_verifier).await;
                }
                Some(T::Cosign(tx, sender)) => {
                    sender
                        .send(self.local_verifier.try_approve(&tx, &ctx).await)
                        .unwrap();
                }
                None => (),
            }
        }
    }
}

async fn process_ledger_event<GaugeId, StateId, Bearer, Verifier>(
    event: (
        BlockEvents<OnChainEvent<GaugeId, StateId, Bearer>>,
        TransactionHandle,
    ),
    verifier: &mut Verifier,
) where
    Verifier: VerifierHandleLedgerEvent,
{
    let (events, tx) = event;
    match events {
        BlockEvents::RollForward {
            events, block_slot, ..
        } => {
            verifier.confirm_block_slot(block_slot);
            for event in events {
                match event {
                    OnChainEvent::BotHarvestingAction { payouts, tx_hash, .. } => {
                        let confirmed_user_harvests = payouts
                            .iter()
                            .map(|(harvest_order, _)| harvest_order.account_key)
                            .collect::<Vec<_>>();
                        verifier.confirm_harvest_tx(tx_hash, &confirmed_user_harvests);
                    }
                    OnChainEvent::BotGaugeBufferingAction { tx_hash, .. } => {
                        verifier.confirm_gauge_buffering_tx(tx_hash);
                    }
                    _ => (),
                }
            }
        }
        BlockEvents::RollBackward {
            events, block_slot, ..
        } => {
            verifier.rollback_block_slot(block_slot);
            for event in events {
                match event {
                    OnChainEvent::BotHarvestingAction { payouts, tx_hash, .. } => {
                        let user_harvests_to_remove = payouts
                            .iter()
                            .map(|(harvest_order, _)| harvest_order.account_key)
                            .collect::<Vec<_>>();
                        verifier.rollback_harvest_tx(tx_hash, user_harvests_to_remove);
                    }
                    OnChainEvent::BotGaugeBufferingAction { tx_hash, .. } => {
                        verifier.rollback_gauge_buffering_tx(tx_hash);
                    }
                    _ => (),
                }
            }
        }
    }
    tx.commit();
}
