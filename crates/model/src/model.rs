//! The validated architecture model: components, patterns, rules, links and policy.

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use turboflex_graph::{CrateName, EdgeGroup, ItemPath, ModulePath, Provenance};

use crate::error::{Location, ModelError, SourceId};

/// The full, dotted id of a component: `worker`, `worker.runtime`. Each dot is one level of
/// nesting; external components are never nested.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ComponentId(String);

impl ComponentId {
    /// Wraps an id as written; does not check that it is well-formed or declared.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `self` is `other` or nested (at any depth) inside it.
    #[must_use]
    pub fn is_within(&self, other: &ComponentId) -> bool {
        self.0
            .strip_prefix(&other.0)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    }
}

impl fmt::Display for ComponentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which side of the [`Origin`](turboflex_graph::Origin) split a component maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ComponentKind {
    /// Declared with `component`: maps workspace entities, may nest, may be a rule source.
    Workspace,
    /// Declared with `external`: maps dependency entities, top-level only, only a rule target.
    External,
}

/// A declared component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub id: ComponentId,
    pub kind: ComponentKind,
    pub doc: Option<String>,
    pub parent: Option<ComponentId>,
    /// Direct children, in declaration order.
    pub children: Vec<ComponentId>,
    /// The component's name argument.
    pub location: Location,
}

/// Index of a [`Pattern`] in [`Model::patterns`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PatternId(pub(crate) usize);

impl PatternId {
    #[must_use]
    pub fn index(self) -> usize {
        self.0
    }
}

/// One mapping pattern, owned by exactly one component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    pub owner: ComponentId,
    pub selector: Selector,
    pub follow: Follow,
    pub location: Location,
}

/// What a pattern matches.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Selector {
    /// `modules "a::b"` / `modules "a::b::**"`: entities by their [`Entity::module`].
    ///
    /// [`Entity::module`]: turboflex_graph::Entity::module
    Module(ModulePattern),
    /// `items "a::b::f"`: the one entity with exactly this path.
    Item(ItemPath),
    /// `crates "sqlx"` / `crates "rustc_*"` (external components only): entities by crate name.
    Crate(CratePattern),
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Module(pattern) => pattern.fmt(f),
            Self::Item(path) => path.fmt(f),
            Self::Crate(pattern) => pattern.fmt(f),
        }
    }
}

/// A module path, matched exactly or by its descendants.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModulePattern {
    pub module: ModulePath,
    pub scope: ModuleScope,
}

impl ModulePattern {
    #[must_use]
    pub fn matches(&self, module: &ModulePath) -> bool {
        let (pattern, module) = (self.module.as_str(), module.as_str());
        match self.scope {
            ModuleScope::Exact => module == pattern,
            ModuleScope::Descendants => module
                .strip_prefix(pattern)
                .is_some_and(|rest| rest.starts_with("::")),
        }
    }
}

impl fmt::Display for ModulePattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.scope {
            ModuleScope::Exact => write!(f, "{}", self.module),
            ModuleScope::Descendants => write!(f, "{}::**", self.module),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModuleScope {
    /// `a::b`: the module itself only.
    Exact,
    /// `a::b::**`: every module strictly below `a::b`, not `a::b` itself.
    Descendants,
}

/// A crate name, exact or by prefix.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CratePattern {
    /// `sqlx`
    Exact(CrateName),
    /// `rustc_*`: every crate whose name starts with the prefix (here `rustc_`).
    Prefix(String),
}

impl CratePattern {
    #[must_use]
    pub fn matches(&self, crate_name: &CrateName) -> bool {
        match self {
            Self::Exact(name) => name == crate_name,
            Self::Prefix(prefix) => crate_name.as_str().starts_with(prefix.as_str()),
        }
    }
}

impl fmt::Display for CratePattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact(name) => name.fmt(f),
            Self::Prefix(prefix) => write!(f, "{prefix}*"),
        }
    }
}

/// Whether an item pattern takes its private helpers along (`items ... follow=#true`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Follow {
    /// Only the named item moves.
    Off,
    /// The item's exclusive helpers in its module move with it; see the crate docs.
    Helpers,
}

