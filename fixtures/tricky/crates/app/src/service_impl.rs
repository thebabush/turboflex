//! Target for c15: impl of `service::Service` whose async body awaits
//! `infra::net::fetch`.

pub struct Echo;

impl crate::service::Service for Echo {
    async fn handle(&self) -> u32 {
        crate::infra::net::fetch().await
    }
}
