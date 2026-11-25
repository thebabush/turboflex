//! Fact extraction from one compiled crate: item signatures/headers/predicates (HIR-level
//! queries) plus runtime MIR bodies (calls, constructions, discriminant matches, drops, globals,
//! local types), with macro/desugaring provenance taken from span expansion data.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use rustc_hir::attrs::lang_items::LangItem;
use rustc_hir::def::{CtorOf, DefKind};
use rustc_hir::def_id::{CRATE_DEF_ID, DefId, LocalDefId};
use rustc_hir::find_attr;
use rustc_infer::infer::TyCtxtInferExt;
use rustc_middle::mir::interpret::{GlobalAlloc, Scalar};
use rustc_middle::mir::visit::Visitor;
use rustc_middle::mir::{self, CallSource, ConstValue, Location, TerminatorKind};
use rustc_middle::traits::{CodegenObligationError, ImplSource};
use rustc_middle::ty::adjustment::PointerCoercion;
use rustc_middle::ty::{
    self, GenericArgsRef, Instance, InstanceKind, Ty, TyCtxt, TypeVisitableExt, TypingMode,
};
use rustc_span::hygiene::{ExpnKind, MacroKind};
use rustc_span::{Span, Symbol, SyntaxContext, sym};
use rustc_trait_selection::traits::{
    Obligation, ObligationCause, ObligationCtxt, SelectionContext, SelectionError,
};
use turboflex_graph::{CrateName, Edge, EdgeKind, Entity, ItemPath, Provenance};

use crate::names::Names;

/// How deep to follow impl where-clauses when resolving the predicates of a callee
/// (callee -> std impl -> nested obligation -> workspace impl ...).
const MAX_NESTED_DEPTH: usize = 4;
/// How deep to look into field types for workspace `Drop` impls.
const MAX_DROP_DEPTH: usize = 4;

/// The facts of one crate.
pub(crate) struct Facts {
    /// Every edge endpoint, plus every named item of the crate; sorted by path.
    pub(crate) entities: Vec<Entity>,
    /// One per `(src, dst, kind)`; sorted by that key.
    pub(crate) edges: Vec<Edge>,
    /// Items whose path collided with another item's and got a ` #<hash>` suffix.
    pub(crate) collisions: Vec<ItemPath>,
}

/// Provenance of an edge derived from a span's expansion chain, plus a detail string.
struct Prov {
    provenance: Provenance,
    detail: String,
}

/// Nature of an edge that is not a plain source-level fact.
#[derive(Clone, Copy)]
enum Implicit {
    No,
    /// Call inserted by overloaded operator / autoderef / deref coercion lowering.
    Operator,
    /// Scope-end drop glue.
    Drop,
}

/// What led to a nested-impl edge, for its detail.
#[derive(Clone, Copy)]
enum Via {
    /// The where-clauses of a called fn.
    Call(DefId),
    /// A vtable built for `dyn Trait`.
    Dyn(DefId),
}

/// `workspace`: crate names of the analysed workspace (their items are [`Origin::Workspace`]).
///
/// [`Origin::Workspace`]: turboflex_graph::Origin::Workspace
pub(crate) fn extract(tcx: TyCtxt<'_>, workspace: &BTreeSet<CrateName>) -> Facts {
    let mut x = Extractor {
        tcx,
        names: Names::new(tcx, workspace),
        entities: HashMap::new(),
        by_path: HashMap::new(),
        collisions: Vec::new(),
        edges: Vec::new(),
        index: HashMap::new(),
        ty_seen: HashSet::new(),
        macro_seen: HashSet::new(),
        variants: HashMap::new(),
        out_dir: std::env::var_os("OUT_DIR").map(PathBuf::from),
    };
    x.entity(CRATE_DEF_ID.to_def_id());
    for def in tcx.hir_crate_items(()).definitions() {
        let named = x.names.fold(def.to_def_id());
        if !x.is_harness(named) {
            x.entity(named);
        }
        x.item(def);
    }
    for owner in tcx.hir_body_owners() {
        x.body(owner);
    }
    let mut entities: Vec<Entity> = x.entities.into_values().collect();
    entities.sort_by(|a, b| a.path.cmp(&b.path));
    let mut edges = x.edges;
    for (i, variants) in x.variants {
        let variants = variants.into_iter().collect::<Vec<_>>().join(",");
        let detail = &mut edges[i].detail;
        if !detail.is_empty() {
            detail.push(';');
        }
        detail.push_str("variants=");
        detail.push_str(&variants);
    }
    edges.sort_by(|a, b| (&a.src, &a.dst, a.kind).cmp(&(&b.src, &b.dst, b.kind)));
    Facts {
        entities,
        edges,
        collisions: x.collisions,
    }
}

