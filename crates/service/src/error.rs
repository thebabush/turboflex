//! Why a check could not run.

use std::io;
use std::path::PathBuf;

use turboflex_graph::{MergeError, ReadError};
use turboflex_model::ModelError;

/// A check that could not run: unreadable input, invalid facts or an invalid model. Every
/// variant names the file or directory it is about.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{}: cannot read the facts directory: {source}", path.display())]
    FactsDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{}: no fact files (*.jsonl)", path.display())]
    NoFacts { path: PathBuf },
    #[error("{}: cannot open: {source}", path.display())]
    FactsFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{}: {source}", path.display())]
    Facts {
        path: PathBuf,
        #[source]
        source: ReadError,
    },
    #[error("{}: cannot merge the fact files: {source}", path.display())]
    Merge {
        path: PathBuf,
        #[source]
        source: MergeError,
    },
    #[error("{}: cannot read the model: {source}", path.display())]
    ModelFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// Displays as `file:line:column: message`, one line per problem.
    #[error(transparent)]
    Model(#[from] ModelError),
}
