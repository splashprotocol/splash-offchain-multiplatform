use crate::account::{LockId, PoolAccountState};
use crate::position_db::accounts;
use crate::position_db::accounts::LockRejection;
use actix_web::dev::{AppService, HttpServiceFactory};
use actix_web::error::{HttpError, ParseError};
use actix_web::http::header::{Header, HeaderName, HeaderValue, TryIntoHeaderValue};
use actix_web::{guard, web, HttpMessage, HttpResponse, Responder};
use cml_chain::certs::Credential;
use cml_core::serialization::FromBytes;
use cml_core::Slot;
use serde::Serialize;
use spectrum_offchain_cardano::data::PoolId;
use std::marker::PhantomData;

#[derive(Clone, Serialize)]
pub struct LockAccountResponse {
    pub locked_at: Slot,
    pub lock_id: LockId,
    pub total_share: u64,
}

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
                Some(cred) => match accounts.get_ref().query_account(cred).await {
                    None => HttpResponse::NotFound().finish(),
                    Some(state) => HttpResponse::Ok().json(AccountStateResponse::new(state)),
                },
            }
        }

        async fn lock_account<Accounts: accounts::Accounts>(
            account: web::Path<String>,
            accounts: web::Data<Accounts>,
            lock_id: web::Header<XLockId>,
        ) -> impl Responder {
            match hex::decode(account.as_bytes())
                .ok()
                .and_then(|xs| Credential::from_bytes(xs).ok())
            {
                None => HttpResponse::BadRequest().finish(),
                Some(cred) => {
                    let XLockId(lock_id) = lock_id.into_inner();
                    match accounts.get_ref().lock(cred, lock_id).await {
                        Ok(_) => HttpResponse::Ok().finish(),
                        Err(LockRejection::ConcurrentLock(locked_by)) => {
                            HttpResponse::Conflict().json(LockedByAnotherRequest { locked_by })
                        }
                        Err(LockRejection::NotSynced) => HttpResponse::Conflict().json(NotSynced),
                    }
                }
            }
        }

        let query_resource = actix_web::Resource::new("/accounts/{account}")
            .name("account-query")
            .guard(guard::Get())
            .to(query_account::<Accounts>);

        let lock_resource = actix_web::Resource::new("/accounts/{account}/lock")
            .name("account-lock")
            .guard(guard::Post())
            .to(lock_account::<Accounts>);

        HttpServiceFactory::register(query_resource, config);
        HttpServiceFactory::register(lock_resource, config);
    }
}

#[derive(Clone, Serialize)]
pub struct AccountStateResponse {
    pub shares_by_pool: Vec<(PoolId, u64)>,
    pub activated_at: Option<Slot>,
}

impl AccountStateResponse {
    fn new(accounts: Vec<(PoolId, PoolAccountState)>) -> Self {
        let mut slf = Self {
            shares_by_pool: vec![],
            activated_at: None,
        };
        for (pool, acc) in accounts {
            slf.shares_by_pool.push((pool, acc.avg_share_bps));
            slf.activated_at = acc.activated_at;
        }
        slf
    }
}

#[derive(Debug, Serialize)]
pub struct LockedByAnotherRequest {
    locked_by: LockId,
}

#[derive(Debug, Serialize)]
pub struct NotSynced;

pub struct XLockId(LockId);

const HDR_NAME: &str = "X-Lock-Id";

impl TryIntoHeaderValue for XLockId {
    type Error = HttpError;

    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        let bytes: [u8; 32] = self.0.into();
        HeaderValue::from_str(&hex::encode(bytes)).map_err(|e| HttpError::from(e))
    }
}

impl Header for XLockId {
    fn name() -> HeaderName {
        HeaderName::from_lowercase(HDR_NAME.as_bytes()).unwrap()
    }
    fn parse<M: HttpMessage>(msg: &M) -> Result<Self, ParseError> {
        msg.headers()
            .get(HDR_NAME)
            .ok_or(ParseError::Header)?
            .to_str()
            .map_err(|_| ParseError::Header)
            .and_then(|s| hex::decode(s).map_err(|_| ParseError::Header))
            .and_then(|xs| {
                if xs.len() == 32 {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&xs);
                    Ok(XLockId(LockId::from(arr)))
                } else {
                    Err(ParseError::Header)
                }
            })
    }
}
