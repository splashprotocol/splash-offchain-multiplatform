use crate::account::AccountPosition;
use crate::position_db::accounts;
use actix_web::dev::{AppService, HttpServiceFactory};
use actix_web::http::header::{Header, TryIntoHeaderValue};
use actix_web::{guard, web, HttpResponse, Responder};
use cml_chain::certs::Credential;
use cml_core::serialization::FromBytes;
use cml_core::Slot;
use serde::Serialize;
use spectrum_offchain_cardano::data::PoolId;
use std::marker::PhantomData;
use splash_yf_offchain::Epoch;
use crate::position_db::accounts::AccountReward;

pub struct AccountsApi<Accounts>(pub PhantomData<Accounts>);

impl<Accounts: accounts::Accounts + 'static> HttpServiceFactory for AccountsApi<Accounts> {
    fn register(self, config: &mut AppService) {
        async fn query_account<Accounts: accounts::Accounts>(
            account: web::Path<String>,
            accounts: web::Data<Accounts>,
        ) -> impl Responder {
            match hex::decode(account.as_bytes())
                .ok()
                .and_then(|xs| Credential::from_bytes(xs).ok())
            {
                None => HttpResponse::BadRequest().finish(),
                Some(cred) => match accounts.get_ref().query_account(cred, Epoch::from(0)).await {
                    None => HttpResponse::NotFound().finish(),
                    Some(state) => HttpResponse::Ok().json(AccountStateResponse::new(state)),
                },
            }
        }

        let query_resource = actix_web::Resource::new("/accounts/{account}")
            .name("account-query")
            .guard(guard::Get())
            .to(query_account::<Accounts>);

        HttpServiceFactory::register(query_resource, config);
    }
}

#[derive(Clone, Serialize)]
pub struct AccountStateResponse {
    pub shares_by_pool: Vec<(PoolId, u64)>,
    pub activated_at: Option<Slot>,
}

impl AccountStateResponse {
    fn new(rew: AccountReward) -> Self {
        todo!()
    }
}

#[derive(Debug, Serialize)]
pub struct NotSynced;
