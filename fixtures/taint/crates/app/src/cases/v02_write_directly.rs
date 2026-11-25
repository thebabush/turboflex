//! v02 (capability violation): writes a file without going through `storage`.

pub fn dump(path: &str, body: &str) {
    let _ = std::fs::write(path, body);
}
