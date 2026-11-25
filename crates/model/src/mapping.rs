//! Which component owns each entity of a [`Graph`], and why.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use turboflex_graph::{
    CrateName, EdgeKind, Entity, EntityKind, Graph, ItemPath, ModulePath, Origin,
};

use crate::model::{
    ComponentId, ComponentKind, CratePattern, Follow, Model, ModuleScope, PatternId, Selector,
};

/// An entity's owner and the reason it was chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub component: ComponentId,
    pub reason: Reason,
}

/// Why an entity belongs to a component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The most specific pattern matching the entity.
    Pattern(PatternId),
    /// A helper that follows `anchor`, an item mapped by the `follow=#true` item pattern
    /// `pattern`.
    Follows {
        anchor: ItemPath,
        pattern: PatternId,
    },
}

/// Where an entity ended up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    Owned(Assignment),
    /// A workspace entity no pattern matches.
    Unmapped,
    /// Several components claim the entity with nothing to choose between them; it has no owner
    /// until the model is fixed. Holds every claim.
    Conflict(Vec<Assignment>),
}

/// The component of every entity of a graph under a model.
///
/// Workspace entities are mapped by workspace components and external entities by external
/// components; a pattern of one kind never matches an entity of the other origin. A workspace
/// entity nothing matches is [`Placement::Unmapped`]; an external entity nothing matches is out of
/// scope and has no placement at all.
#[derive(Debug, Clone, Default)]
pub struct Mapping {
    placements: BTreeMap<ItemPath, Placement>,
    unused_patterns: Vec<PatternId>,
}

impl Mapping {
    #[must_use]
    pub fn new(model: &Model, graph: &Graph) -> Self {
        let workspace = Matcher::new(model, ComponentKind::Workspace);
        let external = Matcher::new(model, ComponentKind::External);
        let mut used = vec![false; model.patterns().len()];
        let mut placements = BTreeMap::new();
        for entity in graph.entities() {
            let matcher = match entity.origin {
                Origin::Workspace => &workspace,
                Origin::External => &external,
            };
            let placement = match matcher.best(entity) {
                Some(pattern) => {
                    used[pattern.0] = true;
                    Placement::Owned(Assignment {
                        component: model.pattern(pattern).owner.clone(),
                        reason: Reason::Pattern(pattern),
                    })
                }
                None => match entity.origin {
                    Origin::Workspace => Placement::Unmapped,
                    Origin::External => continue,
                },
            };
            placements.insert(entity.path.clone(), placement);
        }
        let mut mapping = Self {
            placements,
            unused_patterns: (0..used.len())
                .filter(|&i| !used[i])
                .map(PatternId)
                .collect(),
        };
        mapping.follow_helpers(model, graph);
        mapping
    }

    /// The placement of `path`; `None` for out-of-scope external entities and paths not in the
    /// graph.
    #[must_use]
    pub fn placement(&self, path: &ItemPath) -> Option<&Placement> {
        self.placements.get(path)
    }

    #[must_use]
    pub fn assignment(&self, path: &ItemPath) -> Option<&Assignment> {
        match self.placements.get(path)? {
            Placement::Owned(assignment) => Some(assignment),
            Placement::Unmapped | Placement::Conflict(_) => None,
        }
    }

    #[must_use]
    pub fn component_of(&self, path: &ItemPath) -> Option<&ComponentId> {
        self.assignment(path)
            .map(|assignment| &assignment.component)
    }

    /// Every placed entity (all workspace entities, and the external entities some external
    /// component claims), ordered by path.
    pub fn placements(&self) -> impl Iterator<Item = (&ItemPath, &Placement)> {
        self.placements.iter()
    }

    /// Entities with an owner, ordered by path.
    pub fn assignments(&self) -> impl Iterator<Item = (&ItemPath, &Assignment)> {
        self.placements
            .iter()
            .filter_map(|(path, placement)| match placement {
                Placement::Owned(assignment) => Some((path, assignment)),
                Placement::Unmapped | Placement::Conflict(_) => None,
            })
    }

    /// Workspace entities no pattern matches, ordered by path.
    pub fn unmapped(&self) -> impl Iterator<Item = &ItemPath> {
        self.placements
            .iter()
            .filter_map(|(path, placement)| match placement {
                Placement::Unmapped => Some(path),
                Placement::Owned(_) | Placement::Conflict(_) => None,
            })
    }

