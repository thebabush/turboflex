//! Item naming, module attribution and folding of anonymous bodies into named items.
//!
//! # Path rules
//!
//! An [`ItemPath`] must identify exactly one item across every unit of an analysis (see its docs
//! in `turboflex-graph`). Paths are crate-qualified and lifetime-free.
//!
//! Every unit must spell an item the same way, but rustc's *visible* path of an item (its
//! shortest re-export, `serde::Serialize` for `serde_core::ser::Serialize`) depends on which
//! crates the unit depends on. So the rules use three spellings:
//!
//! - *definition path*: crate name plus the named segments of the item's definition path;
//! - *canonical path*: the shortest public path to the item *within its own crate*
//!   (`core::ops::Deref` for `core::ops::deref::Deref`, `alloc::string::String`), which depends
//!   only on that crate's metadata; the definition path for workspace items. rustc cannot print
//!   these (its printer only follows re-exports from crates the unit depends on directly), so
//!   types are printed with definition paths and then rewritten item by item;
//! - *visible path*: rustc's `visible_parent_map` path, minus re-exports through workspace
//!   crates; the one a user writes: `sqlx::query`, `std::fs::read`.
//!
//! **Workspace items** (the crate being compiled and the other workspace crates it depends on)
//! are named by their definition path, never by a re-export. Types and traits inside `<..>` and
//! `{impl ..}` are canonical paths: `<app::money::Money as core::fmt::Display>::fmt`.
//!
//! - Modules, free fns, types, traits, consts, statics: `app::util::helper`.
//! - Macros get a `!`: `app::macros_m::make!`. A derive macro shares its path with its trait
//!   (`serde::Serialize`), and a `macro_rules!` may share one with a fn.
//! - A fn, const or static whose module also defines a module, type or trait of the same name
//!   (different namespaces, so legal Rust) is written `app::{fn parse}`, `app::{const X}`,
//!   `app::{static X}`.
//! - An anonymous `const _` on the way to an item prints as `_#N`, N being rustc's disambiguator
//!   (helper items that macros put in `const _: () = { ... };` blocks would collide otherwise).
//! - Impl blocks are named by the module they are written in: `app::impls_c::{impl
//!   app::traits_b::Describe for app::model_a::Widget}`, `app::split_ext::{impl app::split::Thing}`.
//!   When two inherent impl blocks of one type in one module print the same, each gets ` #k`
//!   (`{impl app::m::Foo #1}`, `{impl app::m::Foo #2}`; k counts them in definition order).
//! - Items of a trait impl: `<app::animals::Dog as core_api::Greeter>::greet`. Coherence allows
//!   one impl per (self type, trait with its arguments), and both are in the name.
//! - Items of an inherent impl: `app::shapes::Circle::area`. Rust rejects two inherent items of
//!   one name for overlapping self types, but `impl Foo<u32>` and `impl Foo<u64>` may both define
//!   `new`; when another inherent impl of the type defines an item of the same name, the self
//!   type is spelled out: `<app::Foo<u32>>::new`. Non-ADT self types always are: `<dyn
//!   app::Tr>::m`, `<[T]>::len` (primitives stay bare: `u32::pow`).
//! - Items declared in a trait: `core_api::Greeter::greet`.
//!
//! **External items** (std/core/alloc and dependencies) whose name is a plain path are named by
//! their visible path: `std::fs::read`, `sqlx::query`, `reqwest::Client::new`,
//! `std::io::Read::read`, `std::println!`. Their module is the visible module (for methods: of
//! the type or trait), e.g. `sqlx`, `std::fs`, so a model can name external components by path
//! prefix. A unit that does not depend on `sqlx` directly may spell the same item
//! `sqlx_core::query::query`; that only splits an external entity in two, never a workspace one.
//! External items whose name needs `<..>` (trait impl items, methods of non-ADT or disambiguated
//! self types) and external impl blocks are spelled canonically, and their module is the
//! definition path of the module they are written in, so units never disagree on the module of
//! a path: `<alloc::string::String as core::clone::Clone>::clone` in `alloc::string`.
//!
//! Remaining ambiguities, caught as in-unit collisions (see `Extractor`): an associated type and
//! an associated fn/const of the same name in one trait; trait impls whose headers differ only in
//! lifetimes; types defined inside an anonymous `const _` printed inside `<..>` (rustc prints
//! the `const _` as `_`); external items whose visible paths coincide.
//!
//! # Folding
//!
//! Edges and entities are attributed to named items: closures, coroutines (async fn/blocks),
//! inline consts and every item defined inside a body (local impls, `tokio::select!`'s helper
//! module, serde's `__Visitor`, helper fns a macro writes into a `const` initializer) fold into
//! the outermost enclosing body owner: a fn, or a named const or static. An anonymous `const _`
//! is not one: macros use it to scope module-level items (serde's `impl Serialize`), which stay
//! entities of their own, while the `const _` itself folds into its module. Variants, fields and
//! constructors fold into their type; `use`, `extern crate`, `extern {}` blocks and
//! `global_asm!` into their module.

