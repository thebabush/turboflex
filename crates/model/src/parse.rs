//! KDL source to a validated [`Model`]. Collects every problem instead of stopping at the first.

use std::collections::btree_map::{BTreeMap, Entry};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};
use miette::SourceSpan;
use turboflex_graph::{CrateName, EdgeGroup, ItemPath, ModulePath, Provenance};

use crate::error::{Location, ModelError, Problem, ProblemKind, SourceFile, SourceId};
use crate::model::{
    Component, ComponentId, ComponentKind, CratePattern, Follow, Link, LinkVia, Model,
    ModulePattern, ModuleScope, Pattern, Policy, Rule, Selector, Source, TestEdges,
};

const TOP_LEVEL: &str = "component, external, allow, link, policy, include";
const COMPONENT_BODY: &str = "doc, modules, items, component";
const EXTERNAL_BODY: &str = "doc, crates, modules, items";
const POLICY_BODY: &str = "test-edges, edges, provenance, targets";
const ARROW: &str = "->";
const ARROW_ARGS: &str = "a source component, `->`, then one or more target components";
const EDGE_GROUPS: &str = "a comma-separated list of edge groups: types, calls, impls, macros, \
                           globals, constructs, matches";
const EDGE_GROUP_ARGS: &str = "one or more distinct edge groups: types, calls, impls, macros, \
                               globals, constructs, matches";
const PROVENANCE_ARGS: &str = "one or more distinct provenances: source, macro_expanded, \
                               desugared, derive";
const TARGETS_ARGS: &str = "one or more distinct component ids";

/// What [`Model::parse_with`](crate::Model::parse_with) calls for every `include`.
pub(crate) type Load<'l> = dyn FnMut(&str, &str) -> Result<Source, String> + 'l;

