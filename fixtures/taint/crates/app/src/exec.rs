//! Process sink: the only component that may run a program (`std::process`). Takes a `SafeCmd`.

use std::process::Command;

use crate::safe_cmd::SafeCmd;

/// Runs a sanitized command line and returns whether it succeeded.
pub fn run(cmd: SafeCmd) -> bool {
    // Destructuring reads the field; it builds nothing.
    let SafeCmd(line) = cmd;
    let mut words = line.split_whitespace();
    let Some(program) = words.next() else {
        return false;
    };
    Command::new(program)
        .args(words)
        .status()
        .is_ok_and(|status| status.success())
}

/// Violation v03 (capability): the process sink also writes a log file, which only `storage`
/// may do.
pub fn run_logged(cmd: SafeCmd, log: &str) -> bool {
    let line = cmd.as_str().to_owned();
    let ok = run(cmd);
    let _ = std::fs::write(log, format!("{line}: {ok}\n"));
    ok
}
