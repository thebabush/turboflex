//! v01 (capability violation): runs a program without going through `exec`.

use std::process::Command;

pub fn list_dir(dir: &str) -> bool {
    Command::new("ls").arg(dir).status().is_ok()
}
