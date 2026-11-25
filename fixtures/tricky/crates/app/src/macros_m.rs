//! Target for c05: "module M" — defines a crate-local `macro_rules!` whose
//! body calls a fn in "module N" (`crate::sink_n`).

macro_rules! record_via_n {
    ($x:expr) => {
        $crate::sink_n::record($x)
    };
}

pub(crate) use record_via_n;
