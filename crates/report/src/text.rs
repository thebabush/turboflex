//! The text rendering: compact, one finding per line, evidence indented under it.

use std::io::{self, Write};

use serde::Serialize;

use crate::summary::{Absence, Conflict, Divergence, EdgeSample, Report, View};

pub(crate) fn write(report: &Report, mut w: impl Write) -> io::Result<()> {
    let facts = &report.facts;
    writeln!(
        w,
        "facts: {} units, {} entities, {} edges",
        facts.units, facts.entities, facts.edges
    )?;
    for view in &report.views {
        writeln!(w)?;
        write_view(view, &mut w)?;
    }
    w.flush()
}

fn write_view(view: &View, w: &mut impl Write) -> io::Result<()> {
    let headline = &view.headline;
    writeln!(w, "== {}", view.model)?;
    writeln!(
        w,
        "convergences {}, divergences {}, absences {}, unmapped {}, conflicts {}",
        headline.convergences,
        headline.divergences,
        headline.absences,
        headline.unmapped,
        headline.conflicts,
    )?;
    let skipped: Vec<String> = view
        .skipped
        .iter()
        .map(|(reason, count)| format!("{} {count}", wire_name(reason)))
        .collect();
    writeln!(w, "skipped edges: {}", skipped.join(", "))?;
    if !view.divergences.is_empty() {
        writeln!(w, "\ndivergences (group:allowed+divergent!):")?;
        for divergence in &view.divergences {
            write_divergence(divergence, w)?;
        }
    }
    if !view.absences.is_empty() {
        writeln!(w, "\nabsences:")?;
        for absence in &view.absences {
            write_absence(absence, w)?;
        }
    }
    if !view.unmapped.is_empty() {
        writeln!(w, "\nunmapped (module: entities, unchecked edges):")?;
        for module in &view.unmapped {
            writeln!(
                w,
                "  {}: {} entities, {} edges",
                module.module, module.entities, module.edges
            )?;
        }
    }
    if !view.conflicts.is_empty() {
        writeln!(w, "\nconflicts:")?;
        for conflict in &view.conflicts {
            write_conflict(conflict, w)?;
        }
    }
    Ok(())
}

fn write_divergence(divergence: &Divergence, w: &mut impl Write) -> io::Result<()> {
    let groups: Vec<String> = divergence
        .groups
        .iter()
        .map(|(group, counts)| {
            let name = wire_name(group);
            match (counts.allowed, counts.divergent) {
                (allowed, 0) => format!("{name}:{allowed}"),
                (0, divergent) => format!("{name}:{divergent}!"),
                (allowed, divergent) => format!("{name}:{allowed}+{divergent}!"),
            }
        })
        .collect();
    writeln!(
        w,
        "  {} -> {}  {} edges, {} divergent  [{}]",
        divergence.from,
        divergence.to,
        divergence.edges,
        divergence.divergent,
        groups.join(" "),
    )?;
    for sample in &divergence.samples {
        write_sample(sample, w)?;
    }
    let more = divergence
        .divergent
        .saturating_sub(divergence.samples.len());
    if more > 0 && !divergence.samples.is_empty() {
        writeln!(w, "      ... {more} more")?;
    }
    Ok(())
}

fn write_sample(sample: &EdgeSample, w: &mut impl Write) -> io::Result<()> {
    write!(
        w,
        "      {} -> {} ({}, {})",
        sample.src,
        sample.dst,
        wire_name(&sample.kind),
        wire_name(&sample.provenance),
    )?;
    match &sample.span {
        Some(span) => writeln!(w, " {}:{}", span.file, span.line),
        None => writeln!(w),
    }
}

fn write_absence(absence: &Absence, w: &mut impl Write) -> io::Result<()> {
    let kinds = absence.kinds.as_ref().map_or_else(String::new, |kinds| {
        let kinds: Vec<String> = kinds.iter().map(wire_name).collect();
        format!(" kinds={}", kinds.join(","))
    });
    let location = &absence.location;
    writeln!(
        w,
        "  {} -> {}{kinds}  {}:{}:{}",
        absence.from, absence.to, location.file, location.line, location.column
    )
}

fn write_conflict(conflict: &Conflict, w: &mut impl Write) -> io::Result<()> {
    writeln!(
        w,
        "  {}: claimed by {}; {} edges",
        conflict.path,
        conflict.claims.join(", "),
        conflict.edges
    )
}

/// The serialized name of a unit enum variant (`uses_type_sig`), so text and JSON agree.
fn wire_name(value: &impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        Ok(other) => other.to_string(),
        Err(error) => format!("<{error}>"),
    }
}
