//! c10: `format!`/`println!` of a type whose `Display` impl lives in
//! `app::money_fmt` (type in `app::money`).

use crate::money::Money;

pub fn render(m: &Money) -> String {
    format!("{m}")
}

pub fn print(m: &Money) {
    println!("{}", m);
}
