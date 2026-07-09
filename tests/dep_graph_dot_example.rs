//! Output test for `examples/dep_graph_dot.rs` — runs the example binary
//! end-to-end (`std::process::Command`, the simplest option for locking down
//! an example's own `main()`-level behavior: `examples/` targets aren't a
//! `lib` a `tests/` integration binary can `use`, so their edge-building
//! logic can only be exercised either by extracting it into `src/` or by
//! actually running the compiled example — this crate's fixture corpus
//! convention already favors real end-to-end fixtures over unit-level
//! extraction wherever practical, so this follows that) against a small
//! two-file fixture pair under `tests/fixtures/dep_graph/` (deliberately
//! *not* under `fixtures/` itself — `tests/conformance.rs`'s own
//! `fixture_groups_cover_every_fixtures_subdirectory` locks `fixtures/`'s
//! subdirectories to exactly `FIXTURE_GROUPS`, and this pair isn't a
//! conformance corpus entry, just a fixture for this one output test).
//!
//! The fixture pair pins the three example-level fixes together:
//! - **A-2**: `ref="&amp;widgetFactory"` (the well-formed-XML, *entity*
//!   spelling of the FactoryBean marker — this crate's model stores refs
//!   raw/unresolved, invariant #4) must resolve to the node `widgetFactory`,
//!   not the garbage `amp;widgetFactory` a bare-`&`-only strip produces.
//! - **A-3**: both files contain a byte-for-byte identical anonymous
//!   `<bean class="com.example.Helper"/>` at the same span (`file_a.xml`'s
//!   `id="svcA"` and `file_b.xml`'s `id="svcB"` are equal-length, so every
//!   byte from there on — including the anonymous inner bean — lines up at
//!   the same offset in both files) — `main()` merges both files' edges
//!   into one graph, so the two anonymous nodes must stay distinct
//!   (file-qualified), not collide into one merged node under the same
//!   unqualified `$anon@197-231` key.
//! - **A-4**: `<aop:advisor advice-ref="loggingAdviceA"/>` is a decorator
//!   child of `svcA` — its `advice-ref` edge must be emitted from `svcA`
//!   itself, not from the decorator's own (disconnected) node.

use std::path::Path;
use std::process::Command;

#[test]
fn dep_graph_dot_two_file_fixture_produces_the_expected_edge_set() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));

    let output = Command::new(env!("CARGO"))
        .current_dir(manifest_dir)
        .args([
            "run",
            "--quiet",
            "--example",
            "dep_graph_dot",
            "--",
            "tests/fixtures/dep_graph/file_a.xml",
            "tests/fixtures/dep_graph/file_b.xml",
        ])
        .output()
        .expect("spawn `cargo run --example dep_graph_dot`");

    assert!(
        output.status.success(),
        "dep_graph_dot exited non-zero: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is valid utf-8");

    let edge = |from: &str, to: &str| format!("  \"{from}\" -> \"{to}\";");

    // A-3: the two files' identical-span anonymous inner beans get distinct,
    // file-qualified node keys rather than colliding into one.
    let anon_a = "$anon@tests/fixtures/dep_graph/file_a.xml:197-231";
    let anon_b = "$anon@tests/fixtures/dep_graph/file_b.xml:197-231";
    assert_ne!(anon_a, anon_b, "sanity: the two anon keys must differ");

    let expected_edges = [
        edge("svcA", anon_a),
        // A-2: the entity-form factory marker resolves to the clean id, not
        // "amp;widgetFactory".
        edge("svcA", "widgetFactory"),
        // A-4: the decorator's advice-ref is connected to the containing
        // bean, not emitted from the decorator's own disconnected node.
        edge("svcA", "loggingAdviceA"),
        edge("svcB", anon_b),
        edge("svcB", "widgetFactory"),
        edge("svcB", "loggingAdviceB"),
    ];
    for expected in &expected_edges {
        assert!(
            stdout.lines().any(|line| line == expected),
            "expected edge line {expected:?} not found in dep_graph_dot output:\n{stdout}"
        );
    }

    // A-2 regression guard: the garbage un-decoded-strip node must never
    // appear.
    assert!(
        !stdout.contains("amp;widgetFactory"),
        "factory marker must be stripped from the entity form, not left as \
         garbage \"amp;widgetFactory\":\n{stdout}"
    );

    // A-4 regression guard: no edge should still originate from either
    // decorator's own (disconnected) anonymous node — every `advice-ref`
    // edge above is already pinned as coming from svcA/svcB directly, so a
    // decorator-sourced duplicate would only show up as an extra,
    // unexpected line.
    assert!(
        !stdout.contains("scoped-proxy") && !stdout.contains("advisor@"),
        "no edge should be sourced from the decorator's own node:\n{stdout}"
    );
}
