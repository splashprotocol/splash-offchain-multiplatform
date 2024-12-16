use crate::tx_submission::TxSubmissionChannel;
use actix_cors::Cors;
use actix_web::dev::{AppService, HttpServiceFactory};
use actix_web::web::Data;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use cml_chain::Deserialize;
use futures::StreamExt;
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

impl<const ERA: u16, Tx: Deserialize + 'static> HttpServiceFactory for SubmitTx<ERA, Tx> {
    fn register(self, config: &mut AppService) {
        async fn submit_tx<const ERA: u16, Tx: Deserialize>(
            limits: Data<Limits>,
            tx_submission: Data<TxSubmissionChannel<ERA, Tx>>,
            mut body: web::Payload,
        ) -> impl Responder {
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
                    let mut channel = tx_submission.get_ref().clone();
                    channel.submit(tx).await;
                    HttpResponse::Ok().body("success")
                }
                Err(_) => HttpResponse::BadRequest().body("invalid cbor"),
            }
        }
        let resource = actix_web::Resource::new("/submit")
            .name("submit_tx")
            .guard(actix_web::guard::Post())
            .to(submit_tx::<ERA, Tx>);
        HttpServiceFactory::register(resource, config);
    }
}

pub async fn build_api_server<const ERA: u16, Tx: Send + Deserialize + 'static>(
    limits: Limits,
    tx_submission: TxSubmissionChannel<ERA, Tx>,
    bind_addr: SocketAddr,
) -> Result<impl Future<Output = io::Result<()>>, io::Error> {
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
