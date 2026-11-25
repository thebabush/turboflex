//! c15: `async fn` awaiting another module's async fn
//! (`app::infra::net::fetch`); an `async` block doing the same; and a native
//! `async fn` trait method (`app::service::Service`, impl in
//! `app::service_impl`) called via a bound and via a concrete receiver.

use crate::service::Service;
use crate::service_impl::Echo;
use std::future::Future;

pub async fn await_fn() -> u32 {
    crate::infra::net::fetch().await + 1
}

pub fn async_block() -> impl Future<Output = u32> {
    async { crate::infra::net::fetch().await }
}

pub async fn via_bound<S: Service>(s: &S) -> u32 {
    s.handle().await
}

pub async fn via_concrete() -> u32 {
    Echo.handle().await
}