pub(crate) fn parse_model(
    source_name: &str,
    text: &str,
    load: Option<&mut Load<'_>>,
) -> Result<Model, ModelError> {
    let mut parser = Parser {
        load,
        sources: Vec::new(),
        current: SourceId::ROOT,
        including: Vec::new(),
        incomplete: false,
        problems: Vec::new(),
        components: Vec::new(),
        index: HashMap::new(),
        patterns: Vec::new(),
        pattern_owners: BTreeMap::new(),
        rules: Vec::new(),
        links: Vec::new(),
        policy: None,
        targets: None,
    };
    parser.parse_source(source_name.to_owned(), text.to_owned());
    parser.resolve()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArrowNode {
    Allow,
    Link,
}

/// `allow` / `link` as written, before their components are resolved.
struct RawArrow {
    from: Spanned<String>,
    to: Vec<Spanned<String>>,
    /// Located at the `kinds` property.
    kinds: Option<Spanned<BTreeSet<EdgeGroup>>>,
    /// Always `Some` for a valid `link`.
    via: Option<LinkVia>,
}

#[derive(Clone)]
struct Spanned<T> {
    value: T,
    location: Location,
}

struct Parser<'a, 'l> {
    load: Option<&'a mut Load<'l>>,
    /// Every file read, indexed by [`SourceId`].
    sources: Vec<SourceFile>,
    /// The file being parsed.
    current: SourceId,
    /// The files being parsed, the root first and `current` last: each includes the next.
    including: Vec<SourceId>,
    /// Some file could not be read or parsed, so references to what it declares would be
    /// reported as unknown: skip resolving them.
    incomplete: bool,
    problems: Vec<Problem>,
    components: Vec<Component>,
    index: HashMap<ComponentId, usize>,
    patterns: Vec<Pattern>,
    pattern_owners: BTreeMap<Selector, (ComponentId, Location)>,
    rules: Vec<RawArrow>,
    links: Vec<RawArrow>,
    policy: Option<(Location, Policy)>,
    /// The ids `policy`'s `targets` names, each at its argument; resolved with the rest.
    targets: Option<BTreeMap<String, Location>>,
}

impl Parser<'_, '_> {
    fn locate(&self, span: SourceSpan) -> Location {
        self.sources[self.current.index()].locate(span)
    }

    fn problem(&mut self, kind: ProblemKind, location: Location) {
        self.problems.push(Problem {
            kind,
            location,
            previous: None,
        });
    }

    /// Parses one file, root or included, in place: its declarations join those parsed so far.
    fn parse_source(&mut self, name: String, text: String) {
        let id = SourceId(self.sources.len());
        let source = SourceFile::new(id, name, text);
        let text = Arc::clone(&source.text);
        self.sources.push(source);
        let outer = std::mem::replace(&mut self.current, id);
        self.including.push(id);
        match KdlDocument::parse_v2(&text) {
            Ok(document) => self.parse_document(&document),
            Err(error) => {
                for diagnostic in &error.diagnostics {
                    let mut message = diagnostic
                        .message
                        .clone()
                        .unwrap_or_else(|| "invalid KDL".to_owned());
                    if let Some(help) = &diagnostic.help {
                        message = format!("{message} ({help})");
                    }
                    let location = self.locate(diagnostic.span);
                    self.problem(ProblemKind::Syntax { message }, location);
                }
                self.incomplete = true;
            }
        }
        self.including.pop();
        self.current = outer;
    }

    fn parse_document(&mut self, document: &KdlDocument) {
        for node in document.nodes() {
            match node.name().value() {
                "component" => self.parse_component(node, None, ComponentKind::Workspace),
                "external" => self.parse_component(node, None, ComponentKind::External),
                "include" => self.parse_include(node),
                "foundation" => {
                    let location = self.locate(node.name().span());
                    self.problem(ProblemKind::FoundationRemoved, location);
                }
                "allow" => {
                    if let Some(rule) = self.parse_arrow(node, ArrowNode::Allow) {
                        self.rules.push(rule);
                    }
                }
                "link" => {
                    if let Some(link) = self.parse_arrow(node, ArrowNode::Link) {
                        self.links.push(link);
                    }
                }
                "policy" => {
                    if self.in_root(node, "policy") {
                        self.parse_policy(node);
                    }
                }
                name => self.unknown_node(node, name, TOP_LEVEL),
            }
        }
    }

    /// Whether the file being parsed is the root one; if not, reports `node` as misplaced.
    fn in_root(&mut self, node: &KdlNode, node_name: &'static str) -> bool {
        if self.current == SourceId::ROOT {
            return true;
        }
        let location = self.locate(node.name().span());
        self.problem(ProblemKind::RootOnly { node: node_name }, location);
        false
    }

    /// `include "PATH"`: loads the file and parses it in place, unless it is already loaded.
    fn parse_include(&mut self, node: &KdlNode) {
        let args = self.parse_list(node, "include");
        let [path] = args.as_slice() else {
            self.bad_arguments(node, "include", "exactly one path");
            self.incomplete = true;
            return;
        };
        let including = self.sources[self.current.index()].name.clone();
        let Some(load) = self.load.as_deref_mut() else {
            self.problem(ProblemKind::IncludeWithoutLoader, path.location);
            self.incomplete = true;
            return;
        };
        let source = match load(&including, &path.value) {
            Ok(source) => source,
            Err(message) => {
                let (location, path) = (path.location, path.value.clone());
                self.problem(ProblemKind::IncludeFailed { path, message }, location);
                self.incomplete = true;
                return;
            }
        };
        let name_of = |id: &SourceId| self.sources[id.index()].name.as_str();
        if let Some(start) = self
            .including
            .iter()
            .position(|id| name_of(id) == source.name)
        {
            let chain = self.including[start..]
                .iter()
                .map(|id| name_of(id).to_owned())
                .chain(std::iter::once(source.name))
                .collect();
            self.problem(ProblemKind::IncludeCycle { chain }, path.location);
        } else if !self.sources.iter().any(|loaded| loaded.name == source.name) {
            self.parse_source(source.name, source.text);
        }
    }

    fn unknown_node(&mut self, node: &KdlNode, name: &str, expected: &'static str) {
        let name = name.to_owned();
        let location = self.locate(node.name().span());
        self.problem(ProblemKind::UnknownNode { name, expected }, location);
    }

    /// Reports type annotations on the node or its entries.
    fn check_annotations(&mut self, node: &KdlNode) {
        if let Some(ty) = node.ty() {
            let location = self.locate(ty.span());
            self.problem(ProblemKind::TypeAnnotation, location);
        }
        for entry in node.entries() {
            if entry.ty().is_some() {
                let location = self.locate(entry.span());
                self.problem(ProblemKind::TypeAnnotation, location);
            }
        }
    }

    /// The positional arguments, which must all be strings. Non-strings are reported and skipped.
    fn string_args(&mut self, node: &KdlNode) -> Vec<Spanned<String>> {
        let mut args = Vec::new();
        for entry in node.entries().iter().filter(|e| e.name().is_none()) {
            let location = self.locate(entry.span());
            match entry.value() {
                KdlValue::String(value) => args.push(Spanned {
                    value: value.clone(),
                    location,
                }),
                other @ (KdlValue::Integer(_)
                | KdlValue::Float(_)
                | KdlValue::Bool(_)
                | KdlValue::Null) => {
                    let found = other.to_string();
                    self.problem(ProblemKind::NotAString { found }, location);
                }
            }
        }
        args
    }

    /// The properties of `node`, reporting duplicates and names outside `allowed`.
    fn props<'n>(
        &mut self,
        node: &'n KdlNode,
        node_name: &'static str,
        allowed: &'static [&'static str],
    ) -> HashMap<&'static str, (&'n KdlEntry, Location)> {
        let mut props = HashMap::new();
        for entry in node.entries() {
            let Some(name) = entry.name() else { continue };
            let location = self.locate(entry.span());
            let Some(&known) = allowed.iter().find(|&&a| a == name.value()) else {
                let kind = if allowed.is_empty() {
                    ProblemKind::UnexpectedProperty { node: node_name }
                } else {
                    ProblemKind::UnknownProperty {
                        node: node_name,
                        name: name.value().to_owned(),
                        expected: allowed.join(", "),
                    }
                };
                self.problem(kind, location);
                continue;
            };
            if props.insert(known, (entry, location)).is_some() {
                let name = known.to_owned();
                self.problem(ProblemKind::DuplicateProperty { name }, location);
            }
        }
        props
    }

    fn no_children(&mut self, node: &KdlNode, node_name: &'static str) {
        if node.children().is_some() {
            let location = self.locate(node.span());
            self.problem(
                ProblemKind::UnexpectedChildren { node: node_name },
                location,
            );
        }
    }

    /// A node of the form `name "arg" "arg" ...` with no properties or children.
    fn parse_list(&mut self, node: &KdlNode, node_name: &'static str) -> Vec<Spanned<String>> {
        self.check_annotations(node);
        self.props(node, node_name, &[]);
        self.no_children(node, node_name);
        self.string_args(node)
    }

    fn bad_arguments(&mut self, node: &KdlNode, node_name: &'static str, expected: &'static str) {
        let location = self.locate(node.span());
        self.problem(
            ProblemKind::BadArguments {
                node: node_name,
                expected,
            },
            location,
        );
    }

    fn parse_component(
        &mut self,
        node: &KdlNode,
        parent: Option<&ComponentId>,
        kind: ComponentKind,
    ) {
        let node_name = match kind {
            ComponentKind::Workspace => "component",
            ComponentKind::External => "external",
        };
        self.check_annotations(node);
        self.props(node, node_name, &[]);
        let args = self.string_args(node);
        let [name] = args.as_slice() else {
            self.bad_arguments(node, node_name, "exactly one name");
            return;
        };
        if !is_component_name(&name.value) {
            let kind = ProblemKind::InvalidComponentName {
                name: name.value.clone(),
            };
            self.problem(kind, name.location);
        }
        let id = ComponentId::new(match parent {
            Some(parent) => format!("{parent}.{}", name.value),
            None => name.value.clone(),
        });
        if let Some(&first) = self.index.get(&id) {
            self.problems.push(Problem {
                kind: ProblemKind::DuplicateComponent { id },
                location: name.location,
                previous: Some(self.components[first].location),
            });
            return;
        }
        self.index.insert(id.clone(), self.components.len());
        self.components.push(Component {
            id: id.clone(),
            kind,
            doc: None,
            parent: parent.cloned(),
            children: Vec::new(),
            location: name.location,
        });
        if let Some(&parent) = parent.and_then(|parent| self.index.get(parent)) {
            self.components[parent].children.push(id.clone());
        }
        let Some(body) = node.children() else { return };
        for child in body.nodes() {
            match kind {
                ComponentKind::Workspace => self.parse_component_body(&id, child),
                ComponentKind::External => self.parse_external_body(&id, child),
            }
        }
    }

    fn parse_component_body(&mut self, id: &ComponentId, node: &KdlNode) {
        match node.name().value() {
            "doc" => self.parse_doc(id, node),
            "modules" => self.parse_modules(id, node),
            "items" => self.parse_items(id, node),
            "component" => self.parse_component(node, Some(id), ComponentKind::Workspace),
            name => self.unknown_node(node, name, COMPONENT_BODY),
        }
    }

    fn parse_external_body(&mut self, id: &ComponentId, node: &KdlNode) {
        match node.name().value() {
            "doc" => self.parse_doc(id, node),
            "crates" => self.parse_crates(id, node),
            "modules" => self.parse_modules(id, node),
            "items" => self.parse_items(id, node),
            name => self.unknown_node(node, name, EXTERNAL_BODY),
        }
    }

    fn component_mut(&mut self, id: &ComponentId) -> Option<&mut Component> {
        let &i = self.index.get(id)?;
        self.components.get_mut(i)
    }

    fn parse_doc(&mut self, id: &ComponentId, node: &KdlNode) {
        let args = self.parse_list(node, "doc");
        let [text] = args.as_slice() else {
            self.bad_arguments(node, "doc", "exactly one string");
            return;
        };
        let location = self.locate(node.name().span());
        let Some(component) = self.component_mut(id) else {
            return;
        };
        if component.doc.is_some() {
            self.problem(ProblemKind::DuplicateNode { node: "doc" }, location);
        } else {
            component.doc = Some(text.value.clone());
        }
    }

    fn parse_modules(&mut self, id: &ComponentId, node: &KdlNode) {
        let args = self.parse_list(node, "modules");
        if !has_args(node) {
            self.bad_arguments(node, "modules", "one or more module patterns");
        }
        for arg in args {
            match parse_module_pattern(&arg.value) {
                Ok(pattern) => {
                    self.add_pattern(id, Selector::Module(pattern), Follow::Off, arg.location);
                }
                Err(reason) => self.malformed("module", arg, reason),
            }
        }
    }

    fn parse_items(&mut self, id: &ComponentId, node: &KdlNode) {
        self.check_annotations(node);
        let external = self
            .component_mut(id)
            .is_some_and(|c| c.kind == ComponentKind::External);
        let allowed: &'static [&'static str] = if external { &[] } else { &["follow"] };
        let props = self.props(node, "items", allowed);
        let mut follow = Follow::Off;
        if let Some((entry, location)) = props.get("follow") {
            match entry.value() {
                KdlValue::Bool(true) => follow = Follow::Helpers,
                KdlValue::Bool(false) => follow = Follow::Off,
                KdlValue::String(_)
                | KdlValue::Integer(_)
                | KdlValue::Float(_)
                | KdlValue::Null => {
                    let kind = ProblemKind::BadPropertyValue {
                        name: "follow",
                        expected: "#true or #false",
                    };
                    self.problem(kind, *location);
                }
            }
        }
        self.no_children(node, "items");
        let args = self.string_args(node);
        if !has_args(node) {
            self.bad_arguments(node, "items", "one or more item paths");
        }
        for arg in args {
            match parse_item_pattern(&arg.value) {
                Ok(path) => self.add_pattern(id, Selector::Item(path), follow, arg.location),
                Err(reason) => self.malformed("item", arg, reason),
            }
        }
    }

    fn parse_crates(&mut self, id: &ComponentId, node: &KdlNode) {
        let args = self.parse_list(node, "crates");
        if !has_args(node) {
            self.bad_arguments(node, "crates", "one or more crate names");
        }
        for arg in args {
            match parse_crate_pattern(&arg.value) {
                Ok(pattern) => {
                    self.add_pattern(id, Selector::Crate(pattern), Follow::Off, arg.location);
                }
                Err(reason) => self.malformed("crate", arg, reason),
            }
        }
    }

    fn malformed(&mut self, what: &'static str, arg: Spanned<String>, reason: &'static str) {
        let kind = ProblemKind::MalformedPattern {
            what,
            pattern: arg.value,
            reason,
        };
        self.problem(kind, arg.location);
    }

    fn add_pattern(
        &mut self,
        owner: &ComponentId,
        selector: Selector,
        follow: Follow,
        location: Location,
    ) {
        match self.pattern_owners.entry(selector.clone()) {
            Entry::Occupied(first) => {
                let (first_owner, first_location) = first.get();
                self.problems.push(Problem {
                    kind: ProblemKind::DuplicatePattern {
                        pattern: selector.to_string(),
                        first: first_owner.clone(),
                        second: owner.clone(),
                    },
                    location,
                    previous: Some(*first_location),
                });
            }
            Entry::Vacant(slot) => {
                slot.insert((owner.clone(), location));
                self.patterns.push(Pattern {
                    owner: owner.clone(),
                    selector,
                    follow,
                    location,
                });
            }
        }
    }

    /// `allow "a" -> "b" "c" kinds="types"` or `link "a" -> "b" via="process"`.
    fn parse_arrow(&mut self, node: &KdlNode, arrow: ArrowNode) -> Option<RawArrow> {
        let (node_name, allowed): (_, &'static [&'static str]) = match arrow {
            ArrowNode::Allow => ("allow", &["kinds"]),
            ArrowNode::Link => ("link", &["via"]),
        };
        self.check_annotations(node);
        self.no_children(node, node_name);
        let props = self.props(node, node_name, allowed);
        let args = self.string_args(node);
        let mut kinds = None;
        if let Some(&(entry, location)) = props.get("kinds") {
            kinds = entry
                .value()
                .as_string()
                .and_then(parse_edge_groups)
                .map(|value| Spanned { value, location });
            if kinds.is_none() {
                let kind = ProblemKind::BadPropertyValue {
                    name: "kinds",
                    expected: EDGE_GROUPS,
                };
                self.problem(kind, location);
            }
        }
        let via = props
            .get("via")
            .and_then(|(entry, _)| entry.value().as_string())
            .and_then(parse_link_via);
        if arrow == ArrowNode::Link && via.is_none() {
            let location = props
                .get("via")
                .map_or_else(|| self.locate(node.span()), |(_, location)| *location);
            let kind = ProblemKind::BadPropertyValue {
                name: "via",
                expected: "via=\"process\"",
            };
            self.problem(kind, location);
        }
        match args.as_slice() {
            [from, arrow, to @ ..] if arrow.value == ARROW && !to.is_empty() => Some(RawArrow {
                from: from.clone(),
                to: to.to_vec(),
                kinds,
                via,
            }),
            _ => {
                self.bad_arguments(node, node_name, ARROW_ARGS);
                None
            }
        }
    }

    fn parse_policy(&mut self, node: &KdlNode) {
        let location = self.locate(node.name().span());
        self.check_annotations(node);
        self.props(node, "policy", &[]);
        if !self.string_args(node).is_empty() {
            self.bad_arguments(node, "policy", "no arguments, only a children block");
        }
        let mut policy = Policy::default();
        let mut targets = None;
        let (mut test_edges_at, mut edges_at, mut provenance_at, mut targets_at) =
            (None, None, None, None);
        for child in node.children().into_iter().flat_map(KdlDocument::nodes) {
            match child.name().value() {
                "test-edges" => {
                    let args = self.parse_list(child, "test-edges");
                    let parsed = match args.as_slice() {
                        [value] => parse_test_edges(&value.value),
                        _ => None,
                    };
                    let first = self.first_in_policy(child, "test-edges", &mut test_edges_at);
                    match parsed {
                        Some(test_edges) if first => policy.test_edges = test_edges,
                        Some(_) => {}
                        None => self.bad_arguments(
                            child,
                            "test-edges",
                            "exactly one of \"exclude\" or \"check\"",
                        ),
                    }
                }
                "edges" => {
                    let edges = self.parse_set(child, "edges", EDGE_GROUP_ARGS, parse_edge_group);
                    if self.first_in_policy(child, "edges", &mut edges_at) {
                        policy.edges = edges;
                    }
                }
                "provenance" => {
                    let provenance =
                        self.parse_set(child, "provenance", PROVENANCE_ARGS, parse_provenance);
                    if self.first_in_policy(child, "provenance", &mut provenance_at) {
                        policy.provenance = provenance;
                    }
                }
                "targets" => {
                    let ids = self.parse_located_set(child, "targets", TARGETS_ARGS, |id| {
                        Some(id.to_owned())
                    });
                    if self.first_in_policy(child, "targets", &mut targets_at) {
                        targets = ids;
                    }
                }
                name => self.unknown_node(child, name, POLICY_BODY),
            }
        }
        if let Some((first, _)) = &self.policy {
            self.problems.push(Problem {
                kind: ProblemKind::DuplicateNode { node: "policy" },
                location,
                previous: Some(*first),
            });
        } else {
            self.policy = Some((location, policy));
            self.targets = targets;
        }
    }

    /// `NODE "NAME"...`: a non-empty set of distinct names, each read by `parse`. `None` (with
    /// the problems reported) unless every name is known.
    fn parse_set<T: Ord>(
        &mut self,
        node: &KdlNode,
        node_name: &'static str,
        expected: &'static str,
        parse: fn(&str) -> Option<T>,
    ) -> Option<BTreeSet<T>> {
        self.parse_located_set(node, node_name, expected, parse)
            .map(|seen| seen.into_keys().collect())
    }

    /// Like [`Self::parse_set`], keeping each name's location.
    fn parse_located_set<T: Ord>(
        &mut self,
        node: &KdlNode,
        node_name: &'static str,
        expected: &'static str,
        parse: fn(&str) -> Option<T>,
    ) -> Option<BTreeMap<T, Location>> {
        let args = self.parse_list(node, node_name);
        if !has_args(node) {
            self.bad_arguments(node, node_name, expected);
            return None;
        }
        // Non-string arguments and properties are already reported.
        let mut valid = args.len() == node.entries().len();
        let mut seen: BTreeMap<T, Location> = BTreeMap::new();
        for arg in args {
            let Some(value) = parse(&arg.value) else {
                let kind = ProblemKind::BadArguments {
                    node: node_name,
                    expected,
                };
                self.problem(kind, arg.location);
                valid = false;
                continue;
            };
            match seen.entry(value) {
                Entry::Occupied(first) => self.problems.push(Problem {
                    kind: ProblemKind::DuplicateValue {
                        node: node_name,
                        value: arg.value,
                    },
                    location: arg.location,
                    previous: Some(*first.get()),
                }),
                Entry::Vacant(slot) => {
                    slot.insert(arg.location);
                }
            }
        }
        valid.then_some(seen)
    }

    /// Whether `node`, a `node_name` child of `policy`, is the first one. `seen` holds the first
    /// one's location; later ones are reported as duplicates.
    fn first_in_policy(
        &mut self,
        node: &KdlNode,
        node_name: &'static str,
        seen: &mut Option<Location>,
    ) -> bool {
        let location = self.locate(node.name().span());
        if let Some(first) = *seen {
            self.problems.push(Problem {
                kind: ProblemKind::DuplicateNode { node: node_name },
                location,
                previous: Some(first),
            });
            return false;
        }
        *seen = Some(location);
        true
    }

    /// Resolves component references and returns the model, or every problem found.
    fn resolve(mut self) -> Result<Model, ModelError> {
        if self.incomplete {
            return Err(ModelError::new(&self.sources, self.problems));
        }
        let mut policy = self
            .policy
            .take()
            .map_or_else(Policy::default, |(_, policy)| policy);
        policy.targets = self.resolve_targets();
        let mut rules = Vec::new();
        let mut seen_rules: HashMap<(ComponentId, ComponentId), Location> = HashMap::new();
        for raw in std::mem::take(&mut self.rules) {
            if let (Some(kinds), Some(edges)) = (&raw.kinds, &policy.edges)
                && kinds.value.is_disjoint(edges)
            {
                self.problem(ProblemKind::KindsOutsidePolicy, kinds.location);
            }
            let Some(from) = self.lookup_source(&raw.from, "rule") else {
                continue;
            };
            for target in raw.to {
                let Some(to) = self.lookup(&target) else {
                    continue;
                };
                let key = (from.clone(), to.clone());
                if let Some(&first) = seen_rules.get(&key) {
                    self.problems.push(Problem {
                        kind: ProblemKind::DuplicateRule {
                            from: key.0,
                            to: key.1,
                        },
                        location: target.location,
                        previous: Some(first),
                    });
                    continue;
                }
                seen_rules.insert(key, target.location);
                rules.push(Rule {
                    from: from.clone(),
                    to,
                    kinds: raw.kinds.as_ref().map(|kinds| kinds.value.clone()),
                    location: target.location,
                });
            }
        }
        let mut links = Vec::new();
        for raw in std::mem::take(&mut self.links) {
            let Some(from) = self.lookup_source(&raw.from, "link") else {
                continue;
            };
            let Some(via) = raw.via else { continue };
            for target in raw.to {
                let Some(to) = self.lookup(&target) else {
                    continue;
                };
                if self.component_kind(&to) == Some(ComponentKind::External) {
                    let kind = ProblemKind::BadArguments {
                        node: "link",
                        expected: "workspace components on both ends",
                    };
                    self.problem(kind, target.location);
                    continue;
                }
                links.push(Link {
                    from: from.clone(),
                    to,
                    via,
                    location: target.location,
                });
            }
        }
        if !self.problems.is_empty() {
            return Err(ModelError::new(&self.sources, self.problems));
        }
        Ok(Model {
            sources: self.sources.into_iter().map(|source| source.name).collect(),
            components: self.components,
            index: self.index,
            patterns: self.patterns,
            rules,
            links,
            policy,
        })
    }

    /// The ids `policy`'s `targets` names, if given. Unknown ones are reported and left out.
    fn resolve_targets(&mut self) -> Option<BTreeSet<ComponentId>> {
        let raw = self.targets.take()?;
        Some(
            raw.into_iter()
                .filter_map(|(value, location)| self.lookup(&Spanned { value, location }))
                .collect(),
        )
    }

    fn component_kind(&self, id: &ComponentId) -> Option<ComponentKind> {
        self.index.get(id).map(|&i| self.components[i].kind)
    }

    fn lookup(&mut self, name: &Spanned<String>) -> Option<ComponentId> {
        let id = ComponentId::new(name.value.clone());
        if self.index.contains_key(&id) {
            return Some(id);
        }
        let id = name.value.clone();
        self.problem(ProblemKind::UnknownComponent { id }, name.location);
        None
    }

    /// Like [`Self::lookup`], and the component must be a workspace one.
    fn lookup_source(&mut self, name: &Spanned<String>, node: &'static str) -> Option<ComponentId> {
        let id = self.lookup(name)?;
        match self.component_kind(&id)? {
            ComponentKind::Workspace => Some(id),
            ComponentKind::External => {
                self.problem(ProblemKind::ExternalSource { id, node }, name.location);
                None
            }
        }
    }
}