    /// Entities claimed by several components, with every claim, ordered by path.
    pub fn conflicts(&self) -> impl Iterator<Item = (&ItemPath, &[Assignment])> {
        self.placements
            .iter()
            .filter_map(|(path, placement)| match placement {
                Placement::Conflict(claims) => Some((path, claims.as_slice())),
                Placement::Owned(_) | Placement::Unmapped => None,
            })
    }

    /// Patterns that matched no entity of the graph (typos, or code that moved or is gone).
    #[must_use]
    pub fn unused_patterns(&self) -> &[PatternId] {
        &self.unused_patterns
    }

    /// Moves the exclusive helpers of `follow=#true` items; see the crate docs for the rule.
    fn follow_helpers(&mut self, model: &Model, graph: &Graph) {
        // Anchors grouped by (target component, module).
        let mut groups: BTreeMap<(ComponentId, ModulePath), Vec<(&Entity, PatternId)>> =
            BTreeMap::new();
        for (i, pattern) in model.patterns().iter().enumerate() {
            let Selector::Item(path) = &pattern.selector else {
                continue;
            };
            if pattern.follow == Follow::Off {
                continue;
            }
            let Some(anchor) = graph.entity(path) else {
                continue;
            };
            if anchor.origin != Origin::Workspace {
                continue;
            }
            let key = (pattern.owner.clone(), anchor.module.clone());
            groups.entry(key).or_default().push((anchor, PatternId(i)));
        }
        if groups.is_empty() {
            return;
        }
        let modules: HashSet<&ModulePath> = groups.keys().map(|(_, module)| module).collect();
        let mut members: HashMap<&ModulePath, Vec<&Entity>> = HashMap::new();
        for entity in graph.entities() {
            if entity.origin == Origin::Workspace && modules.contains(&entity.module) {
                members.entry(&entity.module).or_default().push(entity);
            }
        }
        let mut claims: BTreeMap<ItemPath, Vec<Assignment>> = BTreeMap::new();
        for ((component, module), anchors) in &groups {
            let eligible: HashSet<&ItemPath> = members
                .get(module)
                .into_iter()
                .flatten()
                .filter(|h| self.is_eligible(model, &h.path, component))
                .map(|h| &h.path)
                .collect();
            for (helper, anchor, pattern) in self.followers(graph, anchors, eligible) {
                claims.entry(helper.clone()).or_default().push(Assignment {
                    component: component.clone(),
                    reason: Reason::Follows {
                        anchor: anchor.clone(),
                        pattern,
                    },
                });
            }
        }
        for (path, mut claims) in claims {
            let placement = if claims.len() == 1 {
                Placement::Owned(claims.remove(0))
            } else {
                Placement::Conflict(claims)
            };
            self.placements.insert(path, placement);
        }
    }

    /// A helper candidate for `component`: mapped to another component by a module pattern (an
    /// item pattern is an explicit choice that following never overrides).
    fn is_eligible(&self, model: &Model, path: &ItemPath, component: &ComponentId) -> bool {
        let Some(Placement::Owned(Assignment {
            component: owner,
            reason: Reason::Pattern(pattern),
        })) = self.placements.get(path)
        else {
            return false;
        };
        let by_module = match model.pattern(*pattern).selector {
            Selector::Module(_) => true,
            Selector::Item(_) | Selector::Crate(_) => false,
        };
        by_module && owner != component
    }