struct Extractor<'tcx> {
    tcx: TyCtxt<'tcx>,
    names: Names<'tcx>,
    /// Folded def -> its entity.
    entities: HashMap<DefId, Entity>,
    /// Path -> the def that holds it, to catch two items printing the same.
    by_path: HashMap<ItemPath, DefId>,
    collisions: Vec<ItemPath>,
    edges: Vec<Edge>,
    /// `(src, dst, kind)` -> position in `edges`.
    index: HashMap<(ItemPath, ItemPath, EdgeKind), usize>,
    ty_seen: HashSet<(DefId, Ty<'tcx>, EdgeKind, bool)>,
    macro_seen: HashSet<(DefId, SyntaxContext)>,
    /// Position in `edges` -> the enum variants a `constructs` edge built, for its detail.
    variants: HashMap<usize, BTreeSet<String>>,
    /// Build-script output dir of the crate being compiled (set by cargo), for `include!` provenance.
    out_dir: Option<PathBuf>,
}

impl<'tcx> Extractor<'tcx> {
    /// The path of folded item `def_id`, recording its entity on first sight.
    fn entity(&mut self, def_id: DefId) -> ItemPath {
        if let Some(entity) = self.entities.get(&def_id) {
            return entity.path.clone();
        }
        let names = &self.names;
        let kind = names
            .entity_kind(def_id)
            .expect("`Names::fold` only returns items with an entity kind");
        let (mut path, module) = names.naming(def_id);
        if self
            .by_path
            .get(&path)
            .is_some_and(|&other| other != def_id)
        {
            // Last resort for the ambiguities listed in the `names` docs: stable per item, but
            // only applied in units that see both items.
            let hash = self.tcx.def_path_hash(def_id).local_hash().as_u64();
            let unique = ItemPath::new(format!("{path} #{hash:016x}"));
            self.tcx.dcx().warn(format!(
                "turboflex: two items print as `{path}`; naming one `{unique}`"
            ));
            self.collisions.push(unique.clone());
            path = unique;
        }
        let entity = Entity {
            path: path.clone(),
            kind,
            module,
            crate_name: names.crate_name(def_id),
            origin: names.origin(def_id),
            span: names.def_span(def_id),
        };
        self.by_path.insert(path.clone(), def_id);
        self.entities.insert(def_id, entity);
        path
    }

    /// Generated by the test harness: its `main` and the `#[test]`/`#[bench]` descriptor statics
    /// (which share their test fn's name). Not user code, and not entities.
    fn is_harness(&self, def_id: DefId) -> bool {
        let tcx = self.tcx;
        if !def_id.is_local() {
            return false;
        }
        if tcx.sess.is_test_crate() && tcx.entry_fn(()).is_some_and(|(main, _)| main == def_id) {
            return true;
        }
        find_attr!(tcx, def_id, RustcTestMarker(..))
            || tcx
                .def_span(def_id)
                .macro_backtrace()
                .any(|expn| matches!(expn.kind, ExpnKind::AstPass(_)))
    }

    fn provenance(&self, span: Span, src: DefId, implicit: Implicit) -> Prov {
        let mut desugaring = None;
        let mut derive = None;
        let mut bang = None;
        for expn in span.macro_backtrace() {
            match expn.kind {
                ExpnKind::Desugaring(kind) => {
                    desugaring.get_or_insert_with(|| format!("{kind:?}"));
                }
                ExpnKind::Macro(MacroKind::Derive, name) => derive = Some(name.to_string()),
                // Outermost macro wins (e.g. `format!` rather than the inner `format_args!`).
                ExpnKind::Macro(MacroKind::Bang | MacroKind::Attr, name) => {
                    bang = Some(name.to_string());
                }
                ExpnKind::AstPass(pass) => bang = Some(pass.descr().to_string()),
                ExpnKind::Root => {}
            }
        }
        if let Some(name) = derive {
            return Prov {
                provenance: Provenance::Derive,
                detail: format!("derive={name}"),
            };
        }
        // A desugaring inside macro output (e.g. `FormatLiteral` inside `format_args!`) is
        // macro-generated code; a macro call inside a desugaring is not possible.
        if let Some(name) = bang {
            let detail = match desugaring {
                Some(kind) => format!("macro={name};{kind}"),
                None => format!("macro={name}"),
            };
            return Prov {
                provenance: Provenance::MacroExpanded,
                detail,
            };
        }
        if let Some(kind) = desugaring {
            return Prov {
                provenance: Provenance::Desugared,
                detail: kind,
            };
        }
        // `include!`d files are parsed with root contexts: recognise build-script output by path.
        if let Some(out_dir) = &self.out_dir
            && let rustc_span::FileName::Real(real) =
                self.tcx.sess.source_map().span_to_filename(span)
            && real.local_path().is_some_and(|p| p.starts_with(out_dir))
        {
            return Prov {
                provenance: Provenance::MacroExpanded,
                detail: "include!(OUT_DIR)".into(),
            };
        }
        match implicit {
            Implicit::Operator => {
                return Prov {
                    provenance: Provenance::Desugared,
                    detail: "OverloadedOperator".into(),
                };
            }
            Implicit::Drop => {
                return Prov {
                    provenance: Provenance::Desugared,
                    detail: "DropGlue".into(),
                };
            }
            Implicit::No => {}
        }
        // Spans inside derive output often come from the input tokens (field names/types): fall
        // back to the expansion of the owning item.
        let item_span = self.tcx.def_span(src);
        if let Some(name) = item_span.macro_backtrace().find_map(|e| match e.kind {
            ExpnKind::Macro(MacroKind::Derive, name) => Some(name),
            ExpnKind::Macro(MacroKind::Bang | MacroKind::Attr, _)
            | ExpnKind::Root
            | ExpnKind::AstPass(_)
            | ExpnKind::Desugaring(_) => None,
        }) {
            return Prov {
                provenance: Provenance::Derive,
                detail: format!("derive={name}"),
            };
        }
        Prov {
            provenance: Provenance::Source,
            detail: String::new(),
        }
    }

