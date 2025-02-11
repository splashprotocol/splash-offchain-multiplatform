mod accounts;

use crate::db;
use crate::http_api::accounts::AccountsApi;
use actix_web::web::Data;
use actix_web::{App, HttpServer};
use std::future::Future;
use std::io;
use std::marker::PhantomData;
use std::net::SocketAddr;

pub async fn build_api_server<Accounts: db::accounts::Accounts + Send + Sync + 'static>(
    accounts: Accounts,
    bind_addr: SocketAddr,
) -> Result<impl Future<Output = io::Result<()>>, io::Error> {
    let accounts = Data::new(accounts);
    Ok(HttpServer::new(move || {
        let cors = actix_cors::Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header();

        App::new()
            .wrap(cors)
            .app_data(accounts.clone())
            .service(AccountsApi::<Accounts>(PhantomData))
    })
    .bind(bind_addr)?
    .workers(8)
    .run())
}