use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt::Write as _;
use std::num::NonZeroU32;

use rustc_hir::def::DefKind;
use rustc_hir::def_id::{CrateNum, DefId, LOCAL_CRATE};
use rustc_middle::ty::print::{
    with_no_trimmed_paths, with_no_visible_paths, with_resolve_crate_name,
};
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::{FileName, Symbol, kw};
use turboflex_graph::{CrateName, EntityKind, ItemPath, ModulePath, Origin, Span};

pub(crate) struct Names<'tcx> {
    tcx: TyCtxt<'tcx>,
    workspace: BTreeSet<CrateNum>,
    /// Canonical parents of external items (see [`canonical_parents`]).
    canonical: HashMap<DefId, DefId>,
}

/// For every item of a non-workspace crate, its parent on the shortest public path from its own
/// crate's root through items of that crate only. Like rustc's `visible_parent_map`, a parent
/// that re-exports the item as `_` or is `#[doc(hidden)]` is used only if there is no other.
fn canonical_parents(tcx: TyCtxt<'_>, workspace: &BTreeSet<CrateNum>) -> HashMap<DefId, DefId> {
    let mut parents = HashMap::new();
    let mut fallback = HashMap::new();
    for &cnum in tcx.crates(()) {
        if workspace.contains(&cnum) {
            continue;
        }
        let mut queue = VecDeque::from([cnum.as_def_id()]);
        while let Some(parent) = queue.pop_front() {
            for child in tcx.module_children(parent) {
                let Some(def_id) = child.res.opt_def_id() else {
                    continue;
                };
                if !child.vis.is_public() || def_id.krate != cnum || parents.contains_key(&def_id) {
                    continue;
                }
                let hidden = child.ident.name == kw::Underscore
                    || tcx.is_doc_hidden(parent)
                    || child
                        .reexport_chain
                        .first()
                        .and_then(|r| r.id())
                        .is_some_and(|id| tcx.is_doc_hidden(id));
                let seen = fallback.contains_key(&def_id);
                if !hidden {
                    parents.insert(def_id, parent);
                } else if let Entry::Vacant(slot) = fallback.entry(def_id) {
                    slot.insert(parent);
                }
                if !seen && child.res.module_like_def_id().is_some() {
                    queue.push_back(def_id);
                }
            }
        }
    }
    for (child, parent) in fallback {
        parents.entry(child).or_insert(parent);
    }
    parents
}

/// Which parent map a path walk follows.
#[derive(Clone, Copy)]
enum Parents {
    /// rustc's `visible_parent_map`, minus re-exports through workspace crates.
    Visible,
    /// [`canonical_parents`].
    Canonical,
}

/// The namespace-clash marker of a value item (see the module docs).
#[derive(Clone, Copy)]
enum ValueItem {
    Fn,
    Const,
    Static,
}

impl ValueItem {
    fn keyword(self) -> &'static str {
        match self {
            Self::Fn => "fn",
            Self::Const => "const",
            Self::Static => "static",
        }
    }
}