    /// Records the edge `src -> dst` (both folded first), returning its position in `edges`. Of
    /// several occurrences of one `(src, dst, kind)` the first is kept, unless a later one is plain
    /// source and it is not (e.g. the written `fetch()` call over the `.await`-desugared poll of
    /// the same coroutine).
    fn emit(
        &mut self,
        src: DefId,
        dst: DefId,
        kind: EdgeKind,
        span: Span,
        implicit: Implicit,
        extra: &str,
    ) -> Option<usize> {
        let src = self.names.fold(src);
        let dst = self.names.fold(dst);
        if src == dst || self.is_harness(src) {
            return None;
        }
        let src_path = self.entity(src);
        let dst_path = self.entity(dst);
        let key = (src_path, dst_path, kind);
        let prov = self.provenance(span, src, implicit);
        let existing = self.index.get(&key).copied();
        if let Some(i) = existing
            && !(self.edges[i].provenance != Provenance::Source
                && prov.provenance == Provenance::Source)
        {
            return existing;
        }
        let detail = match (prov.detail.is_empty(), extra.is_empty()) {
            (true, _) => extra.to_owned(),
            (false, true) => prov.detail,
            (false, false) => format!("{};{extra}", prov.detail),
        };
        let (src, dst, kind) = key.clone();
        let edge = Edge {
            src,
            dst,
            kind,
            provenance: prov.provenance,
            span: self.names.span(span.source_callsite()),
            detail,
        };
        if let Some(i) = existing {
            self.edges[i] = edge;
            Some(i)
        } else {
            let i = self.edges.len();
            self.index.insert(key, i);
            self.edges.push(edge);
            Some(i)
        }
    }

    /// `constructs` from `src` to ADT `adt`, collecting the enum `variant` built into the detail.
    fn constructs(&mut self, src: DefId, adt: DefId, variant: Option<Symbol>, span: Span) {
        let i = self.emit(src, adt, EdgeKind::Constructs, span, Implicit::No, "");
        if let (Some(i), Some(variant)) = (i, variant) {
            self.variants
                .entry(i)
                .or_default()
                .insert(variant.to_string());
        }
    }

    /// src item -> macro definition, for every macro in the expansion chain of `span`.
    fn macro_uses(&mut self, src: DefId, span: Span) {
        if span.ctxt().is_root() || !self.macro_seen.insert((src, span.ctxt())) {
            return;
        }
        for expn in span.macro_backtrace() {
            if let ExpnKind::Macro(..) = expn.kind
                && let Some(mac) = expn.macro_def_id
            {
                self.emit(
                    src,
                    mac,
                    EdgeKind::MacroUse,
                    expn.call_site,
                    Implicit::No,
                    "",
                );
            }
        }
    }

    // ---- types -------------------------------------------------------------------------------

