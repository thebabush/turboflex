//! Errors of the engine's queries.

use turboflex_model::ComponentId;

/// A query the engine cannot answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    #[error("no component `{0}` in the model")]
    UnknownComponent(ComponentId),
}
