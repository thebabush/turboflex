//! c05: `macro_rules!` defined in module M (`app::macros_m`) whose body calls
//! module N (`app::sink_n`), invoked here (module P); and a cross-crate
//! `#[macro_export]` macro from `macro_kit`.

pub fn local_macro() -> u32 {
    crate::macros_m::record_via_n!(5)
}

pub fn exported_macro() -> u32 {
    macro_kit::exported_double!(3)
}
