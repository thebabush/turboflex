//! `turboflex-driver`: a rustc driver that extracts item-level dependency facts from a Cargo
//! workspace and writes them as `turboflex-graph` fact files, one per compile unit.
//!
//! The same binary has two modes:
//! - front-end (`turboflex-driver <workspace> [cargo check args]`): runs `cargo check` over the
//!   workspace with `RUSTC_WORKSPACE_WRAPPER` pointing back at this binary, then reads every fact
//!   file back and merges them as a validation step;
//! - driver (`turboflex-driver /path/to/rustc <rustc args>`, invoked by cargo for workspace
//!   members): runs the real compiler via `rustc_driver` and writes the unit's facts after
//!   analysis.
#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_infer;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;
extern crate rustc_trait_selection;

mod driver;
mod extract;
mod frontend;
mod names;
mod workspace;

use std::env;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let invoked_as_wrapper = args
        .get(1)
        .and_then(|a| Path::new(a).file_stem())
        .is_some_and(|stem| stem == "rustc");
    match args.get(2..) {
        Some(rustc_args) if invoked_as_wrapper => driver::run(rustc_args),
        _ => frontend::run(args.get(1..).unwrap_or_default()),
    }
}
