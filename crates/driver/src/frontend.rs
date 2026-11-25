//! Front-end: drive `cargo check` over a workspace with this binary as the workspace wrapper,
//! then read back and merge every fact file the driver wrote, as the analysis side will.
#![expect(
    clippy::print_stderr,
    reason = "the front-end reports progress on stderr"
)]

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;
use std::{env, fmt};

use serde::Deserialize;
use turboflex_graph::{CrateName, Graph, MergeError, Origin, ReadError, Scope, TargetKind, Unit};

use crate::workspace::{ENV_OUT, ENV_WORKSPACE, Target, Workspace};

const TOOLCHAIN: &str = env!("TURBOFLEX_DRIVER_TOOLCHAIN");

const USAGE: &str = "usage: turboflex-driver <workspace-dir> [--out DIR] [--target-dir DIR] [cargo check args...]
  --out DIR         where fact files go (default <workspace>/out/turboflex)
  --target-dir DIR  cargo target dir (default <out>/target, keeps the workspace's own target/ untouched)
  other args are passed to `cargo check` (e.g. --all-targets --all-features)";

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("{USAGE}")]
    Usage,
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },
    #[error("cargo metadata failed: {0}")]
    Metadata(String),
    #[error("cargo metadata output: {0}")]
    MetadataJson(#[from] serde_json::Error),
    #[error("cargo metadata: target `{target}` has unknown kind {kinds:?}")]
    TargetKind { target: String, kinds: Vec<String> },
    #[error("cargo clean failed; extraction requires freshly compiled workspace crates")]
    Clean,
    #[error("cargo check failed")]
    Check,
    #[error("cargo check produced no fact files")]
    Empty,
    #[error("{}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: ReadError,
    },
    #[error("merging fact files: {0}")]
    Merge(#[from] MergeError),
}

fn io_error(context: impl fmt::Display) -> impl FnOnce(io::Error) -> Error {
    move |source| Error::Io {
        context: context.to_string(),
        source,
    }
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    match extract(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Error::Usage) => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("[turboflex] error: {e}");
            ExitCode::FAILURE
        }
    }
}

struct Args {
    workspace: PathBuf,
    out: Option<PathBuf>,
    target_dir: Option<PathBuf>,
    cargo: Vec<String>,
}

fn parse_args(args: &[String]) -> Result<Args, Error> {
    let Some((workspace, rest)) = args.split_first() else {
        return Err(Error::Usage);
    };
    if workspace.starts_with('-') {
        return Err(Error::Usage);
    }
    let mut parsed = Args {
        workspace: PathBuf::from(workspace),
        out: None,
        target_dir: None,
        cargo: Vec::new(),
    };
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => parsed.out = Some(it.next().ok_or(Error::Usage)?.into()),
            "--target-dir" => parsed.target_dir = Some(it.next().ok_or(Error::Usage)?.into()),
            _ => parsed.cargo.push(a.clone()),
        }
    }
    Ok(parsed)
}

fn extract(args: &[String]) -> Result<(), Error> {
    let Args {
        workspace,
        out,
        target_dir,
        cargo: mut cargo_args,
    } = parse_args(args)?;
    let workspace = fs::canonicalize(&workspace).map_err(io_error(workspace.display()))?;
    let out = absolute(out.unwrap_or_else(|| workspace.join("out/turboflex")))?;
    let target_dir = absolute(target_dir.unwrap_or_else(|| out.join("target")))?;
    let manifest = workspace.join("Cargo.toml");
    let (packages, members) = workspace_members(&manifest)?;
    let store = out.join(".turboflex");
    fs::create_dir_all(&store).map_err(io_error(store.display()))?;
    let parent = target_dir.parent().ok_or(Error::Usage)?;
    fs::create_dir_all(parent).map_err(io_error(parent.display()))?;
    let lock_path = target_dir.with_added_extension("turboflex-extraction.lock");
    let lock = File::options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(io_error(lock_path.display()))?;
    lock.lock().map_err(io_error(lock_path.display()))?;
    let stage = tempfile::Builder::new()
        .prefix("facts-")
        .tempdir_in(&store)
        .map_err(io_error(store.display()))?;
    eprintln!(
        "[turboflex] toolchain {TOOLCHAIN}; workspace crates: {}",
        members
            .crates
            .iter()
            .map(CrateName::as_str)
            .collect::<Vec<_>>()
            .join(",")
    );

    // Cargo only re-invokes the wrapper for dirty crates; force the workspace members to rebuild.
    let mut clean = cargo();
    clean
        .arg("clean")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target_dir);
    for p in &packages {
        clean.args(["-p", p]);
    }
    clean.arg("--quiet");
    if target_dir.exists()
        && !clean
            .status()
            .map_err(io_error("spawning cargo clean"))?
            .success()
    {
        return Err(Error::Clean);
    }

    // In a workspace whose root is also a package, plain `cargo check` only builds the root package
    // (other members compile as lib deps only, so their tests/examples/bins would be skipped).
    let selects_packages = cargo_args.iter().any(|a| {
        matches!(a.as_str(), "--workspace" | "--all" | "-p" | "--package")
            || a.starts_with("--package=")
    });
    if !selects_packages {
        cargo_args.push("--workspace".to_owned());
    }

    let exe = env::current_exe().map_err(io_error("locating turboflex-driver"))?;
    let started = Instant::now();
    let status = cargo()
        .arg("check")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target_dir)
        .args(&cargo_args)
        .env("RUSTC_WORKSPACE_WRAPPER", &exe)
        .env(ENV_OUT, stage.path())
        .env(ENV_WORKSPACE, serde_json::to_string(&members)?)
        .status()
        .map_err(io_error("spawning cargo check"))?;
    eprintln!(
        "[turboflex] cargo check finished in {:.1}s",
        started.elapsed().as_secs_f64()
    );
    if !status.success() {
        return Err(Error::Check);
    }
    validate(stage.path())?;
    let mut pointer = tempfile::NamedTempFile::new_in(&store).map_err(io_error(store.display()))?;
    let name = stage
        .path()
        .file_name()
        .ok_or(Error::Check)?
        .as_encoded_bytes();
    io::Write::write_all(&mut pointer, name).map_err(io_error(store.display()))?;
    let snapshot = stage.keep();
    if let Err(error) = pointer.persist(store.join("current")) {
        fs::remove_dir_all(&snapshot).map_err(io_error(snapshot.display()))?;
        return Err(io_error(store.display())(error.error));
    }
    Ok(())
}