/// An allowed dependency: code of `from` (or of any component nested in it) may depend on `to`
/// (or on any component nested in it), restricted to `kinds` when given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub from: ComponentId,
    pub to: ComponentId,
    /// `None` allows every [`EdgeGroup`].
    pub kinds: Option<BTreeSet<EdgeGroup>>,
    /// The target argument.
    pub location: Location,
}

/// A declared, unchecked relationship that is not a code dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub from: ComponentId,
    pub to: ComponentId,
    pub via: LinkVia,
    /// The target argument.
    pub location: Location,
}

/// How a [`Link`] connects its components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinkVia {
    /// `from` runs `to` as a subprocess.
    Process,
}

/// Whether edges seen only in `cfg(test)` builds are checked against the rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum TestEdges {
    /// [`Scope::TestOnly`](turboflex_graph::Scope::TestOnly) edges are ignored.
    #[default]
    Exclude,
    /// Test-only edges are checked like any other.
    Check,
}

/// Analysis knobs. Unmapped workspace entities are always reported; that is not a knob.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Policy {
    pub test_edges: TestEdges,
    /// The edge groups the model considers at all (`edges`); `None` considers every
    /// [`EdgeGroup`].
    pub edges: Option<BTreeSet<EdgeGroup>>,
    /// The edge provenances the model considers at all (`provenance`); `None` considers every
    /// [`Provenance`].
    pub provenance: Option<BTreeSet<Provenance>>,
    /// The components whose incoming dependencies the model considers at all (`targets`), each
    /// with the components nested in it; `None` considers every component.
    pub targets: Option<BTreeSet<ComponentId>>,
}

impl Policy {
    /// Whether edges of `group` are considered (not filtered out by `edges`).
    #[must_use]
    pub fn considers_group(&self, group: EdgeGroup) -> bool {
        self.edges
            .as_ref()
            .is_none_or(|edges| edges.contains(&group))
    }

    /// Whether edges of `provenance` are considered (not filtered out by `provenance`).
    #[must_use]
    pub fn considers_provenance(&self, provenance: Provenance) -> bool {
        self.provenance
            .as_ref()
            .is_none_or(|provenances| provenances.contains(&provenance))
    }

    /// Whether edges to `to` are considered (not filtered out by `targets`): `to` is, or is
    /// nested in, one of the targets.
    #[must_use]
    pub fn considers_target(&self, to: &ComponentId) -> bool {
        self.targets
            .as_ref()
            .is_none_or(|targets| targets.iter().any(|target| to.is_within(target)))
    }

    /// Whether a rule to `to` can apply to a considered edge: `to` is, or is nested in, one of
    /// the targets, or a target is nested in `to`. Rules that cannot are out of the view's
    /// scope.
    #[must_use]
    pub fn considers_rule_target(&self, to: &ComponentId) -> bool {
        self.targets.as_ref().is_none_or(|targets| {
            targets
                .iter()
                .any(|target| to.is_within(target) || target.is_within(to))
        })
    }
}

/// A model source file, as a [`Model::parse_with`] loader returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The file's identity: two includes that load the same name are the same file (loaded
    /// once, or a cycle). Also names the file in diagnostics and in
    /// [`Model::source_name_of`].
    pub name: String,
    pub text: String,
}

/// A validated architecture model. Build one with [`Model::parse`] or, if it has includes,
/// [`Model::parse_with`].
///
/// "Declaration order" is the order of a depth-first walk of the files: an included file's
/// declarations take the place of its `include` node, and a file included again adds nothing.
#[derive(Debug, Clone)]
pub struct Model {
    /// Every file read, indexed by [`SourceId`](crate::SourceId): the root first, then included
    /// files in the order they were first included.
    pub(crate) sources: Vec<String>,
    /// Declaration order: every parent before its children.
    pub(crate) components: Vec<Component>,
    pub(crate) index: HashMap<ComponentId, usize>,
    pub(crate) patterns: Vec<Pattern>,
    pub(crate) rules: Vec<Rule>,
    pub(crate) links: Vec<Link>,
    pub(crate) policy: Policy,
}