    /// Emit `kind` for ADTs/traits at the top level of `ty` (through references, pointers,
    /// slices, arrays, tuples) and `generic_arg` for those nested in generic args.
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "rustc's TyKind: only the listed variants name items"
    )]
    fn walk_ty(&mut self, src: DefId, ty: Ty<'tcx>, kind: EdgeKind, nested: bool, span: Span) {
        if !self.ty_seen.insert((src, ty, kind, nested)) {
            return;
        }
        let k = if nested { EdgeKind::GenericArg } else { kind };
        match *ty.kind() {
            ty::Adt(adt, args) => {
                self.emit(src, adt.did(), k, span, Implicit::No, "");
                for t in args.types() {
                    self.walk_ty(src, t, kind, true, span);
                }
            }
            ty::Foreign(def) => {
                self.emit(src, def, k, span, Implicit::No, "");
            }
            ty::Ref(_, t, _)
            | ty::RawPtr(t, _)
            | ty::Slice(t)
            | ty::Array(t, _)
            | ty::Pat(t, _) => self.walk_ty(src, t, kind, nested, span),
            ty::Tuple(ts) => {
                for t in ts {
                    self.walk_ty(src, t, kind, nested, span);
                }
            }
            ty::Dynamic(preds, ..) => {
                for pred in preds {
                    match pred.skip_binder() {
                        ty::ExistentialPredicate::Trait(tr) => {
                            self.emit(src, tr.def_id, k, span, Implicit::No, "dyn");
                            for t in tr.args.types() {
                                self.walk_ty(src, t, kind, true, span);
                            }
                        }
                        ty::ExistentialPredicate::Projection(p) => {
                            if let Some(t) = p.term.as_type() {
                                self.walk_ty(src, t, kind, true, span);
                            }
                        }
                        ty::ExistentialPredicate::AutoTrait(_) => {}
                    }
                }
            }
            ty::FnPtr(sig_tys, _) => {
                for t in sig_tys.skip_binder().inputs_and_output {
                    self.walk_ty(src, t, kind, true, span);
                }
            }
            ty::FnDef(_, args) => {
                for t in args.skip_binder().types() {
                    self.walk_ty(src, t, kind, true, span);
                }
            }
            ty::Alias(
                _,
                ty::AliasTy {
                    kind: ty::AliasTyKind::Opaque { def_id },
                    ..
                },
            ) => {
                // `impl Trait` (argument-position ones are generic params, handled as predicates).
                let bounds: Vec<_> = self
                    .tcx
                    .explicit_item_bounds(def_id)
                    .iter_identity_copied()
                    .collect();
                for b in bounds {
                    let (clause, _) = b.skip_normalization();
                    self.clause(src, clause, span);
                }
            }
            ty::Alias(_, alias) => {
                for t in alias.args.types() {
                    self.walk_ty(src, t, kind, true, span);
                }
            }
            _ => {}
        }
    }

    /// `bound` edges from where-clauses / `impl Trait` bounds, `generic_arg` for types inside them.
    /// The compiler-implied `Sized`/`MetaSized`/`PointeeSized` bounds are not dependencies.
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "rustc's ClauseKind: only trait and projection clauses name items"
    )]
    fn clause(&mut self, src: DefId, clause: ty::Clause<'tcx>, span: Span) {
        match clause.kind().skip_binder() {
            ty::ClauseKind::Trait(tp) => {
                if let Some(LangItem::Sized | LangItem::MetaSized | LangItem::PointeeSized) =
                    self.tcx.as_lang_item(tp.def_id())
                {
                    return;
                }
                self.emit(src, tp.def_id(), EdgeKind::Bound, span, Implicit::No, "");
                if !matches!(tp.self_ty().kind(), ty::Param(_)) {
                    self.walk_ty(src, tp.self_ty(), EdgeKind::GenericArg, true, span);
                }
                for t in tp.trait_ref.args.types().skip(1) {
                    self.walk_ty(src, t, EdgeKind::GenericArg, true, span);
                }
            }
            ty::ClauseKind::Projection(pp) => {
                let trait_id = pp.projection_term.trait_def_id(self.tcx);
                self.emit(src, trait_id, EdgeKind::Bound, span, Implicit::No, "");
                if let Some(t) = pp.term.as_type() {
                    self.walk_ty(src, t, EdgeKind::GenericArg, true, span);
                }
            }
            _ => {}
        }
    }

    fn predicates(&mut self, def_id: DefId) {
        let span = self.tcx.def_span(def_id);
        let preds: Vec<_> = self.tcx.explicit_clauses_of(def_id).clauses.to_vec();
        for (clause, _) in preds {
            self.clause(def_id, clause, span);
        }
    }

    // ---- items -------------------------------------------------------------------------------

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "rustc's DefKind: other definitions have no signature-level dependencies"
    )]
    fn item(&mut self, def: LocalDefId) {
        let tcx = self.tcx;
        let def_id = def.to_def_id();
        let span = tcx.def_span(def_id);
        self.macro_uses(def_id, span);
        match tcx.def_kind(def_id) {
            DefKind::Fn | DefKind::AssocFn => {
                let sig = tcx
                    .fn_sig(def_id)
                    .instantiate_identity()
                    .skip_normalization()
                    .skip_binder();
                for t in sig.inputs_and_output {
                    self.walk_ty(def_id, t, EdgeKind::UsesTypeSig, false, span);
                }
                self.predicates(def_id);
            }
            DefKind::Struct | DefKind::Enum | DefKind::Union => {
                let fields: Vec<_> = tcx
                    .adt_def(def_id)
                    .all_fields()
                    .map(|f| (f.did, tcx.def_span(f.did)))
                    .collect();
                for (field, fspan) in fields {
                    let t = tcx
                        .type_of(field)
                        .instantiate_identity()
                        .skip_normalization();
                    self.walk_ty(def_id, t, EdgeKind::UsesTypeField, false, fspan);
                }
                self.predicates(def_id);
            }
            DefKind::TyAlias | DefKind::Const | DefKind::Static { .. } | DefKind::AssocConst => {
                let t = tcx
                    .type_of(def_id)
                    .instantiate_identity()
                    .skip_normalization();
                self.walk_ty(def_id, t, EdgeKind::UsesTypeSig, false, span);
            }
            DefKind::Impl { of_trait } => {
                if of_trait {
                    let tr = tcx
                        .impl_trait_ref(def_id)
                        .instantiate_identity()
                        .skip_normalization();
                    self.emit(
                        def_id,
                        tr.def_id,
                        EdgeKind::ImplOfTrait,
                        span,
                        Implicit::No,
                        "",
                    );
                    for t in tr.args.types().skip(1) {
                        self.walk_ty(def_id, t, EdgeKind::GenericArg, true, span);
                    }
                }
                let self_ty = tcx
                    .type_of(def_id)
                    .instantiate_identity()
                    .skip_normalization();
                self.walk_ty(def_id, self_ty, EdgeKind::ImplForType, false, span);
                self.predicates(def_id);
            }
            DefKind::Trait | DefKind::AssocTy => self.predicates(def_id),
            _ => {}
        }
    }

    // ---- bodies ------------------------------------------------------------------------------

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "rustc's DefKind: other body owners (array lengths, inline consts) are type-level"
    )]
    fn body(&mut self, owner: LocalDefId) {
        let tcx = self.tcx;
        let def_id = owner.to_def_id();
        let fn_like = match tcx.def_kind(def_id) {
            DefKind::Fn | DefKind::AssocFn | DefKind::Closure | DefKind::SyntheticCoroutineBody => {
                true
            }
            DefKind::Const | DefKind::AssocConst | DefKind::Static { .. } => false,
            _ => return,
        };
        let typing_env = ty::TypingEnv::post_analysis(tcx, def_id);
        let body = tcx.instance_mir(InstanceKind::Item(def_id));
        BodyWalker {
            x: self,
            src: def_id,
            body,
            typing_env,
            nested_seen: HashSet::new(),
        }
        .walk();
        if fn_like {
            for promoted in tcx.promoted_mir(def_id) {
                BodyWalker {
                    x: self,
                    src: def_id,
                    body: promoted,
                    typing_env,
                    nested_seen: HashSet::new(),
                }
                .walk();
            }
        }
    }
}

