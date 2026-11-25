//! l03 (legit, capability view): reading a file is not a write; `std::fs::read_to_string` is in
//! no sink, so anyone may call it.

pub fn load_config(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}