/// Whether `node` has positional arguments (strings or not).
fn has_args(node: &KdlNode) -> bool {
    node.entries().iter().any(|e| e.name().is_none())
}

/// `[A-Za-z][A-Za-z0-9_-]*`: one level of a component id (dots are added by nesting).
fn is_component_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A Rust identifier, optionally raw (`r#type`).
fn is_identifier(segment: &str) -> bool {
    let segment = segment.strip_prefix("r#").unwrap_or(segment);
    let mut chars = segment.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

fn parse_module_pattern(text: &str) -> Result<ModulePattern, &'static str> {
    let (module, scope) = match text.strip_suffix("::**") {
        Some(module) => (module, ModuleScope::Descendants),
        None => (text, ModuleScope::Exact),
    };
    if module.contains('*') {
        return Err("`**` is only allowed as the last segment, as `a::b::**`");
    }
    if !module.split("::").all(is_identifier) {
        return Err("expected `::`-separated identifiers, optionally ending in `::**`");
    }
    Ok(ModulePattern {
        module: ModulePath::new(module),
        scope,
    })
}

fn parse_item_pattern(text: &str) -> Result<ItemPath, &'static str> {
    if text.is_empty() || text.trim() != text {
        return Err("expected an entity path without surrounding whitespace");
    }
    if text.ends_with("::**") {
        return Err("item patterns are exact paths; use `modules` for `::**`");
    }
    Ok(ItemPath::new(text))
}