struct BodyWalker<'a, 'tcx> {
    x: &'a mut Extractor<'tcx>,
    src: DefId,
    body: &'tcx mir::Body<'tcx>,
    typing_env: ty::TypingEnv<'tcx>,
    nested_seen: HashSet<(DefId, GenericArgsRef<'tcx>)>,
}

/// The trait a fn belongs to (trait method or trait-impl method).
fn trait_of_fn(tcx: TyCtxt<'_>, def_id: DefId) -> Option<DefId> {
    let parent = tcx.opt_parent(def_id)?;
    if tcx.def_kind(parent) == DefKind::Trait {
        return Some(parent);
    }
    if tcx.def_kind(parent) == (DefKind::Impl { of_trait: true }) {
        return Some(tcx.impl_trait_ref(parent).skip_binder().def_id);
    }
    None
}

/// Kind for an edge into a trait method/impl, judged by the trait (for implicit calls).
fn implicit_kind(tcx: TyCtxt<'_>, trait_id: Option<DefId>) -> EdgeKind {
    let Some(trait_id) = trait_id else {
        return EdgeKind::Calls;
    };
    if let Some(LangItem::Deref | LangItem::DerefMut) = tcx.as_lang_item(trait_id) {
        return EdgeKind::ImplicitDeref;
    }
    if let Some(LangItem::Drop) = tcx.as_lang_item(trait_id) {
        return EdgeKind::ImplicitDrop;
    }
    if let Some(name) = tcx.get_diagnostic_name(trait_id) {
        if name == sym::From || name == sym::Into || name == sym::TryFrom {
            return EdgeKind::ImplicitFrom;
        }
        if name == sym::Iterator || name == sym::IntoIterator {
            return EdgeKind::ImplicitIter;
        }
    }
    // Everything else in `core::fmt` / `core::ops` / `core::cmp`, by definition path.
    if tcx.crate_name(trait_id.krate) == sym::core
        && let Some(module) = tcx
            .def_path(trait_id)
            .data
            .first()
            .and_then(|elem| elem.data.get_opt_name())
    {
        if module == sym::fmt {
            return EdgeKind::ImplicitFmt;
        }
        if module == sym::ops || module == sym::cmp {
            return EdgeKind::ImplicitOp;
        }
    }
    EdgeKind::Calls
}

impl<'tcx> BodyWalker<'_, 'tcx> {
    fn walk(&mut self) {
        let body = self.body;
        self.visit_body(body);
    }