    /// The helpers that follow `anchors`, each with the anchor that reached it first.
    ///
    /// Starts from everything in `eligible` reachable from the anchors, then repeatedly drops
    /// helpers referenced from their original component by something outside the moving set (and
    /// whatever that disconnects) until nothing changes.
    fn followers<'g>(
        &self,
        graph: &'g Graph,
        anchors: &[(&'g Entity, PatternId)],
        mut candidates: HashSet<&'g ItemPath>,
    ) -> Vec<(&'g ItemPath, &'g ItemPath, PatternId)> {
        let anchor_paths: HashSet<&ItemPath> = anchors.iter().map(|(a, _)| &a.path).collect();
        loop {
            let reached = reach(graph, anchors, &candidates);
            candidates.retain(|h| reached.contains_key(h));
            let shared: Vec<&ItemPath> = candidates
                .iter()
                .copied()
                .filter(|&h| {
                    let original = self.component_of(h);
                    graph.edges_to(h).any(|e| {
                        let src = &e.edge.src;
                        src != h
                            && !candidates.contains(src)
                            && !anchor_paths.contains(src)
                            && self.component_of(src) == original
                    })
                })
                .collect();
            if shared.is_empty() {
                let mut followers: Vec<_> = candidates
                    .iter()
                    .map(|&h| {
                        let (anchor, pattern) = reached[h];
                        (h, anchor, pattern)
                    })
                    .collect();
                followers.sort();
                return followers;
            }
            for h in shared {
                candidates.remove(h);
            }
        }
    }
}

/// Breadth-first from the anchors through `allowed`: along any edge forwards, and from a type to
/// the impl blocks for it (`impl_for_type` backwards). Maps each reached path to the anchor (and
/// its pattern) it was first reached from.
fn reach<'g>(
    graph: &'g Graph,
    anchors: &[(&'g Entity, PatternId)],
    allowed: &HashSet<&'g ItemPath>,
) -> HashMap<&'g ItemPath, (&'g ItemPath, PatternId)> {
    let mut reached = HashMap::new();
    let mut queue: VecDeque<(&ItemPath, &ItemPath, PatternId)> = anchors
        .iter()
        .map(|(anchor, pattern)| (&anchor.path, &anchor.path, *pattern))
        .collect();
    let mut seen: HashSet<&ItemPath> = queue.iter().map(|(path, _, _)| *path).collect();
    while let Some((at, anchor, pattern)) = queue.pop_front() {
        let forward = graph.edges_from(at).map(|e| &e.edge.dst);
        let impls = graph
            .edges_to(at)
            .filter(|e| e.edge.kind == EdgeKind::ImplForType)
            .filter(|e| {
                graph
                    .entity(&e.edge.src)
                    .is_some_and(|s| s.kind == EntityKind::Impl)
            })
            .map(|e| &e.edge.src);
        for next in forward.chain(impls) {
            if allowed.contains(next) && seen.insert(next) {
                reached.insert(next, (anchor, pattern));
                queue.push_back((next, anchor, pattern));
            }
        }
    }
    reached
}

/// The patterns of one [`ComponentKind`], indexed for lookup.
struct Matcher {
    items: HashMap<ItemPath, PatternId>,
    exact_modules: HashMap<ModulePath, PatternId>,
    descendant_modules: HashMap<ModulePath, PatternId>,
    exact_crates: HashMap<CrateName, PatternId>,
    /// Longest prefix first.
    crate_prefixes: Vec<(String, PatternId)>,
}

impl Matcher {
    fn new(model: &Model, kind: ComponentKind) -> Self {
        let mut matcher = Self {
            items: HashMap::new(),
            exact_modules: HashMap::new(),
            descendant_modules: HashMap::new(),
            exact_crates: HashMap::new(),
            crate_prefixes: Vec::new(),
        };
        for (i, pattern) in model.patterns().iter().enumerate() {
            let owner_kind = model.component(&pattern.owner).map(|c| c.kind);
            if owner_kind != Some(kind) {
                continue;
            }
            let id = PatternId(i);
            match &pattern.selector {
                Selector::Item(path) => {
                    matcher.items.insert(path.clone(), id);
                }
                Selector::Module(module) => {
                    let index = match module.scope {
                        ModuleScope::Exact => &mut matcher.exact_modules,
                        ModuleScope::Descendants => &mut matcher.descendant_modules,
                    };
                    index.insert(module.module.clone(), id);
                }
                Selector::Crate(CratePattern::Exact(name)) => {
                    matcher.exact_crates.insert(name.clone(), id);
                }
                Selector::Crate(CratePattern::Prefix(prefix)) => {
                    matcher.crate_prefixes.push((prefix.clone(), id));
                }
            }
        }
        matcher
            .crate_prefixes
            .sort_by(|(a, _), (b, _)| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        matcher
    }

    /// The most specific pattern matching `entity`: an item pattern; else the exact module; else
    /// the `::**` pattern of the nearest ancestor module; else the exact crate; else the longest
    /// crate prefix. Patterns are unique (the parser rejects duplicates), so there is never a tie.
    fn best(&self, entity: &Entity) -> Option<PatternId> {
        if let Some(&id) = self.items.get(&entity.path) {
            return Some(id);
        }
        if let Some(&id) = self.exact_modules.get(&entity.module) {
            return Some(id);
        }
        let module = entity.module.as_str();
        let ancestors = module.rmatch_indices("::").map(|(i, _)| &module[..i]);
        for ancestor in ancestors {
            if let Some(&id) = self.descendant_modules.get(&ModulePath::new(ancestor)) {
                return Some(id);
            }
        }
        if let Some(&id) = self.exact_crates.get(&entity.crate_name) {
            return Some(id);
        }
        self.crate_prefixes
            .iter()
            .find(|(prefix, _)| entity.crate_name.as_str().starts_with(prefix.as_str()))
            .map(|&(_, id)| id)
    }
}
