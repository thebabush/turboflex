//! Command-line front end: checks extracted facts against architecture models.
//!
//! `turboflex check --facts DIR MODEL.kdl...` runs every model (view) over one fact base and
//! prints each view's findings. Exit status: 0 when no view has divergences or conflicts, 1 when
//! one has, 2 when the check could not run (usage, I/O, invalid facts or model).

use std::io::{self, BufWriter, ErrorKind, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use turboflex_service::{Check, Options};

#[derive(Debug, Parser)]
#[command(
    name = "turboflex",
    version,
    about = "Reflexion models for Rust workspaces"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Checks a fact base against one or more models, each a view of its own.
    Check(CheckArgs),
}

#[derive(Debug, Args)]
struct CheckArgs {
    /// Directory of fact files (`*.jsonl`, one per compile unit) written by turboflex-driver.
    #[arg(long, value_name = "DIR")]
    facts: PathBuf,
    /// Model files, checked and printed in this order.
    #[arg(required = true, value_name = "MODEL.kdl")]
    models: Vec<PathBuf>,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    /// Divergent edges shown per divergent component pair.
    #[arg(long, value_name = "N", default_value_t = Options::default().samples)]
    samples: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    /// For humans.
    Text,
    /// One JSON object, for machines.
    Json,
}

const FINDINGS: u8 = 1;
const FAILURE: u8 = 2;

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Check(args) => check(&args),
    }
}

fn check(args: &CheckArgs) -> ExitCode {
    let options = Options {
        samples: args.samples,
    };
    let check = match turboflex_service::check(&args.facts, &args.models, options) {
        Ok(check) => check,
        Err(error) => return fail(&error),
    };
    match write(&check, args.format) {
        // A closed pipe (`| head`) is the reader's choice, not a failure.
        Err(error) if error.kind() != ErrorKind::BrokenPipe => return fail(&error),
        Ok(()) | Err(_) => {}
    }
    if check.has_findings() {
        ExitCode::from(FINDINGS)
    } else {
        ExitCode::SUCCESS
    }
}

fn write(check: &Check, format: Format) -> io::Result<()> {
    let mut out = BufWriter::new(io::stdout().lock());
    match format {
        Format::Text => check.write_text(&mut out)?,
        Format::Json => check.write_json(&mut out)?,
    }
    out.flush()
}

fn fail(error: &dyn std::error::Error) -> ExitCode {
    // Nothing better to do if stderr is gone too.
    let _ = writeln!(io::stderr().lock(), "error: {error}");
    ExitCode::from(FAILURE)
}
