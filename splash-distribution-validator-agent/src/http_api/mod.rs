mod validator;

use crate::http_api::validator::ValidatorApi;
use actix_web::web::Data;
use actix_web::{App, HttpServer};
use splash_distribution::validator::auth_requests_validator::AuthRequestsValidator;
use std::future::Future;
use std::io;
use std::net::SocketAddr;

pub async fn build_api_server(
    validator: AuthRequestsValidator,
    bind_addr: SocketAddr,
) -> Result<impl Future<Output = io::Result<()>>, io::Error> {
    let validator = Data::new(validator);
    Ok(HttpServer::new(move || {
        let cors = actix_cors::Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header();

        App::new()
            .wrap(cors)
            .app_data(validator.clone())
            .service(ValidatorApi())
    })
    .bind(bind_addr)?
    .workers(8)
    .run())
}
