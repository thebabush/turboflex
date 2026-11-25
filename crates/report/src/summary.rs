//! The plain-data summary of a check, as rendered and serialized.

use std::collections::BTreeMap;
use std::io::{self, Write};

use serde::Serialize;
use turboflex_graph::{EdgeGroup, EdgeKind, ItemPath, ModulePath, Provenance, Span};

/// One check: a fact base and every view computed over it, in the order the models were given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub facts: Facts,
    pub views: Vec<View>,
}

/// The size of the merged fact base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Facts {
    /// Fact files (compile units) merged.
    pub units: usize,
    pub entities: usize,
    pub edges: usize,
}

/// The findings of one model over the fact base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct View {
    /// The model's root file, as given (with `.` and `name/..` folded).
    pub model: String,
    pub headline: Headline,
    /// Graph edges not checked, by reason: every [`SkipReason`], 0 where no edge has it, so
    /// every view lists the same reasons in the same order.
    pub skipped: BTreeMap<SkipReason, usize>,
    /// Ordered by `(from, to)`.
    pub divergences: Vec<Divergence>,
    /// In declaration order.
    pub absences: Vec<Absence>,
    /// Unmapped workspace entities grouped by module, ordered by module.
    pub unmapped: Vec<UnmappedModule>,
    /// Ordered by path.
    pub conflicts: Vec<Conflict>,
}

impl View {
    /// Whether the view has divergences or conflicts: the findings that fail a check.
    #[must_use]
    pub fn has_findings(&self) -> bool {
        self.headline.divergences > 0 || self.headline.conflicts > 0
    }
}

/// The counts a view is judged by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Headline {
    /// Component pairs whose edges are all allowed.
    pub convergences: usize,
    /// Component pairs with at least one divergent edge.
    pub divergences: usize,
    /// Rules no edge matches.
    pub absences: usize,
    /// Workspace entities no pattern matches.
    pub unmapped: usize,
    /// Entities several components claim.
    pub conflicts: usize,
}

/// Why a graph edge was not checked; the engine's reasons, by their wire names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// Its group is not in the policy's `edges`.
    GroupFiltered,
    /// Its provenance is not in the policy's `provenance`.
    ProvenanceFiltered,
    /// Seen only in `cfg(test)` builds, and the policy excludes those.
    TestOnly,
    /// An end is an unmapped workspace entity.
    Unmapped,
    /// An end is claimed by several components.
    Conflict,
    /// An end is an external entity no external component claims.
    OutOfScope,
    /// Its target end belongs only to components outside the policy's `targets`.
    NotTargeted,
    /// Both ends belong to the same component.
    SameComponent,
}

impl SkipReason {
    /// Every reason, in declaration order (which is also their `Ord` order).
    pub const ALL: [Self; 8] = [
        Self::GroupFiltered,
        Self::ProvenanceFiltered,
        Self::TestOnly,
        Self::Unmapped,
        Self::Conflict,
        Self::OutOfScope,
        Self::NotTargeted,
        Self::SameComponent,
    ];
}

// Fails to compile when a reason is added to `SkipReason`, as a reminder to extend `ALL`; the
// position check below fails if one is listed out of place.
const _: () = {
    const fn position(reason: SkipReason) -> usize {
        match reason {
            SkipReason::GroupFiltered => 0,
            SkipReason::ProvenanceFiltered => 1,
            SkipReason::TestOnly => 2,
            SkipReason::Unmapped => 3,
            SkipReason::Conflict => 4,
            SkipReason::OutOfScope => 5,
            SkipReason::NotTargeted => 6,
            SkipReason::SameComponent => 7,
        }
    }
    let mut i = 0;
    while i < SkipReason::ALL.len() {
        assert!(
            position(SkipReason::ALL[i]) == i,
            "SkipReason::ALL out of order"
        );
        i += 1;
    }
};

/// A component pair with at least one divergent edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Divergence {
    pub from: String,
    pub to: String,
    /// Every edge of the pair, allowed or not.
    pub edges: usize,
    pub divergent: usize,
    /// The groups present in the pair, by verdict.
    pub groups: BTreeMap<EdgeGroup, GroupCount>,
    /// The first divergent edges, in `(src, dst, kind)` order.
    pub samples: Vec<EdgeSample>,
}

/// How many edges of one group a pair has, by verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct GroupCount {
    pub allowed: usize,
    pub divergent: usize,
}

/// One graph edge, as evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EdgeSample {
    pub src: ItemPath,
    pub dst: ItemPath,
    pub kind: EdgeKind,
    pub provenance: Provenance,
    pub span: Option<Span>,
}

/// A rule no edge matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Absence {
    pub from: String,
    pub to: String,
    /// `None` when the rule allows every group.
    pub kinds: Option<Vec<EdgeGroup>>,
    pub location: ModelLocation,
}

/// Where something is written in a model file; line and column are 1-based.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelLocation {
    pub file: String,
    pub line: usize,
    pub column: usize,
}

/// The unmapped workspace entities of one module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnmappedModule {
    pub module: ModulePath,
    pub entities: usize,
    /// Edges not checked because an end is one of these entities.
    pub edges: usize,
}

/// An entity several components claim (helper-follow).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Conflict {
    pub path: ItemPath,
    /// The claiming components.
    pub claims: Vec<String>,
    /// Edges not checked because an end is this entity.
    pub edges: usize,
}

impl Report {
    /// Whether any view has divergences or conflicts.
    #[must_use]
    pub fn has_findings(&self) -> bool {
        self.views.iter().any(View::has_findings)
    }

    /// Writes the report for humans: the fact base, then each view under a header naming its
    /// model file.
    pub fn write_text(&self, writer: impl Write) -> io::Result<()> {
        crate::text::write(self, writer)
    }

    /// Writes the report as one pretty-printed JSON object, then a newline.
    pub fn write_json(&self, mut writer: impl Write) -> io::Result<()> {
        serde_json::to_writer_pretty(&mut writer, self)?;
        writeln!(writer)
    }
}