impl Model {
    /// Parses and validates a model without includes. `source_name` (usually the file name)
    /// labels diagnostics. Reports every problem found, not just the first; an `include` is one,
    /// as there is nothing to load it with.
    pub fn parse(source_name: &str, text: &str) -> Result<Self, ModelError> {
        crate::parse::parse_model(source_name, text, None)
    }

    /// Parses and validates a model whose files may `include` others. The crate does no I/O:
    /// `load(including, path)` is called for every `include`, with the [`Source::name`] of the
    /// file containing it (`source_name` for the root) and the path exactly as written, and
    /// returns the included file or a message saying why it cannot. It is typically a closure
    /// resolving `path` against the directory of `including` and reading the file.
    ///
    /// Files are told apart by name only, so the loader should return a canonical name, and
    /// `source_name` should be the name the loader would return for the root; otherwise a cycle
    /// through the root is found one step late, after loading the root again. A failed load is a
    /// problem located at the `include`, like any other.
    pub fn parse_with(
        source_name: &str,
        text: &str,
        mut load: impl FnMut(&str, &str) -> Result<Source, String>,
    ) -> Result<Self, ModelError> {
        crate::parse::parse_model(source_name, text, Some(&mut load))
    }

    /// The name of the root file, the one parsing started from.
    #[must_use]
    pub fn source_name(&self) -> &str {
        &self.sources[SourceId::ROOT.index()]
    }

    /// The name of the file `location` is in. `location` must come from this model.
    #[must_use]
    pub fn source_name_of(&self, location: Location) -> &str {
        &self.sources[location.source.index()]
    }

    /// The name of every file of the model, indexed by [`SourceId`]: the root first, then
    /// included files in the order they were first included.
    #[must_use]
    pub fn source_names(&self) -> &[String] {
        &self.sources
    }

    /// Every component, workspace and external, in declaration order (parents first).
    #[must_use]
    pub fn components(&self) -> &[Component] {
        &self.components
    }

    #[must_use]
    pub fn component(&self, id: &ComponentId) -> Option<&Component> {
        self.index.get(id).map(|&i| &self.components[i])
    }

    /// Top-level components, in declaration order.
    pub fn roots(&self) -> impl Iterator<Item = &Component> {
        self.components.iter().filter(|c| c.parent.is_none())
    }

    #[must_use]
    pub fn parent(&self, id: &ComponentId) -> Option<&Component> {
        self.component(id)?
            .parent
            .as_ref()
            .and_then(|parent| self.component(parent))
    }

    /// Direct children of `id`, in declaration order.
    pub fn children(&self, id: &ComponentId) -> impl Iterator<Item = &Component> {
        self.component(id)
            .into_iter()
            .flat_map(|c| &c.children)
            .filter_map(|child| self.component(child))
    }

    /// Ancestors of `id`, nearest first; `id` itself is not included.
    pub fn ancestors(&self, id: &ComponentId) -> impl Iterator<Item = &Component> {
        std::iter::successors(self.parent(id), |c| self.parent(&c.id))
    }

    /// Components nested (at any depth) inside `id`, parents before children; `id` itself is not
    /// included.
    pub fn descendants(&self, id: &ComponentId) -> impl Iterator<Item = &Component> {
        let id = id.clone();
        self.components
            .iter()
            .filter(move |c| c.id != id && c.id.is_within(&id))
    }

    /// Every pattern, in declaration order.
    #[must_use]
    pub fn patterns(&self) -> &[Pattern] {
        &self.patterns
    }

    #[must_use]
    pub fn pattern(&self, id: PatternId) -> &Pattern {
        &self.patterns[id.0]
    }

    /// The patterns `id` owns directly (not those of its children).
    pub fn patterns_of(&self, id: &ComponentId) -> impl Iterator<Item = (PatternId, &Pattern)> {
        self.patterns
            .iter()
            .enumerate()
            .filter(move |(_, p)| &p.owner == id)
            .map(|(i, p)| (PatternId(i), p))
    }

    /// Allowed dependencies, one per `(from, to)` pair, in declaration order.
    #[must_use]
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    #[must_use]
    pub fn links(&self) -> &[Link] {
        &self.links
    }

    #[must_use]
    pub fn policy(&self) -> &Policy {
        &self.policy
    }
}