/// How a path is spelled (see the module docs); an entity's module follows its path.
#[derive(Clone, Copy)]
enum Spelling {
    /// The visible path of an external item.
    Visible,
    /// A definition path, or `<..>` / `{impl ..}` with canonical paths inside.
    Definition,
}

impl<'tcx> Names<'tcx> {
    pub(crate) fn new(tcx: TyCtxt<'tcx>, workspace: &BTreeSet<CrateName>) -> Self {
        let workspace: BTreeSet<CrateNum> = tcx
            .crates(())
            .iter()
            .copied()
            .filter(|&cnum| workspace.contains(&CrateName::new(tcx.crate_name(cnum).as_str())))
            .chain([LOCAL_CRATE])
            .collect();
        let canonical = canonical_parents(tcx, &workspace);
        Names {
            tcx,
            workspace,
            canonical,
        }
    }

    pub(crate) fn in_workspace(&self, def_id: DefId) -> bool {
        self.workspace.contains(&def_id.krate)
    }

    pub(crate) fn crate_name(&self, def_id: DefId) -> CrateName {
        CrateName::new(self.tcx.crate_name(def_id.krate).as_str())
    }

    /// The named item that owns `def_id`, per the folding rules in the module docs. The result
    /// always has an [`entity_kind`](Self::entity_kind).
    pub(crate) fn fold(&self, def_id: DefId) -> DefId {
        let tcx = self.tcx;
        let mut outermost_body = None;
        let mut cur = def_id;
        loop {
            if self.is_body_owner(cur) {
                outermost_body = Some(cur);
            }
            match tcx.opt_parent(cur) {
                Some(p) => cur = p,
                None => break,
            }
        }
        let mut root = tcx.typeck_root_def_id(outermost_body.unwrap_or(def_id));
        while self.entity_kind(root).is_none() {
            match tcx.opt_parent(root) {
                Some(p) => root = tcx.typeck_root_def_id(p),
                None => break,
            }
        }
        root
    }

