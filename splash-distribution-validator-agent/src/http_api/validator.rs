use actix_web::dev::{AppService, HttpServiceFactory};
use actix_web::{guard, web, HttpResponse, Responder};
use cml_chain::builders::tx_builder::SignedTxBuilder;
use cml_chain::certs::Credential;
use cml_chain::crypto::Vkeywitness;
use cml_chain::transaction::TransactionBody;
use cml_core::serialization::{FromBytes, Serialize as DS};
use serde::{Deserialize, Serialize};
use splash_distribution::validator::auth_requests_validator::AuthRequestsValidator;
use std::marker::PhantomData;

pub struct ValidatorApi();

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ValidatorRequest {
    #[serde(with = "serde_bytes")]
    raw_tx_body: Vec<u8>,
}

impl HttpServiceFactory for ValidatorApi {
    fn register(self, config: &mut AppService) {
        fn validate_tx(
            request: web::Json<ValidatorRequest>,
            validator: web::Data<AuthRequestsValidator>,
        ) -> impl Responder {
            let tx_body = TransactionBody::from_bytes(request.raw_tx_body.clone()).unwrap();

            match validator.validate(tx_body) {
                None => HttpResponse::BadRequest().finish(),
                Some(signature) => HttpResponse::Ok().json(hex::encode(signature.to_cbor_bytes())),
            }
        }

        let resource = actix_web::Resource::new("/validate/")
            .name("validate-tx")
            .guard(guard::Post())
            .to(validate_tx);
        HttpServiceFactory::register(resource, config);
    }
}
