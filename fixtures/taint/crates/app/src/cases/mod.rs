//! One module per case. Each file's header says what it tests: `lNN` cases are legit and must
//! not be flagged, `vNN` are capability violations, `bNN` are sanitizer bypasses. b06-b08 live
//! next to the type, in `crate::safe_cmd`; b09 is not caught.

pub mod b01_tuple_literal;
pub mod b02_ctor_as_fn;
pub mod b03_from_string;
pub mod b04_assoc_elsewhere;
pub mod b05_braced_literal;
pub mod b09_field_mutation;
pub mod l01_run_via_exec;
pub mod l02_save_via_storage;
pub mod l03_read_file;
pub mod l04_pass_safe_cmd;
pub mod v01_spawn_directly;
pub mod v02_write_directly;
