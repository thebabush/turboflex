//! c20: method resolved through a blanket impl (`impl<T: Greeter> Shout for T`
//! in `app::blanket`) on a type (`app::animals::Dog`) that never mentions
//! `Shout`.

use crate::blanket::Shout;

pub fn run() -> String {
    crate::animals::Dog.shout()
}
