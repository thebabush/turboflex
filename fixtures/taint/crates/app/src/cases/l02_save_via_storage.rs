//! l02 (legit, capability view): write files through the storage sink only.

use crate::storage;

pub fn save_report(path: &str, body: &str) -> std::io::Result<()> {
    storage::touch(path)?;
    storage::save(path, body)?;
    storage::append(path, "-- end")
}
