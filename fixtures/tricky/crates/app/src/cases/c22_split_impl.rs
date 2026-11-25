//! c22: a type (`app::split::Thing`) with inherent impl blocks in two modules;
//! `extra` is defined in `app::split_ext`.

pub fn run() -> u32 {
    crate::split::Thing::new().extra()
}
