//! Target for c19: manual `sqlx::Type` / `sqlx::Decode` impls for
//! `infra::db_types::UserId`.

use crate::infra::db_types::UserId;
use sqlx::sqlite::{Sqlite, SqliteTypeInfo, SqliteValueRef};
use sqlx::{Decode, Type};

impl Type<Sqlite> for UserId {
    fn type_info() -> SqliteTypeInfo {
        <i64 as Type<Sqlite>>::type_info()
    }
}

impl<'r> Decode<'r, Sqlite> for UserId {
    fn decode(value: SqliteValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        Ok(UserId(<i64 as Decode<'r, Sqlite>>::decode(value)?))
    }
}
