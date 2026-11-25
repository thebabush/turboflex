//! Driver mode: cargo calls us as `RUSTC_WORKSPACE_WRAPPER`, i.e. `turboflex-driver <rustc>
//! <args>`, for workspace members only. We compile with the linked-in rustc, and for the targets
//! we extract from we write one fact file once analysis is done.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::{env, fmt};

use rustc_driver::{Callbacks, Compilation};
use rustc_interface::interface::{Compiler, Config};
use rustc_middle::ty::TyCtxt;
use rustc_session::config::Input;
use turboflex_graph::{CompileMode, CrateName, TargetKind, Unit, UnitHeader};

use crate::extract;
use crate::workspace::{ENV_OUT, ENV_WORKSPACE, Workspace};

/// Where one compile unit's facts go, and its header.
struct Plan {
    out_file: PathBuf,
    header: UnitHeader,
    /// Crate names of the analysed workspace.
    workspace: BTreeSet<CrateName>,
}

#[derive(Debug, thiserror::Error)]
enum PlanError {
    #[error("{ENV_WORKSPACE} is missing or invalid (run through the turboflex-driver front-end)")]
    Workspace(#[source] Option<serde_json::Error>),
    #[error("cannot resolve the crate root `{path}`")]
    Input {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("`{crate_name}` from `{path}` is not a target of the workspace's cargo metadata")]
    UnknownTarget {
        crate_name: CrateName,
        path: PathBuf,
    },
}

/// File-name label of a target kind and mode: `lib`, `lib-test`, `test-test`, ...
struct Label(TargetKind, CompileMode);

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.0 {
            TargetKind::Lib => "lib",
            TargetKind::Bin => "bin",
            TargetKind::Test => "test",
            TargetKind::Example => "example",
            TargetKind::Bench => "bench",
            TargetKind::ProcMacro => "proc_macro",
            TargetKind::BuildScript => "build_script",
        };
        match self.1 {
            CompileMode::Normal => f.write_str(kind),
            CompileMode::Test => write!(f, "{kind}-test"),
        }
    }
}

/// Decides whether this rustc invocation is a unit we extract from (`None`: just compile).
fn plan(config: &Config) -> Option<Result<Plan, PlanError>> {
    let out = env::var_os(ENV_OUT)?;
    if !config.opts.prints.is_empty() {
        return None;
    }
    let crate_name = CrateName::new(config.opts.crate_name.clone()?);
    let Input::File(input) = &config.input else {
        return None;
    };
    plan_target(&PathBuf::from(out), crate_name, input, config).transpose()
}

fn plan_target(
    out: &Path,
    crate_name: CrateName,
    input: &Path,
    config: &Config,
) -> Result<Option<Plan>, PlanError> {
    let workspace = env::var(ENV_WORKSPACE).map_err(|_| PlanError::Workspace(None))?;
    let workspace: Workspace =
        serde_json::from_str(&workspace).map_err(|e| PlanError::Workspace(Some(e)))?;
    let path = fs::canonicalize(input).map_err(|source| PlanError::Input {
        path: input.to_owned(),
        source,
    })?;
    let Some(target) = workspace.target(&crate_name, &path) else {
        return Err(PlanError::UnknownTarget { crate_name, path });
    };
    let kind = target.kind;
    if kind == TargetKind::BuildScript {
        // Every build script is a crate named `build_script_build`: their items would share
        // paths across packages. They are build plumbing, not architecture; skip them.
        return Ok(None);
    }
    let mode = if config.opts.test {
        CompileMode::Test
    } else {
        CompileMode::Normal
    };
    let features = config
        .crate_cfg
        .iter()
        .filter_map(|cfg| {
            let value = cfg.strip_prefix("feature=")?;
            Some(value.trim_matches('"').to_owned())
        })
        .collect();
    // A lib and a bin of one package share a crate name, and so would their `--test` builds;
    // cargo's per-unit `extra-filename` hash keeps the files apart.
    let extra = &config.opts.cg.extra_filename;
    let out_file = out.join(format!("{crate_name}{extra}.{}.jsonl", Label(kind, mode)));
    Ok(Some(Plan {
        out_file,
        header: UnitHeader {
            crate_name,
            target: kind,
            mode,
            features,
        },
        workspace: workspace.crates,
    }))
}

struct FactCallbacks {
    plan: Option<Result<Plan, PlanError>>,
}

impl Callbacks for FactCallbacks {
    fn config(&mut self, config: &mut Config) {
        self.plan = plan(config);
        if let Some(Ok(_)) = self.plan {
            // Keep `const` item references as `Const::Unevaluated` in runtime MIR: at
            // mir-opt-level 0 no pass (GVN, const-prop, inlining) evaluates or inlines them, and
            // drop/call structure stays close to the source.
            config.opts.unstable_opts.mir_opt_level = Some(0);
        }
    }

    #[expect(clippy::print_stderr, reason = "progress line; cargo relays it")]
    fn after_analysis(&mut self, _compiler: &Compiler, tcx: TyCtxt<'_>) -> Compilation {
        let plan = match self.plan.take() {
            None => return Compilation::Continue,
            Some(Err(e)) => tcx.dcx().fatal(format!("turboflex: {e}")),
            Some(Ok(plan)) => plan,
        };
        let facts = extract::extract(tcx, &plan.workspace);
        let unit = Unit {
            header: plan.header,
            entities: facts.entities,
            edges: facts.edges,
        };
        let written =
            File::create(&plan.out_file).and_then(|file| unit.write_jsonl(BufWriter::new(file)));
        if let Err(e) = written {
            tcx.dcx().fatal(format!(
                "turboflex: cannot write {}: {e}",
                plan.out_file.display()
            ));
        }
        eprintln!(
            "[turboflex] {} entities, {} edges{} -> {}",
            unit.entities.len(),
            unit.edges.len(),
            match facts.collisions.len() {
                0 => String::new(),
                n => format!(", {n} path collisions"),
            },
            plan.out_file.display()
        );
        Compilation::Continue
    }
}

pub(crate) fn run(rustc_args: &[String]) -> ExitCode {
    // `run_compiler` expects argv[0] to be the program name.
    let mut args = vec!["rustc".to_owned()];
    args.extend_from_slice(rustc_args);
    rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&args, &mut FactCallbacks { plan: None });
    })
}
