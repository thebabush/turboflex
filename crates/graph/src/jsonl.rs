//! JSON Lines encoding of a [`Unit`]: a header record, then entity records, then edge records.
//!
//! ```text
//! {"record":"header","schema_version":1,"crate_name":"app","target":"lib","mode":"normal","features":[]}
//! {"record":"entity","path":"app::util::helper","kind":"function",...}
//! {"record":"edge","src":"app::main","dst":"app::util::helper","kind":"calls",...}
//! ```

use std::collections::HashSet;
use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};

use crate::fact::{CrateName, Edge, Entity, ItemPath, Unit, UnitHeader};
use crate::kind::{CompileMode, TargetKind};

/// Version of the fact file format this build reads and writes. Bump it on any change to the
/// records: readers reject unknown fields and values, so every change is a breaking one.
pub const SCHEMA_VERSION: u32 = 2;

/// Why a fact file could not be read. Line numbers are 1-based.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("line {line}: read failed")]
    Io {
        line: usize,
        #[source]
        source: io::Error,
    },
    #[error("line {line}: invalid record: {source}")]
    Json {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("first line is not a header record with a schema_version")]
    MissingHeader,
    #[error("unsupported fact schema version {found} (this build reads version {SCHEMA_VERSION})")]
    UnsupportedVersion { found: u32 },
    #[error("line {line}: second header record")]
    DuplicateHeader { line: usize },
    #[error("line {line}: second entity record for `{path}`")]
    DuplicateEntity { line: usize, path: ItemPath },
    #[error("line {line}: edge endpoint `{path}` has no preceding entity record")]
    UndeclaredEndpoint { line: usize, path: ItemPath },
}

/// One line of a fact file. Generic so that writing can borrow what reading owns.
#[derive(Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
enum Record<N, E> {
    Header(HeaderRecord),
    Entity(N),
    Edge(E),
}

/// The wire form of [`UnitHeader`]: the version lives only here, so an in-memory header cannot
/// carry a version other than [`SCHEMA_VERSION`].
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HeaderRecord {
    schema_version: u32,
    crate_name: CrateName,
    target: TargetKind,
    mode: CompileMode,
    features: Vec<String>,
}

/// Just enough of the first line to check the version before trusting the rest of its shape.
#[derive(Deserialize)]
struct VersionProbe {
    schema_version: Option<u32>,
}

impl Unit {
    /// Writes the unit as JSON Lines: header, entities, edges. Flushes `writer` at the end.
    pub fn write_jsonl(&self, mut writer: impl Write) -> io::Result<()> {
        let UnitHeader {
            crate_name,
            target,
            mode,
            features,
        } = &self.header;
        let header = HeaderRecord {
            schema_version: SCHEMA_VERSION,
            crate_name: crate_name.clone(),
            target: *target,
            mode: *mode,
            features: features.clone(),
        };
        write_record(&mut writer, &Record::<&Entity, &Edge>::Header(header))?;
        for entity in &self.entities {
            write_record(&mut writer, &Record::<_, &Edge>::Entity(entity))?;
        }
        for edge in &self.edges {
            write_record(&mut writer, &Record::<&Entity, _>::Edge(edge))?;
        }
        writer.flush()
    }

    /// Reads a unit written by [`Unit::write_jsonl`] (by this or another build with the same
    /// [`SCHEMA_VERSION`]).
    pub fn read_jsonl(reader: impl BufRead) -> Result<Self, ReadError> {
        let mut lines = (1..).zip(reader.lines());
        let Some((line, first)) = lines.next() else {
            return Err(ReadError::MissingHeader);
        };
        let first = first.map_err(|source| ReadError::Io { line, source })?;
        let header = parse_header(line, &first)?;
        let mut unit = Unit {
            header,
            entities: Vec::new(),
            edges: Vec::new(),
        };
        let mut declared = HashSet::new();
        for (line, text) in lines {
            let text = text.map_err(|source| ReadError::Io { line, source })?;
            match parse_record(line, &text)? {
                Record::Header(_) => return Err(ReadError::DuplicateHeader { line }),
                Record::Entity(entity) => {
                    if !declared.insert(entity.path.clone()) {
                        return Err(ReadError::DuplicateEntity {
                            line,
                            path: entity.path,
                        });
                    }
                    unit.entities.push(entity);
                }
                Record::Edge(edge) => {
                    for path in [&edge.src, &edge.dst] {
                        if !declared.contains(path) {
                            let path = path.clone();
                            return Err(ReadError::UndeclaredEndpoint { line, path });
                        }
                    }
                    unit.edges.push(edge);
                }
            }
        }
        Ok(unit)
    }
}

fn write_record(writer: &mut impl Write, record: &impl Serialize) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, record)?;
    writer.write_all(b"\n")
}

fn parse_record(line: usize, text: &str) -> Result<Record<Entity, Edge>, ReadError> {
    serde_json::from_str(text).map_err(|source| ReadError::Json { line, source })
}

fn parse_header(line: usize, text: &str) -> Result<UnitHeader, ReadError> {
    let probe: VersionProbe =
        serde_json::from_str(text).map_err(|source| ReadError::Json { line, source })?;
    match probe.schema_version {
        None => return Err(ReadError::MissingHeader),
        Some(SCHEMA_VERSION) => {}
        Some(found) => return Err(ReadError::UnsupportedVersion { found }),
    }
    match parse_record(line, text)? {
        Record::Header(HeaderRecord {
            schema_version: _,
            crate_name,
            target,
            mode,
            features,
        }) => Ok(UnitHeader {
            crate_name,
            target,
            mode,
            features,
        }),
        Record::Entity(_) | Record::Edge(_) => Err(ReadError::MissingHeader),
    }
}