    /// A named item with a body that items defined inside it fold into (see the module docs).
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "rustc's DefKind: only fns, closures, consts and statics own bodies with items"
    )]
    fn is_body_owner(&self, def_id: DefId) -> bool {
        match self.tcx.def_kind(def_id) {
            DefKind::Fn | DefKind::AssocFn | DefKind::Closure | DefKind::SyntheticCoroutineBody => {
                true
            }
            DefKind::Const | DefKind::AssocConst | DefKind::Static { .. } => {
                self.tcx.opt_item_name(def_id) != Some(kw::Underscore)
            }
            _ => false,
        }
    }

    /// What an item is, or `None` for definitions that are not entities (they fold into one).
    pub(crate) fn entity_kind(&self, def_id: DefId) -> Option<EntityKind> {
        let tcx = self.tcx;
        let kind = match tcx.def_kind(def_id) {
            DefKind::Mod => EntityKind::Module,
            // An `extern { type T; }` is an opaque nominal type; `Struct` is the closest kind.
            DefKind::Struct | DefKind::ForeignTy => EntityKind::Struct,
            DefKind::Union => EntityKind::Union,
            DefKind::Enum => EntityKind::Enum,
            DefKind::Trait | DefKind::TraitAlias => EntityKind::Trait,
            DefKind::TyAlias => EntityKind::TypeAlias,
            DefKind::Fn => EntityKind::Function,
            // `const _: () = { ... };` is macro plumbing: it folds into its module, the items
            // inside it stay entities of their own.
            DefKind::Const if tcx.opt_item_name(def_id) == Some(kw::Underscore) => return None,
            DefKind::Const => EntityKind::Const,
            DefKind::Static { nested: false, .. } => EntityKind::Static,
            DefKind::AssocTy => EntityKind::AssocType,
            DefKind::AssocConst => EntityKind::AssocConst,
            DefKind::AssocFn if tcx.def_kind(tcx.parent(def_id)) == DefKind::Trait => {
                EntityKind::TraitMethod
            }
            DefKind::AssocFn => EntityKind::Method,
            DefKind::Macro(_) => EntityKind::Macro,
            DefKind::Impl { .. } => EntityKind::Impl,
            DefKind::Static { nested: true, .. }
            | DefKind::Variant
            | DefKind::Ctor(..)
            | DefKind::Field
            | DefKind::TyParam
            | DefKind::ConstParam
            | DefKind::LifetimeParam
            | DefKind::ExternCrate
            | DefKind::Use
            | DefKind::ForeignMod
            | DefKind::AnonConst
            | DefKind::OpaqueTy
            | DefKind::GlobalAsm
            | DefKind::Closure
            | DefKind::SyntheticCoroutineBody
            | DefKind::TestBinderConstraints => return None,
        };
        Some(kind)
    }

    pub(crate) fn origin(&self, def_id: DefId) -> Origin {
        if self.in_workspace(def_id) {
            Origin::Workspace
        } else {
            Origin::External
        }
    }

    /// Definition site of `def_id`, when rustc has a real one.
    pub(crate) fn def_span(&self, def_id: DefId) -> Option<Span> {
        self.span(self.tcx.def_span(def_id))
    }

    /// `span` as file and line; `None` for dummy spans and non-file sources.
    pub(crate) fn span(&self, span: rustc_span::Span) -> Option<Span> {
        if span.is_dummy() {
            return None;
        }
        let sm = self.tcx.sess.source_map();
        let loc = sm.lookup_char_pos(span.lo());
        if !matches!(loc.file.name, FileName::Real(_)) {
            return None;
        }
        Some(Span {
            file: sm.filename_for_diagnostics(&loc.file.name).to_string(),
            line: NonZeroU32::new(u32::try_from(loc.line).ok()?)?,
        })
    }

    // ---- paths and modules -------------------------------------------------------------------

    /// The unique path of a folded item and the module it belongs to (see the module docs). The
    /// module of a workspace item is where it is written (for impl items: where the impl block
    /// is written); for external items it depends on how the path is spelled.
    pub(crate) fn naming(&self, def_id: DefId) -> (ItemPath, ModulePath) {
        let (path, spelling) = self.item_path(def_id);
        let module = match (self.in_workspace(def_id), spelling) {
            (true, _) | (false, Spelling::Definition) => {
                self.def_path(self.enclosing_module(def_id))
            }
            (false, Spelling::Visible) => self.visible_module(def_id),
        };
        (ItemPath::new(path), ModulePath::new(module))
    }

    /// `def_id` itself if it is a module, else its nearest module ancestor.
    fn enclosing_module(&self, def_id: DefId) -> DefId {
        let tcx = self.tcx;
        let mut cur = def_id;
        while tcx.def_kind(cur) != DefKind::Mod {
            match tcx.opt_parent(cur) {
                Some(p) => cur = p,
                None => break,
            }
        }
        cur
    }

    /// The visible module of an external item; of its type or trait for an associated item.
    fn visible_module(&self, def_id: DefId) -> String {
        let tcx = self.tcx;
        let mut anchor = def_id;
        if matches!(
            tcx.def_kind(anchor),
            DefKind::AssocFn | DefKind::AssocConst | DefKind::AssocTy
        ) {
            anchor = tcx.parent(anchor);
            if let Some(adt) = self.impl_adt(anchor) {
                anchor = adt;
            }
        }
        let mut cur = anchor;
        while tcx.def_kind(cur) != DefKind::Mod {
            match self
                .parent(Parents::Visible, cur)
                .or_else(|| tcx.opt_parent(cur))
            {
                Some(p) => cur = p,
                None => break,
            }
        }
        self.walk_path(Parents::Visible, cur)
    }

    /// The ADT an impl is for, if `impl_id` is an impl of one.
    fn impl_adt(&self, impl_id: DefId) -> Option<DefId> {
        if !matches!(self.tcx.def_kind(impl_id), DefKind::Impl { .. }) {
            return None;
        }
        self.impl_self_ty(impl_id).ty_adt_def().map(ty::AdtDef::did)
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "rustc's DefKind: every other entity kind is named by its plain path"
    )]
    fn item_path(&self, def_id: DefId) -> (String, Spelling) {
        let tcx = self.tcx;
        match tcx.def_kind(def_id) {
            DefKind::Impl { .. } => (self.impl_path(def_id), Spelling::Definition),
            DefKind::AssocFn | DefKind::AssocConst | DefKind::AssocTy => self.assoc_path(def_id),
            DefKind::Macro(_) => (
                format!("{}!", self.base_path(def_id)),
                self.base_spelling(def_id),
            ),
            DefKind::Fn => self.value_item_path(def_id, ValueItem::Fn),
            DefKind::Const => self.value_item_path(def_id, ValueItem::Const),
            DefKind::Static { .. } => self.value_item_path(def_id, ValueItem::Static),
            _ => (self.base_path(def_id), self.base_spelling(def_id)),
        }
    }

    fn base_spelling(&self, def_id: DefId) -> Spelling {
        if self.in_workspace(def_id) {
            Spelling::Definition
        } else {
            Spelling::Visible
        }
    }

    /// Definition path for workspace items, visible path for external ones.
    fn base_path(&self, def_id: DefId) -> String {
        if self.in_workspace(def_id) {
            self.def_path(def_id)
        } else {
            self.walk_path(Parents::Visible, def_id)
        }
    }

    /// Crate-qualified definition path; anonymous `const _` segments print as `_#N`.
    fn def_path(&self, def_id: DefId) -> String {
        let mut path = self.tcx.crate_name(def_id.krate).to_string();
        for elem in self.tcx.def_path(def_id).data {
            match elem.data.get_opt_name() {
                Some(name) if name == kw::Underscore => {
                    let _ = write!(path, "::_#{}", elem.disambiguator);
                }
                Some(name) => {
                    let _ = write!(path, "::{name}");
                }
                None => {}
            }
        }
        path
    }

    fn value_item_path(&self, def_id: DefId, item: ValueItem) -> (String, Spelling) {
        if !self.in_workspace(def_id) {
            return (self.walk_path(Parents::Visible, def_id), Spelling::Visible);
        }
        let tcx = self.tcx;
        let name = tcx.item_name(def_id);
        let parent = tcx.parent(def_id);
        let parent = if tcx.def_kind(parent) == DefKind::ForeignMod {
            tcx.parent(parent)
        } else {
            parent
        };
        let path = if tcx.def_kind(parent) == DefKind::Mod && self.module_defines_type(parent, name)
        {
            format!("{}::{{{} {name}}}", self.def_path(parent), item.keyword())
        } else {
            self.def_path(def_id)
        };
        (path, Spelling::Definition)
    }

    /// Whether `module` itself defines a type-namespace item (module, type, trait) named `name`.
    fn module_defines_type(&self, module: DefId, name: Symbol) -> bool {
        let tcx = self.tcx;
        let children = match module.as_local() {
            Some(local) => tcx.module_children_local(local),
            None => tcx.module_children(module),
        };
        children.iter().any(|child| {
            child.ident.name == name
                && child.reexport_chain.is_empty()
                && child.res.opt_def_id().is_some_and(|d| {
                    matches!(
                        tcx.def_kind(d),
                        DefKind::Mod
                            | DefKind::Struct
                            | DefKind::Union
                            | DefKind::Enum
                            | DefKind::Trait
                            | DefKind::TraitAlias
                            | DefKind::TyAlias
                            | DefKind::ForeignTy
                    )
                })
        })
    }

    fn impl_path(&self, impl_id: DefId) -> String {
        let module = self.def_path(self.enclosing_module(impl_id));
        let header = self.impl_header(impl_id);
        match self.inherent_impl_rank(impl_id, &header) {
            Some(rank) => format!("{module}::{{{header} #{rank}}}"),
            None => format!("{module}::{{{header}}}"),
        }
    }

    /// `impl core::fmt::Display for app::money::Money` / `impl app::split::Thing`.
    fn impl_header(&self, impl_id: DefId) -> String {
        let self_ty = self.ty_str(self.impl_self_ty(impl_id));
        match self.impl_trait_ref(impl_id) {
            Some(tr) => format!("impl {} for {self_ty}", self.trait_ref_str(tr)),
            None => format!("impl {self_ty}"),
        }
    }

    /// For an inherent impl whose header prints like another inherent impl of the same type in
    /// the same module: its 1-based position among those, in definition order.
    fn inherent_impl_rank(&self, impl_id: DefId, header: &str) -> Option<usize> {
        let tcx = self.tcx;
        if self.impl_trait_ref(impl_id).is_some() {
            return None;
        }
        let ty::Adt(adt, _) = self.impl_self_ty(impl_id).kind() else {
            return None;
        };
        let module = self.enclosing_module(impl_id);
        let mut twins: Vec<DefId> = tcx
            .inherent_impls(adt.did())
            .iter()
            .copied()
            .filter(|&other| {
                self.enclosing_module(other) == module && self.impl_header(other) == header
            })
            .collect();
        if twins.len() < 2 {
            return None;
        }
        twins.sort_by_key(|d| d.index);
        twins.iter().position(|&d| d == impl_id).map(|i| i + 1)
    }

    fn assoc_path(&self, def_id: DefId) -> (String, Spelling) {
        let tcx = self.tcx;
        let name = tcx.item_name(def_id);
        let parent = tcx.parent(def_id);
        let DefKind::Impl { .. } = tcx.def_kind(parent) else {
            let path = format!("{}::{name}", self.base_path(parent));
            return (path, self.base_spelling(parent));
        };
        let self_ty = self.impl_self_ty(parent);
        if let Some(tr) = self.impl_trait_ref(parent) {
            let path = format!(
                "<{} as {}>::{name}",
                self.ty_str(self_ty),
                self.trait_ref_str(tr)
            );
            return (path, Spelling::Definition);
        }
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "rustc's TyKind: every other self type is spelled out"
        )]
        match self_ty.kind() {
            ty::Adt(adt, _) if !self.inherent_name_shared(adt.did(), parent, name) => (
                format!("{}::{name}", self.base_path(adt.did())),
                self.base_spelling(adt.did()),
            ),
            ty::Bool | ty::Char | ty::Int(_) | ty::Uint(_) | ty::Float(_) | ty::Str => (
                format!("{}::{name}", self.ty_str(self_ty)),
                Spelling::Definition,
            ),
            _ => (
                format!("<{}>::{name}", self.ty_str(self_ty)),
                Spelling::Definition,
            ),
        }
    }

    /// Whether an inherent impl of `adt` other than `impl_id` also defines an item `name`.
    fn inherent_name_shared(&self, adt: DefId, impl_id: DefId, name: Symbol) -> bool {
        let tcx = self.tcx;
        tcx.inherent_impls(adt).iter().any(|&other| {
            other != impl_id
                && tcx
                    .associated_item_def_ids(other)
                    .iter()
                    .any(|&item| tcx.opt_item_name(item) == Some(name))
        })
    }

    // ---- printing ----------------------------------------------------------------------------

    fn parent(&self, parents: Parents, def_id: DefId) -> Option<DefId> {
        match parents {
            Parents::Visible => self
                .tcx
                .visible_parent_map(())
                .get(&def_id)
                .copied()
                // Keeps an external item re-exported by a workspace crate (`pub use
                // serde_json::Value`) under its own crate's path.
                .filter(|parent| !self.workspace.contains(&parent.krate)),
            Parents::Canonical => self.canonical.get(&def_id).copied(),
        }
    }

    /// A path up `parents` to a crate root, naming each step as its parent exposes it (a
    /// re-export may rename). From an item without a parent on, the definition path is used.
    fn walk_path(&self, parents: Parents, def_id: DefId) -> String {
        let tcx = self.tcx;
        let mut segments = Vec::new();
        let mut cur = def_id;
        loop {
            if cur.is_crate_root() {
                segments.push(tcx.crate_name(cur.krate).to_string());
                break;
            }
            let Some(parent) = self.parent(parents, cur) else {
                segments.push(self.def_path(cur));
                break;
            };
            let exposed = tcx
                .module_children(parent)
                .iter()
                .find(|c| c.res.opt_def_id() == Some(cur) && c.ident.name != kw::Underscore)
                .map(|c| c.ident.name)
                .or_else(|| tcx.opt_item_name(cur));
            segments.push(exposed.map_or_else(String::new, |name| name.to_string()));
            cur = parent;
        }
        segments.reverse();
        segments.join("::")
    }

    /// The canonical path of an item: [`Parents::Canonical`] for external items, the definition
    /// path for workspace ones.
    fn canonical_path(&self, def_id: DefId) -> String {
        if self.in_workspace(def_id) {
            self.def_path(def_id)
        } else {
            self.walk_path(Parents::Canonical, def_id)
        }
    }

    /// A type with canonical paths throughout. rustc prints it with definition paths; those of
    /// the external items in it are then replaced.
    fn ty_str(&self, ty: Ty<'tcx>) -> String {
        let mut s = with_no_visible_paths!(with_resolve_crate_name!(with_no_trimmed_paths!(
            ty.to_string()
        )));
        let mut items: Vec<(String, String)> = self
            .items_in(ty)
            .into_iter()
            .filter(|&def_id| !self.in_workspace(def_id))
            .map(|def_id| {
                let printed = with_no_visible_paths!(with_resolve_crate_name!(
                    with_no_trimmed_paths!(self.tcx.def_path_str(def_id))
                ));
                (printed, self.canonical_path(def_id))
            })
            .collect();
        // Longest first, so `Iterator::Item` is rewritten before `Iterator`.
        items.sort_by_key(|(printed, _)| std::cmp::Reverse(printed.len()));
        for (printed, canonical) in items {
            if printed != canonical {
                s = replace_path(&s, &printed, &canonical);
            }
        }
        strip_lifetimes(&s)
    }

    /// The items whose paths rustc prints when printing `ty`.
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "rustc's TyKind: only the listed variants print item paths"
    )]
    fn items_in(&self, ty: Ty<'tcx>) -> Vec<DefId> {
        let tcx = self.tcx;
        let mut items = Vec::new();
        for arg in ty.walk() {
            let Some(t) = arg.as_type() else {
                continue;
            };
            match *t.kind() {
                ty::Adt(adt, _) => items.push(adt.did()),
                ty::Foreign(def_id) | ty::FnDef(def_id, _) => items.push(def_id),
                ty::Dynamic(preds, ..) => {
                    for pred in preds {
                        match pred.skip_binder() {
                            ty::ExistentialPredicate::Trait(tr) => items.push(tr.def_id),
                            ty::ExistentialPredicate::Projection(p) => {
                                items.extend([p.def_id, tcx.parent(p.def_id)]);
                            }
                            ty::ExistentialPredicate::AutoTrait(def_id) => items.push(def_id),
                        }
                    }
                }
                ty::Alias(_, alias) => match alias.kind {
                    ty::AliasTyKind::Projection { def_id } => {
                        items.extend([def_id, tcx.parent(def_id)]);
                    }
                    ty::AliasTyKind::Inherent { def_id } | ty::AliasTyKind::Free { def_id } => {
                        items.push(def_id);
                    }
                    // Printed as `impl Trait`; its bounds keep their definition paths.
                    ty::AliasTyKind::Opaque { .. } => {}
                },
                _ => {}
            }
        }
        items
    }

    /// `core::convert::From<app::errors_a::ErrA>` for a trait ref (without its `Self`), with
    /// canonical paths throughout.
    fn trait_ref_str(&self, trait_ref: ty::TraitRef<'tcx>) -> String {
        let mut s = self.canonical_path(trait_ref.def_id);
        let args: Vec<String> = trait_ref
            .args
            .types()
            .skip(1)
            .map(|t| self.ty_str(t))
            .collect();
        if !args.is_empty() {
            s.push('<');
            s.push_str(&args.join(", "));
            s.push('>');
        }
        s
    }

    fn impl_self_ty(&self, impl_id: DefId) -> Ty<'tcx> {
        self.tcx
            .type_of(impl_id)
            .instantiate_identity()
            .skip_normalization()
    }

    fn impl_trait_ref(&self, impl_id: DefId) -> Option<ty::TraitRef<'tcx>> {
        (self.tcx.def_kind(impl_id) == (DefKind::Impl { of_trait: true })).then(|| {
            self.tcx
                .impl_trait_ref(impl_id)
                .instantiate_identity()
                .skip_normalization()
        })
    }
}

