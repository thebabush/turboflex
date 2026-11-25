//! Fixture crate: every module under `cases` exercises one tricky dependency
//! pattern; every other module is a dependency *target* for those cases.
//! See `expected_edges.tsv` at the workspace root.
#![allow(dead_code)]
// Adding `Default` impls or rewriting the async block would add noise edges.
#![allow(clippy::new_without_default, clippy::manual_async_fn)]

pub mod cases;
pub mod infra;

pub mod aliases;
pub mod animals;
pub mod blanket;
pub mod callbacks;
pub mod checks;
pub mod collection;
pub mod config;
pub mod containers;
pub mod defaults;
pub mod error_conv;
pub mod errors_a;
pub mod errors_b;
pub mod extras;
pub mod gen_target;
pub mod impls_c;
pub mod iteration;
pub mod kinds;
pub mod macros_m;
pub mod math;
pub mod model_a;
pub mod payload;
pub mod money;
pub mod money_fmt;
pub mod reexp_a;
pub mod reexp_b;
pub mod render;
pub mod report_impl;
pub mod resources;
pub mod serde_helpers;
pub mod service;
pub mod service_impl;
pub mod shapes;
pub mod sink_n;
pub mod split;
pub mod split_ext;
pub mod tags;
pub mod traits_b;
pub mod types_t;
pub mod util;
pub mod vector;
pub mod vector_ops;
pub mod wrappers;
