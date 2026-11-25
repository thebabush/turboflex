//! Target for c15: trait with a native `async fn` method.

#[allow(async_fn_in_trait)]
pub trait Service {
    async fn handle(&self) -> u32;
}