/// Replaces every occurrence of path `from` in `s` that is not part of a longer path.
fn replace_path(s: &str, from: &str, to: &str) -> String {
    let is_path_char = |c: char| c.is_alphanumeric() || c == '_' || c == ':';
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find(from) {
        let before = rest[..pos].chars().next_back();
        let after = rest[pos + from.len()..].chars().next();
        let whole = !before.is_some_and(is_path_char)
            && !after.is_some_and(|c| c.is_alphanumeric() || c == '_');
        out.push_str(&rest[..pos]);
        out.push_str(if whole { to } else { from });
        rest = &rest[pos + from.len()..];
    }
    out.push_str(rest);
    out
}

/// Remove lifetimes from a printed type/path: `Decode<'r, Sqlite>` -> `Decode<Sqlite>`,
/// `&'a T` -> `&T`, `Foo<'_>` -> `Foo`.
fn strip_lifetimes(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\''
            && chars
                .get(i + 1)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
        {
            i += 1;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            if chars.get(i) == Some(&',') {
                i += 1;
            }
            while chars.get(i) == Some(&' ') {
                i += 1;
            }
            // A lifetime bound of a trait object or `impl Trait`: drop its `+` too, whichever
            // side it is on (`dyn Any + 'static`, `dyn 'a + Any`).
            if out.ends_with(" + ") {
                out.truncate(out.len() - " + ".len());
            } else if chars.get(i) == Some(&'+') {
                i += 1;
                while chars.get(i) == Some(&' ') {
                    i += 1;
                }
            }
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out.replace(", >", ">").replace(",>", ">").replace("<>", "")
}

#[cfg(test)]
mod tests {
    use super::{replace_path, strip_lifetimes};

    #[test]
    fn strips_lifetime_args_and_refs() {
        assert_eq!(strip_lifetimes("Decode<'r, Sqlite>"), "Decode<Sqlite>");
        assert_eq!(strip_lifetimes("&'a T"), "&T");
        assert_eq!(strip_lifetimes("Foo<'_>"), "Foo");
        assert_eq!(strip_lifetimes("Foo<'a, 'b>"), "Foo");
    }

    #[test]
    fn strips_lifetime_bounds_with_their_plus() {
        assert_eq!(
            strip_lifetimes("(dyn core::any::Any + 'static)"),
            "(dyn core::any::Any)"
        );
        assert_eq!(
            strip_lifetimes("Box<dyn Error + Send + 'static>"),
            "Box<dyn Error + Send>"
        );
        assert_eq!(strip_lifetimes("dyn 'a + Any"), "dyn Any");
        assert_eq!(strip_lifetimes("impl Fn() + 'a"), "impl Fn()");
    }

    #[test]
    fn replaces_whole_paths_only() {
        assert_eq!(
            replace_path("a::B<a::Bc, x::a::B>", "a::B", "c::B"),
            "c::B<a::Bc, x::a::B>"
        );
        assert_eq!(
            replace_path("<a::B as T>::m", "a::B", "c::B"),
            "<c::B as T>::m"
        );
    }
}
