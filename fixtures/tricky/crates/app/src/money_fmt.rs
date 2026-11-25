//! Target for c10: `Display for Money`, separate from `Money`.

use crate::money::Money;
use std::fmt;

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${}.{:02}", self.cents / 100, self.cents % 100)
    }
}
