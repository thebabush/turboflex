//! Source locations and the problems a model can have.

use std::fmt;
use std::sync::Arc;

use miette::{Diagnostic, LabeledSpan, NamedSource, SourceCode, SourceSpan};

use crate::model::ComponentId;

/// Which source file of a model a [`Location`] is in: the root file is the first, included
/// files follow in the order they are first included. Ask the [`Model`](crate::Model) for its
/// name with [`Model::source_name_of`](crate::Model::source_name_of).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub(crate) usize);

impl SourceId {
    /// The root file, the one parsing started from.
    pub const ROOT: Self = Self(0);

    #[must_use]
    pub fn index(self) -> usize {
        self.0
    }
}

/// Where something is written in the model sources: the file, a byte range in it, and its
/// 1-based line and column (in characters) for messages. Orders by file, then position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Location {
    pub source: SourceId,
    pub offset: usize,
    pub len: usize,
    pub line: usize,
    pub column: usize,
}

impl From<Location> for SourceSpan {
    fn from(location: Location) -> Self {
        (location.offset, location.len).into()
    }
}

/// `line:column`; the file is not included (see [`ModelError`]'s `Display` for that).
impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// One source file read while parsing a model: its name, text, and the start of every line, to
/// map byte offsets to [`Location`]s.
pub(crate) struct SourceFile {
    id: SourceId,
    pub(crate) name: String,
    pub(crate) text: Arc<String>,
    /// Byte offset of the start of every line.
    starts: Vec<usize>,
}

impl SourceFile {
    pub(crate) fn new(id: SourceId, name: String, text: String) -> Self {
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self {
            id,
            name,
            text: Arc::new(text),
            starts,
        }
    }

    pub(crate) fn locate(&self, span: SourceSpan) -> Location {
        let offset = span.offset().min(self.text.len());
        let line = self.starts.partition_point(|&start| start <= offset);
        let start = self.starts[line - 1];
        let column = self
            .text
            .get(start..offset)
            .map_or(1, |prefix| prefix.chars().count() + 1);
        Location {
            source: self.id,
            offset,
            len: span.len(),
            line,
            column,
        }
    }

    fn named(&self) -> NamedSource<Arc<String>> {
        NamedSource::new(&self.name, Arc::clone(&self.text))
    }
}

/// What is wrong with a model. Every variant is reported with the [`Location`] it is about.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProblemKind {
    /// The text is not valid KDL v2.
    #[error("KDL syntax: {message}")]
    Syntax { message: String },
    #[error("unknown node `{name}` here; expected one of: {expected}")]
    UnknownNode {
        name: String,
        expected: &'static str,
    },
    #[error("unknown property `{name}` on `{node}`; expected one of: {expected}")]
    UnknownProperty {
        node: &'static str,
        name: String,
        expected: String,
    },
    #[error("property `{name}` is given twice")]
    DuplicateProperty { name: String },
    #[error("`{node}` takes no properties")]
    UnexpectedProperty { node: &'static str },
    #[error("type annotations are not part of the model language")]
    TypeAnnotation,
    #[error("`{node}` expects {expected}")]
    BadArguments {
        node: &'static str,
        expected: &'static str,
    },
    #[error("expected a string, found `{found}`")]
    NotAString { found: String },
    #[error("`{name}` expects {expected}")]
    BadPropertyValue {
        name: &'static str,
        expected: &'static str,
    },
    #[error("`{node}` takes no children block")]
    UnexpectedChildren { node: &'static str },
    #[error("`{node}` may appear only once")]
    DuplicateNode { node: &'static str },
    #[error(
        "invalid component name `{name}`: use ASCII letters, digits, `_` and `-`, starting with a letter (nest components instead of writing dots)"
    )]
    InvalidComponentName { name: String },
    #[error("component `{id}` is declared twice")]
    DuplicateComponent { id: ComponentId },
    #[error("malformed {what} pattern `{pattern}`: {reason}")]
    MalformedPattern {
        what: &'static str,
        pattern: String,
        reason: &'static str,
    },
    #[error(
        "pattern `{pattern}` is declared twice (in `{first}` and `{second}`); every pattern needs a single owner"
    )]
    DuplicatePattern {
        pattern: String,
        first: ComponentId,
        second: ComponentId,
    },
    #[error("`{id}` is not a declared component")]
    UnknownComponent { id: String },
    #[error(
        "`{id}` is an external component; only workspace components can be the source of a {node}"
    )]
    ExternalSource { id: ComponentId, node: &'static str },
    #[error("the rule `{from} -> {to}` is declared twice")]
    DuplicateRule { from: ComponentId, to: ComponentId },
    #[error("`{value}` is listed twice in `{node}`")]
    DuplicateValue { node: &'static str, value: String },
    #[error(
        "`kinds` lists only edge groups the policy's `edges` leaves out, so this rule can never match"
    )]
    KindsOutsidePolicy,
    #[error("`include` needs a loader: parse the model with `Model::parse_with`")]
    IncludeWithoutLoader,
    #[error("cannot include `{path}`: {message}")]
    IncludeFailed { path: String, message: String },
    #[error("include cycle: {}", .chain.join(" -> "))]
    IncludeCycle {
        /// The files of the cycle, from the first to the one including it again.
        chain: Vec<String>,
    },
    #[error("`{node}` belongs to the root model, not to an included file")]
    RootOnly { node: &'static str },
    /// `foundation` let every component depend on the listed ones, including components added
    /// later, so it granted permissions nobody wrote down. Reported wherever it appears.
    #[error(
        "`foundation` has been removed: write an `allow` rule for each dependency it covered (`allow \"FROM\" -> \"TO\"...`), or, for a view about a few components, use `policy {{ targets ... }}`"
    )]
    FoundationRemoved,
}

