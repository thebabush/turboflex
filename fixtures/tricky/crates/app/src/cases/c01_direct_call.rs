//! c01: plain free-fn calls — cross-module (`app::util`) and cross-crate
//! (`core_api`).

pub fn cross_module() -> u32 {
    crate::util::helper()
}

pub fn cross_crate() -> u32 {
    core_api::core_helper()
}
