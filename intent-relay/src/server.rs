use crate::intent::AuthedIntent;
use crate::queue::BroadcastQueue;
use actix_cors::Cors;
use actix_web::dev::{AppService, HttpServiceFactory};
use actix_web::web::Data;
use actix_web::{guard, web, App, HttpResponse, HttpServer, Responder};
use derive_more::Into;
use serde::Deserialize;
use std::future::Future;
use std::io;
use std::marker::PhantomData;
use std::net::SocketAddr;

#[derive(PartialEq, Clone, Into, Debug, Deserialize)]
#[serde(try_from = "String")]
pub struct HexString(Vec<u8>);

impl TryFrom<String> for HexString {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match hex::decode(value) {
            Ok(bytes) => Ok(HexString(bytes)),
            Err(err) => Err(format!("Failed to decode hex string: {}", err)),
        }
    }
}

#[derive(PartialEq, Clone, Into, Debug, Deserialize)]
#[serde(try_from = "String")]
pub struct ConstHexString<const N: usize>([u8; N]);

impl<const N: usize> TryFrom<String> for ConstHexString<N> {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match hex::decode(value) {
            Ok(bytes) => {
                let len = bytes.len();
                match <[u8; N]>::try_from(bytes) {
                    Ok(bytes) => Ok(ConstHexString(bytes)),
                    Err(_) => Err(format!("Failed to convert {} bytes to {}-byte array", len, N)),
                }
            }
            Err(err) => Err(format!("Failed to decode hex string: {}", err)),
        }
    }
}

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitIntentRequest {
    /// Account identifier (NFT Token - policy_id || asset_name, 32 bytes hex)
    account_id: ConstHexString<32>,
    intent: HexString,
    prefix: HexString,
    postfix: HexString,
    signature: HexString,
    credential: ConstHexString<32>,
}

impl From<SubmitIntentRequest> for AuthedIntent {
    fn from(value: SubmitIntentRequest) -> Self {
        Self {
            account_id: value.account_id.into(),
            intent: value.intent.into(),
            prefix: value.prefix.into(),
            postfix: value.postfix.into(),
            signature: value.signature.into(),
            credential: value.credential.into(),
        }
    }
}

pub struct Service<R>(PhantomData<R>);

impl<R> HttpServiceFactory for Service<R>
where
    R: BroadcastQueue<AuthedIntent> + 'static,
{
    fn register(self, config: &mut AppService) {
        async fn submit_intent<R>(req: web::Json<SubmitIntentRequest>, queue: Data<R>) -> impl Responder
        where
            R: BroadcastQueue<AuthedIntent> + 'static,
        {
            queue.enqueue(req.into_inner().into()).await;
            HttpResponse::Ok()
        }
        let resource = actix_web::Resource::new("/intent/submit")
            .name("submitIntent")
            .guard(guard::Post())
            .guard(guard::Header("content-type", "application/json"))
            .to(submit_intent::<R>);
        HttpServiceFactory::register(resource, config);
    }
}

pub async fn build_api_server<R>(
    db: R,
    bind_addr: SocketAddr,
) -> Result<impl Future<Output = io::Result<()>>, io::Error>
where
    R: BroadcastQueue<AuthedIntent> + Send + Clone + 'static,
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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("00ff", Ok(HexString(vec![0x00, 0xff])))]
    #[case("deadbeef", Ok(HexString(vec![0xde, 0xad, 0xbe, 0xef])))]
    #[case("nothex", Err("Failed to decode hex string: Invalid character 'n' at position 0".to_string()
    ))]
    fn test_hex_string_conversion(#[case] input: &str, #[case] expected: Result<HexString, String>) {
        let result: Result<HexString, String> = input.to_string().try_into();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case("00000000", Ok(ConstHexString([0x00, 0x00, 0x00, 0x00])))]
    #[case("abcd", Err("Failed to convert 2 bytes to 4-byte array".to_string()))]
    #[case("nothex", Err("Failed to decode hex string: Invalid character 'n' at position 0".to_string()
    ))]
    fn test_const_hex_string_conversion(
        #[case] input: &str,
        #[case] expected: Result<ConstHexString<4>, String>,
    ) {
        let result: Result<ConstHexString<4>, String> = input.to_string().try_into();
        assert_eq!(result, expected);
    }
}
