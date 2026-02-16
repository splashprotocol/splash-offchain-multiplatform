use crate::position_db::accounts;
use actix_web::dev::{AppService, HttpServiceFactory};
use actix_web::{guard, web, HttpResponse, Responder};
use cml_chain::certs::Credential;
use serde::Deserialize;
use splash_yf_offchain::Epoch;
use std::marker::PhantomData;

pub struct AccountsApi<Accounts>(pub PhantomData<Accounts>);

impl<Accounts: accounts::Accounts + 'static> HttpServiceFactory for AccountsApi<Accounts> {
    fn register(self, config: &mut AppService) {
        async fn query_account<Accounts: accounts::Accounts>(
            req: web::Json<QueryAccountRequest>,
            accounts: web::Data<Accounts>,
        ) -> impl Responder {
            let QueryAccountRequest {
                account,
                from_epoch_inclusive,
            } = req.into_inner();
            match accounts
                .get_ref()
                .query_account(account, from_epoch_inclusive)
                .await
            {
                None => HttpResponse::NotFound().finish(),
                Some(state) => HttpResponse::Ok().json(state),
            }
        }

        let query_resource = actix_web::Resource::new("/accounts/query-reward")
            .name("accounts-query-reward")
            .guard(guard::Post())
            .guard(guard::Header("content-type", "application/json"))
            .to(query_account::<Accounts>);

        HttpServiceFactory::register(query_resource, config);
    }
}

#[derive(Debug, Deserialize)]
struct QueryAccountRequest {
    account: Credential,
    from_epoch_inclusive: Epoch,
}