    fn normalize<T: ty::TypeFoldable<TyCtxt<'tcx>> + Copy>(&self, v: T) -> T {
        self.x
            .tcx
            .try_normalize_erasing_regions(self.typing_env, ty::Unnormalized::dummy(v))
            .unwrap_or(v)
    }

    /// A use of fn item `callee::<args>`: a call (`call_source` set) or a reification.
    fn fn_item_use(
        &mut self,
        callee: DefId,
        args: GenericArgsRef<'tcx>,
        span: Span,
        call_source: Option<CallSource>,
    ) {
        let tcx = self.x.tcx;
        let args = self.normalize(args);
        for t in args.types() {
            self.x
                .walk_ty(self.src, t, EdgeKind::GenericArg, true, span);
        }
        // A tuple struct/variant constructor builds a value: `constructs`, not `calls`.
        if let DefKind::Ctor(of, _) = tcx.def_kind(callee) {
            let parent = tcx.parent(callee);
            let (adt, variant) = match of {
                CtorOf::Struct => (parent, None),
                CtorOf::Variant => (tcx.parent(parent), Some(tcx.item_name(parent))),
            };
            self.x.constructs(self.src, adt, variant, span);
            return;
        }
        let resolved = if args.has_escaping_bound_vars() {
            None
        } else {
            Instance::try_resolve(tcx, self.typing_env, callee, args)
                .ok()
                .flatten()
        };
        let (target, target_args, detail) = match resolved.map(|i| (i.def, i.args)) {
            Some((InstanceKind::Item(d), a)) => (d, a, ""),
            Some((InstanceKind::Virtual(d, _), _)) => (d, args, "virtual"),
            _ => (callee, args, ""),
        };
        let (kind, implicit, detail) = match call_source {
            None => (EdgeKind::Calls, Implicit::No, "reify"),
            Some(CallSource::Normal) => {
                let for_loop = span.macro_backtrace().any(|e| {
                    matches!(
                        e.kind,
                        ExpnKind::Desugaring(rustc_span::DesugaringKind::ForLoop)
                    )
                });
                let kind = if for_loop {
                    implicit_kind(tcx, trait_of_fn(tcx, target))
                } else {
                    EdgeKind::Calls
                };
                (kind, Implicit::No, detail)
            }
            Some(_) => (
                implicit_kind(tcx, trait_of_fn(tcx, target)),
                Implicit::Operator,
                detail,
            ),
        };
        self.x.emit(self.src, target, kind, span, implicit, detail);
        self.nested(callee, args, span, 0, Via::Call(callee));
        if target != callee {
            self.nested(target, target_args, span, 0, Via::Call(callee));
        }
    }

    /// Resolve the where-clauses of `def::<args>` to impls; every workspace impl reached (directly
    /// or through further impl where-clauses) yields edges to its methods.
    fn nested(
        &mut self,
        def: DefId,
        args: GenericArgsRef<'tcx>,
        span: Span,
        depth: usize,
        via: Via,
    ) {
        let tcx = self.x.tcx;
        if depth > MAX_NESTED_DEPTH || !self.nested_seen.insert((def, args)) {
            return;
        }
        let own_trait = tcx
            .opt_parent(def)
            .filter(|&parent| tcx.def_kind(parent) == DefKind::Trait);
        let preds = tcx.clauses_of(def).instantiate(tcx, args);
        for clause in preds.clauses {
            let clause = clause.skip_normalization();
            let Some(tp) = clause.as_trait_clause() else {
                continue;
            };
            if tp.polarity() != ty::ClausePolarity::Positive {
                continue;
            }
            let tref = tcx.instantiate_bound_regions_with_erased(tp.map_bound(|p| p.trait_ref));
            // The trait method's own `Self: Trait` is the call itself, already resolved above.
            if Some(tref.def_id) == own_trait {
                continue;
            }
            self.resolve_to_impl(tref, span, depth, via);
        }
    }

    /// Select the impl for `tref`; a workspace impl yields edges to its methods (what the
    /// obligation can end up calling), then its own where-clauses are followed.
    fn resolve_to_impl(&mut self, tref: ty::TraitRef<'tcx>, span: Span, depth: usize, via: Via) {
        let tcx = self.x.tcx;
        let tref = self.normalize(tref);
        if tref.has_escaping_bound_vars() || tref.references_error() {
            return;
        }
        let Ok(ImplSource::UserDefined(data)) = shallow_resolve(tcx, self.typing_env, tref) else {
            return;
        };
        if self.x.names.in_workspace(data.impl_def_id) {
            let kind = implicit_kind(tcx, Some(tref.def_id));
            let detail = match via {
                Via::Call(callee) => {
                    format!("nested-impl via {}", self.x.names.naming(callee).0)
                }
                Via::Dyn(trait_id) => {
                    format!("vtable for dyn {}", self.x.names.naming(trait_id).0)
                }
            };
            let fns: Vec<DefId> = tcx
                .associated_item_def_ids(data.impl_def_id)
                .iter()
                .copied()
                .filter(|d| tcx.def_kind(*d) == DefKind::AssocFn)
                .collect();
            if fns.is_empty() {
                self.x.emit(
                    self.src,
                    data.impl_def_id,
                    kind,
                    span,
                    Implicit::No,
                    &detail,
                );
            }
            for f in fns {
                self.x.emit(self.src, f, kind, span, Implicit::No, &detail);
            }
        }
        self.nested(data.impl_def_id, data.args, span, depth + 1, via);
    }

    /// Unsizing `&T` to `&dyn Trait` builds a vtable from `T: Trait`: resolve that impl (this is
    /// how derived `Debug` reaches its fields' `Debug::fmt`).
    fn unsize(&mut self, from: Ty<'tcx>, to: Ty<'tcx>, span: Span) {
        let tcx = self.x.tcx;
        let (Some(from), Some(to)) = (from.builtin_deref(true), to.builtin_deref(true)) else {
            return;
        };
        let ty::Dynamic(preds, ..) = *to.kind() else {
            return;
        };
        let Some(principal) = preds.principal() else {
            return;
        };
        let tref = tcx.instantiate_bound_regions_with_erased(principal.with_self_ty(tcx, from));
        self.resolve_to_impl(tref, span, 0, Via::Dyn(tref.def_id));
    }

    /// Drop glue reaching workspace `Drop` impls. External destructors are not followed: they
    /// would mostly be std internals (`RawVec`, ...), reached through every `String` local.
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "rustc's TyKind: only the listed variants own droppable values"
    )]
    fn drop_ty(&mut self, ty: Ty<'tcx>, span: Span, depth: usize, seen: &mut HashSet<Ty<'tcx>>) {
        let tcx = self.x.tcx;
        if depth > MAX_DROP_DEPTH || !seen.insert(ty) {
            return;
        }
        match *ty.kind() {
            ty::Adt(adt, args) => {
                if let Some(dtor) = tcx.adt_destructor(adt.did())
                    && self.x.names.in_workspace(dtor.did)
                {
                    let target = if matches!(tcx.def_kind(dtor.did), DefKind::Impl { .. }) {
                        tcx.associated_item_def_ids(dtor.did)
                            .first()
                            .copied()
                            .unwrap_or(dtor.did)
                    } else {
                        dtor.did
                    };
                    self.x.emit(
                        self.src,
                        target,
                        EdgeKind::ImplicitDrop,
                        span,
                        Implicit::Drop,
                        "",
                    );
                }
                for field in adt.all_fields() {
                    let fty = self.normalize(field.ty(tcx, args).skip_normalization());
                    self.drop_ty(fty, span, depth + 1, seen);
                }
                // Containers (Vec<T>, Box<T>, Option<T>...) drop their type arguments.
                for t in args.types() {
                    self.drop_ty(t, span, depth + 1, seen);
                }
            }
            ty::Tuple(ts) => {
                for t in ts {
                    self.drop_ty(t, span, depth + 1, seen);
                }
            }
            ty::Array(t, _) | ty::Slice(t) => self.drop_ty(t, span, depth + 1, seen),
            ty::Closure(_, args) => {
                for t in args.as_closure().upvar_tys() {
                    self.drop_ty(t, span, depth + 1, seen);
                }
            }
            _ => {}
        }
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "rustc's DefKind: only consts and statics are globals"
    )]
    fn global(&mut self, def: DefId, span: Span) {
        let tcx = self.x.tcx;
        match tcx.def_kind(def) {
            DefKind::Const | DefKind::AssocConst => {
                self.x.emit(
                    self.src,
                    def,
                    EdgeKind::ReadsGlobal,
                    span,
                    Implicit::No,
                    "const",
                );
            }
            DefKind::Static { .. } => {
                let mutable = tcx.static_mutability(def) == Some(rustc_hir::Mutability::Mut);
                let ty = tcx.type_of(def).instantiate_identity().skip_normalization();
                let interior = !ty.is_freeze(tcx, self.typing_env);
                let (kind, detail) = match (mutable, interior) {
                    (true, _) => (EdgeKind::WritesGlobal, "static mut (write assumed)"),
                    (false, true) => (
                        EdgeKind::WritesGlobal,
                        "interior mutability (write assumed)",
                    ),
                    (false, false) => (EdgeKind::ReadsGlobal, "static"),
                };
                self.x.emit(self.src, def, kind, span, Implicit::No, detail);
            }
            _ => {}
        }
    }

    /// The span of the `switchInt` ending the block of the assignment at `location`, if it
    /// branches on the assigned value: the whole `match`/`if let`, where the discriminant read
    /// itself has the scrutinee's span (so `matches!(x, ..)` would read as written by hand).
    fn switch_span(&self, location: Location) -> Option<Span> {
        let block = &self.body.basic_blocks[location.block];
        let mir::StatementKind::Assign(assign) =
            &block.statements.get(location.statement_index)?.kind
        else {
            return None;
        };
        let term = block.terminator();
        let TerminatorKind::SwitchInt { discr, .. } = &term.kind else {
            return None;
        };
        (discr.place() == Some(assign.0)).then_some(term.source_info.span)
    }

    fn constant(&mut self, c: mir::Const<'tcx>, span: Span) {
        match c {
            mir::Const::Ty(_, ct) => {
                if let ty::ConstKind::Alias(_, alias) = ct.kind()
                    && let Some(def) = alias.kind.opt_def_id()
                {
                    self.global(def, span);
                }
            }
            mir::Const::Unevaluated(uv, _) => {
                // Promoteds are walked as separate bodies.
                if uv.promoted.is_none() {
                    self.global(uv.def, span);
                }
            }
            mir::Const::Val(ConstValue::Scalar(Scalar::Ptr(ptr, _)), _) => {
                if let Some(GlobalAlloc::Static(def)) =
                    self.x.tcx.try_get_global_alloc(ptr.provenance.alloc_id())
                {
                    self.global(def, span);
                }
            }
            mir::Const::Val(..) => {}
        }
        if let ty::FnDef(def, args) = *c.ty().kind()
            && let Some(args) = args.no_bound_vars()
        {
            self.fn_item_use(def, args, span, None);
        }
    }
}

