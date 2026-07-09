//! Regenerates `schema/beans-xml.v1.json` from the current `src/model.rs`.
//!
//! The output model's serde serialization IS the published schema (see
//! the internal design spec's "enum policy" / "output schema" sections), so this is a thin
//! wrapper around `schemars::schema_for!` — no hand-maintained schema.
//!
//! ```sh
//! cargo run --example gen_schema --features schema > schema/beans-xml.v1.json
//! ```
//!
//! `tests/u0_model.rs`'s `sb_u0_schema_matches_committed_snapshot` (behind the
//! same `schema` feature) fails when run under `--features schema` if the
//! committed file drifts from what this regenerates — re-run the command
//! above to fix. CI's `check` job runs `cargo test --features schema` (see
//! `.github/workflows/ci.yml`), so this is an active gate, not just a local
//! opt-in; the plain (no `--features schema`) `cargo test` gate also keeps
//! its own field-name pin test (`tests/u0_model.rs`) as a default-gate
//! stopgap.

#[cfg(feature = "schema")]
fn main() {
    let schema = schemars::schema_for!(beans_xml::ParseResult);
    let json = serde_json::to_string_pretty(&schema).expect("schema serializes to JSON");
    println!("{json}");
}

#[cfg(not(feature = "schema"))]
fn main() {
    eprintln!("gen_schema requires --features schema");
    std::process::exit(1);
}
