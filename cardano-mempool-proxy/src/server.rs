use actix_cors::Cors;
use actix_web::dev::{AppService, HttpServiceFactory};
use actix_web::web::Data;
use actix_web::{guard, web, App, HttpResponse, HttpServer, Responder};
use cml_chain::Deserialize;
use futures::StreamExt;
use log::{info, trace};
use spectrum_offchain::network::Network;
use spectrum_offchain::tx_hash::CanonicalHash;
use spectrum_offchain_cardano::tx_submission::TxSubmissionChannel;
use std::fmt::Display;
use std::future::Future;
use std::io;
use std::marker::PhantomData;
use std::net::SocketAddr;

#[derive(Copy, Clone, serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    max_payload_len_bytes: usize,
}

pub struct SubmitTx<const ERA: u16, Tx>(PhantomData<Tx>);

impl<const ERA: u16, Tx> HttpServiceFactory for SubmitTx<ERA, Tx>
where
    Tx: CanonicalHash + Deserialize + Send + 'static,
    Tx::Hash: Display,
{
    fn register(self, config: &mut AppService) {
        async fn submit_tx<const ERA: u16, Tx>(
            limits: Data<Limits>,
            tx_submission: Data<TxSubmissionChannel<ERA, Tx>>,
            mut body: web::Payload,
        ) -> impl Responder
        where
            Tx: CanonicalHash + Deserialize + Send,
            Tx::Hash: Display,
        {
            let mut bytes = web::BytesMut::new();
            while let Some(item) = body.next().await {
                if let Ok(item) = item {
                    if bytes.len() > limits.max_payload_len_bytes {
                        return HttpResponse::PayloadTooLarge().finish();
                    }
                    bytes.extend_from_slice(&item);
                } else {
                    return HttpResponse::InternalServerError().finish();
                }
            }
            match Tx::from_cbor_bytes(&*bytes) {
                Ok(tx) => {
                    let hash = tx.canonical_hash();
                    info!("Submitting Tx {}", hash);
                    let mut channel = tx_submission.get_ref().clone();
                    match channel.submit_tx(tx).await {
                        Ok(_) => HttpResponse::Ok().body(hash.to_string()),
                        Err(err) => HttpResponse::BadRequest().body(err.to_string()),
                    }
                }
                Err(_) => HttpResponse::BadRequest().body("invalid cbor"),
            }
        }
        let resource = actix_web::Resource::new("/tx/submit")
            .name("tx-submit")
            .guard(guard::Post())
            .guard(guard::Header("content-type", "application/cbor"))
            .to(submit_tx::<ERA, Tx>);
        HttpServiceFactory::register(resource, config);
    }
}

pub async fn build_api_server<const ERA: u16, Tx>(
    limits: Limits,
    tx_submission: TxSubmissionChannel<ERA, Tx>,
    bind_addr: SocketAddr,
) -> Result<impl Future<Output = io::Result<()>>, io::Error>
where
    Tx: Send + Deserialize + CanonicalHash + 'static,
    Tx::Hash: Display,
{
    let tx_submission = Data::new(tx_submission);
    Ok(HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header();

        App::new()
            .wrap(cors)
            .app_data(tx_submission.clone())
            .app_data(Data::new(limits))
            .service(SubmitTx::<ERA, Tx>(PhantomData))
    })
    .bind(bind_addr)?
    .workers(8)
    .run())
}