fn parse_crate_pattern(text: &str) -> Result<CratePattern, &'static str> {
    let (name, prefix) = match text.strip_suffix('*') {
        Some(name) => (name, true),
        None => (text, false),
    };
    let well_formed = if prefix {
        // A prefix may end in `_` (`rustc_*`) but must itself start like an identifier.
        name.chars().next().is_some_and(char::is_alphabetic)
            && name.chars().all(|c| c.is_alphanumeric() || c == '_')
    } else {
        is_identifier(name)
    };
    if !well_formed {
        return Err(
            "expected a crate name as rustc sees it (`-` written as `_`), optionally ending in `*`",
        );
    }
    Ok(if prefix {
        CratePattern::Prefix(name.to_owned())
    } else {
        CratePattern::Exact(CrateName::new(name))
    })
}

fn parse_edge_groups(text: &str) -> Option<BTreeSet<EdgeGroup>> {
    text.split(',')
        .map(|group| parse_edge_group(group.trim()))
        .collect()
}

fn parse_edge_group(text: &str) -> Option<EdgeGroup> {
    match text {
        "types" => Some(EdgeGroup::Types),
        "calls" => Some(EdgeGroup::Calls),
        "impls" => Some(EdgeGroup::Impls),
        "macros" => Some(EdgeGroup::Macros),
        "globals" => Some(EdgeGroup::Globals),
        "constructs" => Some(EdgeGroup::Constructs),
        "matches" => Some(EdgeGroup::Matches),
        _ => None,
    }
}

fn parse_provenance(text: &str) -> Option<Provenance> {
    match text {
        "source" => Some(Provenance::Source),
        "macro_expanded" => Some(Provenance::MacroExpanded),
        "desugared" => Some(Provenance::Desugared),
        "derive" => Some(Provenance::Derive),
        _ => None,
    }
}

fn parse_link_via(text: &str) -> Option<LinkVia> {
    match text {
        "process" => Some(LinkVia::Process),
        _ => None,
    }
}

fn parse_test_edges(text: &str) -> Option<TestEdges> {
    match text {
        "exclude" => Some(TestEdges::Exclude),
        "check" => Some(TestEdges::Check),
        _ => None,
    }
}
