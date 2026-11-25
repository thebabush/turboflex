//! c19: `#[derive(sqlx::FromRow)]` on a row whose columns are
//! `app::infra::db_types` types — `UserId` decoded via manual `Type`/`Decode`
//! impls in `app::infra::db_codec`, `Email` via `#[sqlx(try_from = "String")]`
//! (`TryFrom<String>` impl in `db_types`); plus runtime `sqlx::query_as`.

use crate::infra::db_types::{Email, UserId};

#[derive(sqlx::FromRow)]
pub struct UserRow {
    pub id: UserId,
    #[sqlx(try_from = "String")]
    pub email: Email,
}

pub async fn load(pool: &sqlx::SqlitePool) -> Result<Vec<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>("SELECT id, email FROM users")
        .fetch_all(pool)
        .await
}