impl<'tcx> Visitor<'tcx> for BodyWalker<'_, 'tcx> {
    fn visit_source_info(&mut self, source_info: &mir::SourceInfo) {
        self.x.macro_uses(self.src, source_info.span);
    }

    fn visit_local_decl(&mut self, local: mir::Local, decl: &mir::LocalDecl<'tcx>) {
        let ty = self.normalize(decl.ty);
        self.x.walk_ty(
            self.src,
            ty,
            EdgeKind::UsesTypeBody,
            false,
            decl.source_info.span,
        );
        self.super_local_decl(local, decl);
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "rustc's TerminatorKind: other terminators are walked generically"
    )]
    fn visit_terminator(&mut self, term: &mir::Terminator<'tcx>, location: Location) {
        let span = term.source_info.span;
        match &term.kind {
            TerminatorKind::Call {
                func,
                args,
                call_source,
                ..
            } => {
                self.visit_source_info(&term.source_info);
                if let Some((callee, gargs)) = func.const_fn_def() {
                    self.fn_item_use(callee, gargs, span, Some(*call_source));
                } else {
                    self.visit_operand(func, location);
                }
                for arg in args {
                    self.visit_operand(&arg.node, location);
                }
                return;
            }
            TerminatorKind::TailCall { func, args, .. } => {
                self.visit_source_info(&term.source_info);
                if let Some((callee, gargs)) = func.const_fn_def() {
                    self.fn_item_use(callee, gargs, span, Some(CallSource::Normal));
                }
                for arg in args {
                    self.visit_operand(&arg.node, location);
                }
                return;
            }
            TerminatorKind::Drop { place, .. } => {
                let ty = self.normalize(place.ty(self.body, self.x.tcx).ty);
                self.drop_ty(ty, span, 0, &mut HashSet::new());
            }
            _ => {}
        }
        self.super_terminator(term, location);
    }

    fn visit_const_operand(&mut self, constant: &mir::ConstOperand<'tcx>, location: Location) {
        self.constant(constant.const_, constant.span);
        self.super_const_operand(constant, location);
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "rustc's Rvalue: only TLS refs, unsizing casts, ADT aggregates and discriminant \
                  reads carry dependencies here"
    )]
    fn visit_rvalue(&mut self, rvalue: &mir::Rvalue<'tcx>, location: Location) {
        let span = self.body.source_info(location).span;
        match rvalue {
            mir::Rvalue::ThreadLocalRef(def) => self.global(*def, span),
            mir::Rvalue::Aggregate(kind, _) => {
                if let mir::AggregateKind::Adt(adt, variant, ..) = **kind {
                    let adt_def = self.x.tcx.adt_def(adt);
                    let variant = adt_def.is_enum().then(|| adt_def.variant(variant).name);
                    self.x.constructs(self.src, adt, variant, span);
                }
            }
            mir::Rvalue::Discriminant(place) => {
                let ty = self.normalize(place.ty(self.body, self.x.tcx).ty);
                if let ty::Adt(adt, _) = *ty.kind()
                    && adt.is_enum()
                {
                    let span = self.switch_span(location).unwrap_or(span);
                    self.x.emit(
                        self.src,
                        adt.did(),
                        EdgeKind::MatchesOn,
                        span,
                        Implicit::No,
                        "",
                    );
                }
            }
            mir::Rvalue::Cast(
                mir::CastKind::PointerCoercion(PointerCoercion::Unsize, _),
                op,
                to,
            ) => {
                let from = self.normalize(op.ty(self.body, self.x.tcx));
                let to = self.normalize(*to);
                self.unsize(from, to, span);
            }
            _ => {}
        }
        self.super_rvalue(rvalue, location);
    }
}

