use crate::db::accounts;
use actix_web::dev::{AppService, HttpServiceFactory};
use actix_web::{guard, web, HttpResponse, Responder};
use cml_chain::certs::Credential;
use cml_core::serialization::FromBytes;
use std::marker::PhantomData;

pub struct AccountsApi<Accounts>(pub PhantomData<Accounts>);

impl<Accounts: accounts::Accounts + 'static> HttpServiceFactory for AccountsApi<Accounts> {
    fn register(self, config: &mut AppService) {
        async fn lock_account<Accounts: accounts::Accounts>(
            account: web::Path<String>,
            accounts: web::Data<Accounts>,
        ) -> impl Responder {
            match hex::decode(account.as_bytes())
                .ok()
                .and_then(|xs| Credential::from_bytes(xs).ok())
            {
                None => HttpResponse::BadRequest().finish(),
                Some(cred) => {
                    if let Some(slot) = accounts.get_ref().lock(cred).await {
                        HttpResponse::Ok().json(slot)
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
