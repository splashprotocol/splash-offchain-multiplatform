use cml_chain::certs::Credential;

#[derive(Debug, Clone, PartialEq)]
pub struct HarvestOrder<OrderId> {
    pub id: OrderId,
    pub account: Credential,
}