fn absolute(p: PathBuf) -> Result<PathBuf, Error> {
    if p.is_absolute() {
        return Ok(p);
    }
    Ok(env::current_dir()
        .map_err(io_error("current directory"))?
        .join(p))
}

/// The `*.jsonl` files in `dir`, sorted.
fn fact_files(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).map_err(io_error(dir.display()))? {
        let path = entry.map_err(io_error(dir.display()))?.path();
        if path.extension().is_some_and(|e| e == "jsonl") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Reads every fact file back and merges them, exactly as the analysis side will; prints counts.
fn validate(out: &Path) -> Result<(), Error> {
    let files = fact_files(out)?;
    if files.is_empty() {
        return Err(Error::Empty);
    }
    let mut units = Vec::with_capacity(files.len());
    let mut bytes = 0;
    for path in &files {
        let file = File::open(path).map_err(io_error(path.display()))?;
        bytes += file.metadata().map_err(io_error(path.display()))?.len();
        let unit = Unit::read_jsonl(BufReader::new(file)).map_err(|source| Error::Read {
            path: path.clone(),
            source,
        })?;
        units.push(unit);
    }
    let graph = Graph::merge(units)?;
    let (mut workspace, mut external) = (0, 0);
    for entity in graph.entities() {
        match entity.origin {
            Origin::Workspace => workspace += 1,
            Origin::External => external += 1,
        }
    }
    let (mut normal, mut test_only, mut to_external) = (0, 0, 0);
    for edge in graph.edges() {
        match edge.scope {
            Scope::Normal => normal += 1,
            Scope::TestOnly => test_only += 1,
        }
        if graph
            .entity(&edge.edge.dst)
            .is_some_and(|e| e.origin == Origin::External)
        {
            to_external += 1;
        }
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "byte counts are far below f64's exact range"
    )]
    let mib = bytes as f64 / (1024.0 * 1024.0);
    eprintln!(
        "[turboflex] merged {} units ({mib:.1} MiB) from {}: entities {workspace} workspace + \
         {external} external; edges {normal} normal + {test_only} test-only \
         ({to_external} to external entities)",
        graph.headers().len(),
        out.display(),
    );
    Ok(())
}

/// A `cargo` command pinned to the driver's own toolchain, so that non-workspace dependencies are
/// compiled by a rustc whose metadata the driver can read.
fn cargo() -> Command {
    let mut cmd = Command::new("cargo");
    cmd.env("RUSTUP_TOOLCHAIN", TOOLCHAIN);
    for var in [
        "RUSTC",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "CARGO_TARGET_DIR",
        "CARGO_BUILD_TARGET_DIR",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    targets: Vec<MetadataTarget>,
}

#[derive(Deserialize)]
struct MetadataTarget {
    name: String,
    kind: Vec<String>,
    src_path: PathBuf,
}

/// Workspace package names, and the crates and targets of all their targets.
fn workspace_members(manifest: &Path) -> Result<(Vec<String>, Workspace), Error> {
    let output = cargo()
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()
        .map_err(io_error("spawning cargo metadata"))?;
    if !output.status.success() {
        return Err(Error::Metadata(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    let meta: Metadata = serde_json::from_slice(&output.stdout)?;
    let mut packages = Vec::new();
    let mut crates = BTreeSet::new();
    let mut targets = Vec::new();
    for pkg in meta.packages {
        packages.push(pkg.name);
        for target in pkg.targets {
            let kind = parse_target_kind(&target.kind).ok_or_else(|| Error::TargetKind {
                target: target.name.clone(),
                kinds: target.kind.clone(),
            })?;
            let crate_name = CrateName::new(target.name.replace('-', "_"));
            if kind != TargetKind::BuildScript {
                crates.insert(crate_name.clone());
            }
            let src_path =
                fs::canonicalize(&target.src_path).map_err(io_error(target.src_path.display()))?;
            targets.push(Target {
                crate_name,
                src_path,
                kind,
            });
        }
    }
    Ok((packages, Workspace { crates, targets }))
}

/// A target's kind from `cargo metadata` (a lib target lists its crate types, e.g. `["rlib",
/// "cdylib"]`).
fn parse_target_kind(kinds: &[String]) -> Option<TargetKind> {
    let kind = match kinds.first()?.as_str() {
        "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" => TargetKind::Lib,
        "bin" => TargetKind::Bin,
        "test" => TargetKind::Test,
        "example" => TargetKind::Example,
        "bench" => TargetKind::Bench,
        "proc-macro" => TargetKind::ProcMacro,
        "custom-build" => TargetKind::BuildScript,
        _ => return None,
    };
    Some(kind)
}
