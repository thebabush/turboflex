//! c16: conditionally compiled dependencies — a `#[cfg(test)]` module calling
//! `app::checks`, and a `#[cfg(feature = "extra")]` fn calling
//! `app::extras` (feature off by default).

#[cfg(feature = "extra")]
pub fn extra_entry() -> u32 {
    crate::extras::bonus()
}

#[cfg(test)]
mod tests {
    #[test]
    fn validates() {
        assert!(crate::checks::validate(1));
    }
}
