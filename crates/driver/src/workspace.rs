//! What the front-end tells the driver processes about the workspace, through the environment.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use turboflex_graph::{CrateName, TargetKind};

/// Directory the driver writes fact files to. Unset: the driver just compiles.
pub(crate) const ENV_OUT: &str = "TURBOFLEX_OUT";
/// JSON-encoded [`Workspace`].
pub(crate) const ENV_WORKSPACE: &str = "TURBOFLEX_WORKSPACE";

/// The analysed workspace, from `cargo metadata`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Workspace {
    /// Crate names of all member targets: items of these crates are [`Origin::Workspace`].
    ///
    /// [`Origin::Workspace`]: turboflex_graph::Origin::Workspace
    pub(crate) crates: BTreeSet<CrateName>,
    pub(crate) targets: Vec<Target>,
}

/// One Cargo target of a workspace member.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Target {
    pub(crate) crate_name: CrateName,
    /// Canonical path of the crate root file, as the driver sees it after canonicalizing rustc's
    /// input.
    pub(crate) src_path: PathBuf,
    pub(crate) kind: TargetKind,
}

impl Workspace {
    /// The target rustc is compiling: crate name and root file identify it (a package's lib and
    /// bin share the crate name).
    pub(crate) fn target(&self, crate_name: &CrateName, src_path: &Path) -> Option<&Target> {
        self.targets
            .iter()
            .find(|t| &t.crate_name == crate_name && t.src_path == src_path)
    }
}
