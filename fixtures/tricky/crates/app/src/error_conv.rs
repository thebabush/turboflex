//! Target for c07: the `From<ErrA> for ErrB` impl lives in a third module.

use crate::errors_a::ErrA;
use crate::errors_b::ErrB;

impl From<ErrA> for ErrB {
    fn from(_: ErrA) -> Self {
        ErrB { code: 42 }
    }
}
