//! Reading fact directories and model files.

use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Component, Path, PathBuf};

use turboflex_graph::{Graph, Unit};
use turboflex_model::{Model, Source};

use crate::error::ServiceError;

/// Reads every `*.jsonl` fact file directly in `dir` (by sorted path) and merges them. Strict:
/// the first unreadable or invalid file is an error naming it, and so is a directory without
/// fact files.
pub fn load_facts(dir: &Path) -> Result<Graph, ServiceError> {
    let dir_error = |source| ServiceError::FactsDir {
        path: dir.to_owned(),
        source,
    };
    let store = dir.join(".turboflex");
    let snapshot = match fs::read_to_string(store.join("current")) {
        Ok(name) => {
            if !name.starts_with("facts-")
                || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return Err(dir_error(std::io::Error::other("invalid snapshot name")));
            }
            store.join(name)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => dir.to_owned(),
        Err(e) => return Err(dir_error(e)),
    };
    let mut paths = Vec::new();
    for entry in fs::read_dir(snapshot).map_err(dir_error)? {
        let path = entry.map_err(dir_error)?.path();
        if path.extension().is_some_and(|ext| ext == "jsonl") {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        return Err(ServiceError::NoFacts {
            path: dir.to_owned(),
        });
    }
    paths.sort();
    let mut units = Vec::with_capacity(paths.len());
    for path in paths {
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(source) => return Err(ServiceError::FactsFile { path, source }),
        };
        match Unit::read_jsonl(BufReader::new(file)) {
            Ok(unit) => units.push(unit),
            Err(source) => return Err(ServiceError::Facts { path, source }),
        }
    }
    Graph::merge(units).map_err(|source| ServiceError::Merge {
        path: dir.to_owned(),
        source,
    })
}

/// Reads and parses one model file and every file it includes.
///
/// Each file is named by its path, derived from `path` as given and normalized lexically (`.`
/// and `name/..` folded): diagnostics and report locations show paths the way the user typed
/// them (`views/a.kdl`, and `views/common.kdl` for an `include "common.kdl"` in it), and the
/// same file reached through different spellings (`./common.kdl`, `x/../common.kdl`) gets one
/// name, so it is parsed once. An include path is resolved against the directory of the including
/// file; an absolute one is used as written.
///
/// Files are told apart by these names only, not by what is on disk: a file reached both
/// directly and through a symlink (or through a symlinked directory) has two names, so it is
/// loaded twice (its declarations then clash as duplicates) and a cycle through it is found one
/// step late. Likewise `link/..` is folded lexically even when `link` is a symlink.
pub fn load_model(path: &Path) -> Result<Model, ServiceError> {
    let text = fs::read_to_string(path).map_err(|source| ServiceError::ModelFile {
        path: path.to_owned(),
        source,
    })?;
    let name = normalize(path).display().to_string();
    Ok(Model::parse_with(&name, &text, load_include)?)
}

/// The [`Model::parse_with`] loader: reads `path` (as written in an `include`) relative to the
/// directory of the file named `including`.
fn load_include(including: &str, path: &str) -> Result<Source, String> {
    let dir = Path::new(including).parent().unwrap_or(Path::new(""));
    let resolved = normalize(&dir.join(path));
    let text = fs::read_to_string(&resolved)
        .map_err(|error| format!("{}: {error}", resolved.display()))?;
    Ok(Source {
        name: resolved.display().to_string(),
        text,
    })
}

/// Folds `.` and `name/..` out of `path` without touching the filesystem (no symlink
/// resolution). Leading `..` of a relative path are kept; `..` right after the root is dropped,
/// as the root is its own parent. A path that folds to nothing is `.`.
fn normalize(path: &Path) -> PathBuf {
    let mut parts: Vec<Component<'_>> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match parts.last() {
                Some(Component::Normal(_)) => {
                    parts.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                Some(Component::ParentDir | Component::CurDir) | None => parts.push(component),
            },
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                parts.push(component);
            }
        }
    }
    if parts.is_empty() {
        return PathBuf::from(".");
    }
    parts.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::normalize;

    fn normalized(path: &str) -> String {
        normalize(Path::new(path)).display().to_string()
    }

    #[test]
    fn folds_dots_lexically() {
        assert_eq!(normalized("turboflex.kdl"), "turboflex.kdl");
        assert_eq!(normalized("./common.kdl"), "common.kdl");
        assert_eq!(normalized("views/x/../common.kdl"), "views/common.kdl");
        assert_eq!(normalized("views/./a/./b.kdl"), "views/a/b.kdl");
        assert_eq!(normalized("/models/../common.kdl"), "/common.kdl");
    }

    #[test]
    fn keeps_what_it_cannot_fold() {
        assert_eq!(normalized("../common.kdl"), "../common.kdl");
        assert_eq!(normalized("a/../../b/../c.kdl"), "../c.kdl");
        assert_eq!(normalized("/../a.kdl"), "/a.kdl");
        assert_eq!(normalized("a/.."), ".");
    }
}
