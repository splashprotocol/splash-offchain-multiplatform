use crate::position_db::accounts;
use actix_web::dev::{AppService, HttpServiceFactory};
use actix_web::{guard, web, HttpResponse, Responder};
use cml_chain::certs::Credential;
use cml_core::serialization::{Deserialize as SDeserialize, FromBytes};
use cml_core::Slot;
use serde::{Deserialize, Serialize};
use spectrum_offchain_cardano::data::PoolId;
use std::marker::PhantomData;
use log::info;

#[derive(Clone, Serialize, Deserialize)]
pub struct AccountsResponse {
    pub lock: Slot,
    pub user_share: Vec<(u64, PoolId)>,
}

pub struct AccountsApi<Accounts>(pub PhantomData<Accounts>);

impl<Accounts: accounts::Accounts + 'static> HttpServiceFactory for AccountsApi<Accounts> {
    fn register(self, config: &mut AppService) {
        async fn lock_account<Accounts: accounts::Accounts>(
            account: web::Path<String>,
            accounts: web::Data<Accounts>,
        ) -> impl Responder {
            info!("[HTTP-API Accounts] Got request for lock: {}", account.clone());
            match hex::decode(account.clone())
                .ok()
                .and_then(|xs| Credential::from_cbor_bytes(xs.as_ref()).ok())
            {
                None => HttpResponse::BadRequest().finish(),
                Some(cred) => {
                    info!("[HTTP-API Accounts] Going to lock account: {}", account.clone());
                    if let Some(slot) = accounts.get_ref().lock(cred.clone()).await {
                        info!("[HTTP-API Accounts] Successfully locked {}", account.clone());
                        let user_shares = accounts.get_ref().get_user_shares(cred).await;
                        info!("[HTTP-API Accounts] Share for {} is {} elems", account.clone(), user_shares.len());
                        HttpResponse::Ok().json(AccountsResponse {
                            lock: slot,
                            user_share: user_shares,
                        })
                    } else {
                        HttpResponse::Ok().finish()
                    }
                }
            }
        }
        let resource = actix_web::Resource::new("/accounts/{account}/lock")
            .name("account-lock")
            .guard(guard::Post())
            .to(lock_account::<Accounts>);
        HttpServiceFactory::register(resource, config);
    }
}
