//! Target for c19: column types. `UserId`'s sqlx impls are in
//! `infra::db_codec`; `Email` is decoded via `#[sqlx(try_from = "String")]`.

pub struct UserId(pub i64);

pub struct Email(pub String);

impl TryFrom<String> for Email {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        if s.contains('@') { Ok(Email(s)) } else { Err(s) }
    }
}