/// Shallow trait selection (which impl satisfies `trait_ref`?), after
/// `rustc_traits::codegen::codegen_select_candidate` -- but tolerant of generic params, using the
/// body's `ParamEnv`, and returning `Err` instead of ICE-ing on ambiguity.
fn shallow_resolve<'tcx>(
    tcx: TyCtxt<'tcx>,
    typing_env: ty::TypingEnv<'tcx>,
    trait_ref: ty::TraitRef<'tcx>,
) -> Result<ImplSource<'tcx, ()>, CodegenObligationError> {
    let infcx = tcx
        .infer_ctxt()
        .ignoring_regions()
        .build(TypingMode::PostAnalysis);
    let mut selcx = SelectionContext::new(&infcx);
    let obligation = Obligation::new(
        tcx,
        ObligationCause::dummy(),
        typing_env.param_env,
        ty::Binder::dummy(trait_ref),
    );
    let selection = match selcx.poly_select(&obligation) {
        Ok(Some(selection)) => selection,
        Err(SelectionError::Unimplemented) => return Err(CodegenObligationError::Unimplemented),
        Ok(None) | Err(_) => return Err(CodegenObligationError::Ambiguity),
    };
    let ocx = ObligationCtxt::new(&infcx);
    let impl_source = selection.map(|obligation| ocx.register_obligation(obligation));
    if ocx.evaluate_obligations_error_on_ambiguity().has_errors() {
        return Err(CodegenObligationError::Ambiguity);
    }
    let impl_source = infcx.deeply_resolve_ignoring_regions(impl_source);
    let impl_source = tcx.erase_and_anonymize_regions(impl_source);
    if impl_source.has_infer() {
        return Err(CodegenObligationError::Ambiguity);
    }
    Ok(impl_source)
}
