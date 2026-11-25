//! Runs the pipeline: loads facts and models, computes each view's reflexion, summarizes it.
//!
//! A *view* is one model file: its components, mapping, rules and policy. [`check`] loads a
//! directory of fact files once ([`load_facts`]), parses every model ([`load_model`]), maps and
//! computes each view over the same [`Graph`], and summarizes the results
//! into a [`Report`] the front ends render. Nothing here prints;
//! failures are [`ServiceError`]s.

mod error;
mod load;
mod summarize;

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use turboflex_engine::Reflexion;
use turboflex_graph::Graph;
use turboflex_model::{Mapping, Model};
use turboflex_report::{Facts, Report};

pub use error::ServiceError;
pub use load::{load_facts, load_model};
pub use summarize::summarize;

/// How much evidence a summary carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Divergent edges listed per divergent pair.
    pub samples: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self { samples: 3 }
    }
}

/// One model computed over the fact base.
#[derive(Debug, Clone)]
pub struct View {
    path: PathBuf,
    model: Model,
    reflexion: Reflexion,
}

impl View {
    /// The model file, as given.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn model(&self) -> &Model {
        &self.model
    }

    #[must_use]
    pub fn reflexion(&self) -> &Reflexion {
        &self.reflexion
    }
}

/// The result of [`check`]: the fact base, every view, and their summary.
#[derive(Debug, Clone)]
pub struct Check {
    graph: Graph,
    views: Vec<View>,
    report: Report,
}

impl Check {
    #[must_use]
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// The views, in the order their models were given.
    #[must_use]
    pub fn views(&self) -> &[View] {
        &self.views
    }

    #[must_use]
    pub fn report(&self) -> &Report {
        &self.report
    }

    /// Whether any view has divergences or conflicts.
    #[must_use]
    pub fn has_findings(&self) -> bool {
        self.report.has_findings()
    }

    /// Renders the summary for humans.
    pub fn write_text(&self, writer: impl Write) -> io::Result<()> {
        self.report.write_text(writer)
    }

    /// Renders the summary as JSON.
    pub fn write_json(&self, writer: impl Write) -> io::Result<()> {
        self.report.write_json(writer)
    }
}

/// Checks the facts in `facts_dir` against every model in `models`, in order.
///
/// The models are parsed first, so a broken model fails before the (slower) fact loading.
pub fn check(
    facts_dir: &Path,
    models: &[PathBuf],
    options: Options,
) -> Result<Check, ServiceError> {
    let models = models
        .iter()
        .map(|path| Ok((path.clone(), load_model(path)?)))
        .collect::<Result<Vec<_>, ServiceError>>()?;
    let graph = load_facts(facts_dir)?;
    let views: Vec<View> = models
        .into_iter()
        .map(|(path, model)| {
            let mapping = Mapping::new(&model, &graph);
            let reflexion = Reflexion::compute(&model, &graph, &mapping);
            View {
                path,
                model,
                reflexion,
            }
        })
        .collect();
    let report = Report {
        facts: Facts {
            units: graph.headers().len(),
            entities: graph.entities().count(),
            edges: graph.edges().len(),
        },
        views: views
            .iter()
            .map(|view| summarize(&view.model, &graph, &view.reflexion, options))
            .collect(),
    };
    Ok(Check {
        graph,
        views,
        report,
    })
}
