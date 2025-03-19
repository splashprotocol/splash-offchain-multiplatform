use crate::index::UtxoResolver;
use actix_cors::Cors;
use actix_web::dev::{AppService, HttpServiceFactory};
use actix_web::web::Data;
use actix_web::{guard, web, App, HttpResponse, HttpServer, Responder};
use cml_chain::address::Address;
use cml_chain::transaction::TransactionOutput;
use cml_crypto::{RawBytesEncoding, TransactionHash};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::OutputRef;
use std::future::Future;
use std::io;
use std::marker::PhantomData;
use std::net::SocketAddr;

#[derive(Clone, serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GetUTxOsRequest {
    address: Address,
    offset: usize,
    limit: usize,
}

#[derive(Clone, serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UTxO {
    pub transaction_hash: TransactionHash,
    pub index: usize,
    pub address: Address,
    pub value: Vec<Asset>,
}

impl From<(OutputRef, TransactionOutput)> for UTxO {
    fn from((oref, txo): (OutputRef, TransactionOutput)) -> Self {
        Self {
            transaction_hash: oref.tx_hash(),
            index: oref.index() as usize,
            address: txo.address().clone(),
            value: vec![Asset {
                policy_id: "".to_string(),
                base16_name: "".to_string(),
                amount: txo.value().coin.to_string(),
            }]
            .into_iter()
            .chain(txo.value().multiasset.iter().flat_map(|(pol, assets)| {
                assets.iter().map(|(name, amt)| Asset {
                    policy_id: pol.to_string(),
                    base16_name: name.to_raw_hex(),
                    amount: amt.to_string(),
                })
            }))
            .collect(),
        }
    }
}

#[derive(Clone, serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub policy_id: String,
    pub base16_name: String,
    pub amount: String,
}

pub struct Service<R>(PhantomData<R>);

impl<R> HttpServiceFactory for Service<R>
where
    R: UtxoResolver + 'static,
{
    fn register(self, config: &mut AppService) {
        async fn get_utxos<R>(req: web::Json<GetUTxOsRequest>, db: Data<R>) -> impl Responder
        where
            R: UtxoResolver + 'static,
        {
            let utxos = db.get_utxos(req.address.clone(), req.offset, req.limit).await;
            let result = utxos.into_iter().map(UTxO::from).collect::<Vec<_>>();
            HttpResponse::Ok().json(result)
        }
        let resource = actix_web::Resource::new("/getUtxos")
            .name("getUtxos")
            .guard(guard::Header("content-type", "application/json"))
            .to(get_utxos::<R>);
        HttpServiceFactory::register(resource, config);
    }
}

pub async fn build_api_server<R>(
    db: R,
    bind_addr: SocketAddr,
) -> Result<impl Future<Output = io::Result<()>>, io::Error>
where
    R: UtxoResolver + Send + Clone + 'static,
{
    Ok(HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header();

        App::new()
            .wrap(cors)
            .app_data(Data::new(db.clone()))
            .service(Service(PhantomData::<R>))
    })
    .bind(bind_addr)?
    .workers(8)
    .run())
}