/// One problem in a model, located in its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub kind: ProblemKind,
    pub location: Location,
    /// The earlier declaration, for problems about something declared twice.
    pub previous: Option<Location>,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.location, self.kind)?;
        if let Some(previous) = self.previous {
            write!(f, " (first declared at {previous})")?;
        }
        Ok(())
    }
}

impl std::error::Error for Problem {}

impl Diagnostic for Problem {
    fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan> + '_>> {
        let here = LabeledSpan::new_primary_with_span(Some("here".to_owned()), self.location);
        let previous = self
            .previous
            .map(|previous| LabeledSpan::new_with_span(Some("first here".to_owned()), previous));
        Some(Box::new(std::iter::once(here).chain(previous)))
    }
}

/// Why a model was rejected: every problem found, in source order (the root file first, then
/// included files in the order they were first included).
///
/// `Display` lists them as `name:line:column: message`, each with the name of its own file; as a
/// [`Diagnostic`] it carries the sources so a `miette` report handler can render each problem in
/// the context of its file.
#[derive(Debug, Clone)]
pub struct ModelError {
    /// The root file.
    source: NamedSource<Arc<String>>,
    problems: Vec<Problem>,
    /// `problems`, in the same order, each with its file.
    located: Vec<LocatedProblem>,
}

impl ModelError {
    /// `sources` are indexed by [`SourceId`]; every problem's locations must be in them.
    pub(crate) fn new(sources: &[SourceFile], mut problems: Vec<Problem>) -> Self {
        problems.sort_by_key(|problem| problem.location);
        let located = problems
            .iter()
            .map(|problem| LocatedProblem {
                problem: problem.clone(),
                source: sources[problem.location.source.0].named(),
                previous_file: problem
                    .previous
                    .filter(|previous| previous.source != problem.location.source)
                    .map(|previous| sources[previous.source.0].name.clone()),
            })
            .collect();
        Self {
            source: sources[SourceId::ROOT.0].named(),
            problems,
            located,
        }
    }

    /// Every problem found, in source order. Never empty.
    #[must_use]
    pub fn problems(&self) -> &[Problem] {
        &self.problems
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: invalid architecture model", self.source.name())?;
        for problem in &self.located {
            write!(f, "\n{}:{problem}", problem.source.name())?;
        }
        Ok(())
    }
}

impl std::error::Error for ModelError {}

impl Diagnostic for ModelError {
    fn source_code(&self) -> Option<&dyn SourceCode> {
        Some(&self.source)
    }

    fn related<'a>(&'a self) -> Option<Box<dyn Iterator<Item = &'a dyn Diagnostic> + 'a>> {
        Some(Box::new(
            self.located
                .iter()
                .map(|problem| problem as &dyn Diagnostic),
        ))
    }
}

/// A [`Problem`] with the file it is in, so it renders against that file's text.
#[derive(Debug, Clone)]
struct LocatedProblem {
    problem: Problem,
    source: NamedSource<Arc<String>>,
    /// The file of `problem.previous`, when it is not `source`.
    previous_file: Option<String>,
}

impl fmt::Display for LocatedProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Problem {
            kind,
            location,
            previous,
        } = &self.problem;
        write!(f, "{location}: {kind}")?;
        match (previous, &self.previous_file) {
            (Some(previous), Some(file)) => write!(f, " (first declared at {file}:{previous})"),
            (Some(previous), None) => write!(f, " (first declared at {previous})"),
            (None, _) => Ok(()),
        }
    }
}

impl std::error::Error for LocatedProblem {}

impl Diagnostic for LocatedProblem {
    fn source_code(&self) -> Option<&dyn SourceCode> {
        Some(&self.source)
    }

    /// The earlier declaration is labelled only when it is in the same file; otherwise the
    /// message names its file.
    fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan> + '_>> {
        let here =
            LabeledSpan::new_primary_with_span(Some("here".to_owned()), self.problem.location);
        let previous = self
            .problem
            .previous
            .filter(|_| self.previous_file.is_none())
            .map(|previous| LabeledSpan::new_with_span(Some("first here".to_owned()), previous));
        Some(Box::new(std::iter::once(here).chain(previous)))
    }
}
