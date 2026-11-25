//! Filesystem-write sink: the only component that may write files (`std::fs` writes).

use std::fs::{self, File};
use std::io::{self, Write};

/// Writes `data` to `path`, replacing it.
pub fn save(path: &str, data: &str) -> io::Result<()> {
    fs::write(path, data)
}

/// Appends a line to `path`, creating it.
pub fn append(path: &str, line: &str) -> io::Result<()> {
    let mut file = fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")
}

/// Creates (or truncates) `path`.
pub fn touch(path: &str) -> io::Result<()> {
    File::create(path).map(drop)
}
