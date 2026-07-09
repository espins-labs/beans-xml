//! Conformance corpus runner — `fixtures/**/{name}.xml` paired with
//! `{name}.expected.json`.
//!
//! This corpus IS the crate's public contract (per this crate's own
//! `AGENTS.md`: "the **public** contract ... is the published JSON schema
//! ... plus the conformance fixtures in `fixtures/`") — ports to other
//! languages conform to these pairs (tree-sitter corpus-test style), not to
//! this codebase. Same convention as sibling crate `batis-xml`'s own
//! `tests/conformance.rs`.
//!
//! `expected.json` files are never written by hand — they are generated via
//! `examples/gen_fixtures.rs`, then hand-reviewed (M1e-fixtures' own task:
//! independently derive a sample of fixtures' bean/ref lists by reading the
//! XML *before* looking at the generated output) before being committed and
//! locked against regressions here.

use std::fs;
use std::path::Path;

/// One directory per fixture group (spec's "minimum set" — core DI,
/// p/c-namespace, collections, import/alias, component-scan, profile, eGov
/// archetypes, hostile). Listed explicitly (rather than walking
/// `fixtures/`'s subdirectories generically) so a typo'd/empty group
/// directory fails loudly via `run_dir`'s own `checked > 0` assertion below,
/// instead of silently not being walked at all.
const FIXTURE_GROUPS: &[&str] = &[
    "core",
    "pc-namespace",
    "collections",
    "import-alias",
    "component-scan",
    "profile",
    "egov-archetypes",
    "hostile",
];

/// Spec's per-group minimum fixture counts (spec's "픽스처 코퍼스" section: "최소:
/// 코어 DI 15+, p/c-namespace 4+, 컬렉션 5+(merge/value-type), import/alias 4+,
/// component-scan 3+, profile 3+, eGov 아키타입 ... 5+"). `hostile` has no numeric
/// floor in the spec (just "hostile 세트"), so it's covered only by `run_dir`'s
/// own `checked > 0` assertion, not listed here.
const FIXTURE_GROUP_MINIMUMS: &[(&str, usize)] = &[
    ("core", 15),
    ("pc-namespace", 4),
    ("collections", 5),
    ("import-alias", 4),
    ("component-scan", 3),
    ("profile", 3),
    ("egov-archetypes", 5),
];

fn run_dir(dir: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(dir);
    let mut checked = 0;
    let mut entries: Vec<_> = fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("fixture dir {dir} exists: {e}"))
        .map(|entry| entry.expect("dir entry").path())
        .collect();
    // Deterministic order: file system read-dir order isn't guaranteed
    // stable across platforms, and a flaky test-run order is exactly the
    // kind of thing that erodes trust in this conformance suite.
    entries.sort();
    for path in entries {
        if path.extension().and_then(|e| e.to_str()) != Some("xml") {
            continue;
        }
        let expected_path = path.with_extension("expected.json");
        let expected: serde_json::Value = serde_json::from_slice(
            &fs::read(&expected_path)
                .unwrap_or_else(|e| panic!("read expected {}: {e}", expected_path.display())),
        )
        .unwrap_or_else(|e| panic!("{} is valid json: {e}", expected_path.display()));
        let input = fs::read(&path).expect("read fixture xml");
        let actual =
            serde_json::to_value(beans_xml::parse_bytes(&input)).expect("serialize ParseResult");
        assert_eq!(actual, expected, "conformance mismatch: {}", path.display());
        checked += 1;
    }
    println!("{dir}: {checked} pairs checked");
    assert!(
        checked > 0,
        "no {dir} conformance pairs found — did the fixtures move?"
    );
}

#[test]
fn conformance_core() {
    run_dir("core");
}

#[test]
fn conformance_pc_namespace() {
    run_dir("pc-namespace");
}

#[test]
fn conformance_collections() {
    run_dir("collections");
}

#[test]
fn conformance_import_alias() {
    run_dir("import-alias");
}

#[test]
fn conformance_component_scan() {
    run_dir("component-scan");
}

#[test]
fn conformance_profile() {
    run_dir("profile");
}

#[test]
fn conformance_egov_archetypes() {
    run_dir("egov-archetypes");
}

#[test]
fn conformance_hostile() {
    run_dir("hostile");
}

/// Every entry of `FIXTURE_GROUPS` actually goes through `run_dir`'s real
/// parse-vs-`.expected.json` comparison — the per-group `#[test]` functions
/// above each call `run_dir` too, but each one only exists because someone
/// remembered to hand-write it. A newly registered `FIXTURE_GROUPS` entry
/// (with its directory added and populated, so
/// `fixture_groups_cover_every_fixtures_subdirectory` and
/// `every_fixture_has_a_paired_expected_json` both stay green) but no
/// matching dedicated `#[test]` would otherwise never have its fixtures
/// actually re-parsed and diffed against their `.expected.json` by anything
/// in this file — this loop closes that gap unconditionally, independent of
/// whether a per-group test was ever added.
#[test]
fn conformance_every_registered_group_runs_through_run_dir() {
    for group in FIXTURE_GROUPS {
        run_dir(group);
    }
}

/// `fixtures/`'s subdirectories are exactly `FIXTURE_GROUPS` — every check in
/// this file iterates the hard-coded `FIXTURE_GROUPS` list (not a generic
/// walk of `fixtures/`, so a typo'd/empty group directory fails loudly rather
/// than silently not being walked — see `FIXTURE_GROUPS`'s own doc comment).
/// That means a *new* group directory (e.g. a future `fixtures/encoding/`)
/// added without updating `FIXTURE_GROUPS` and wiring up a matching
/// `run_dir` test above would otherwise be silently skipped by every check
/// here, including `examples/gen_fixtures.rs`'s own generic walk still
/// happily generating `.expected.json` files nothing ever locks. This test
/// is the loud failure for that gap.
#[test]
fn fixture_groups_cover_every_fixtures_subdirectory() {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut actual: Vec<String> = fs::read_dir(&fixtures_root)
        .expect("fixtures/ exists")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.is_dir())
        .map(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .expect("utf-8 dir name")
                .to_string()
        })
        .collect();
    actual.sort();
    let mut expected: Vec<String> = FIXTURE_GROUPS.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(
        actual, expected,
        "fixtures/ subdirectories drifted from FIXTURE_GROUPS — add a run_dir test \
         above and list the new group in FIXTURE_GROUPS (or remove the stray directory)"
    );
}

/// Every `.xml` fixture has a paired `.expected.json` and vice versa —
/// catches both a fixture added without ever running
/// `examples/gen_fixtures.rs` for it (a scaffolding gap the per-group
/// `run_dir` tests above wouldn't otherwise flag, since they only assert
/// `checked > 0`, not full pairing) and a stale `.expected.json` left behind
/// after its `.xml` was deleted or renamed (which would otherwise never be
/// checked against anything, silently rotting).
#[test]
fn every_fixture_has_a_paired_expected_json() {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut missing_json = Vec::new();
    let mut orphan_json = Vec::new();
    let mut total_xml = 0;
    for group in FIXTURE_GROUPS {
        let dir = fixtures_root.join(group);
        for entry in fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {group}: {e}")) {
            let path = entry.expect("dir entry").path();
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .expect("utf-8 file name");
            if let Some(stem) = file_name.strip_suffix(".expected.json") {
                if !path.with_file_name(format!("{stem}.xml")).exists() {
                    orphan_json.push(path);
                }
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("xml") {
                continue;
            }
            total_xml += 1;
            if !path.with_extension("expected.json").exists() {
                missing_json.push(path);
            }
        }
    }
    assert!(
        missing_json.is_empty(),
        "fixture(s) missing a paired .expected.json (run `cargo run --example gen_fixtures`): {missing_json:?}"
    );
    assert!(
        orphan_json.is_empty(),
        "orphan .expected.json with no matching .xml fixture (stale — delete it): {orphan_json:?}"
    );
    assert!(total_xml >= 40, "fewer fixtures than expected: {total_xml}");
}

/// Corpus size doesn't just clear a loose global floor (`total_xml >= 40`
/// above) — each group individually meets the spec's own stated minimum
/// (spec's "픽스처 코퍼스" section). Catches e.g. a regression that deletes
/// several `core` fixtures while adding enough elsewhere to keep the global
/// total above 40, which the global-only check would miss.
#[test]
fn fixture_corpus_meets_spec_minimums_per_group() {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    for &(group, minimum) in FIXTURE_GROUP_MINIMUMS {
        let dir = fixtures_root.join(group);
        let count = fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read {group}: {e}"))
            .map(|entry| entry.expect("dir entry").path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("xml"))
            .count();
        assert!(
            count >= minimum,
            "fixtures/{group} has {count} fixture(s), below the spec's minimum of {minimum}"
        );
    }
}

/// Every fixture's `ParseResult` JSON round-trips through `Deserialize` back
/// to an equal value — the whole corpus, not just the hand-picked cases in
/// `src/model.rs`'s own unit tests, exercising `InjectValue`/`Collection`'s
/// hand-rolled `Deserialize` impls (invariant #6: "InjectValue/Collection's
/// unknown `kind` JSON is round-tripped") against every real shape this
/// crate's own parser actually produces.
#[test]
fn parse_result_round_trips_through_serde_across_the_whole_corpus() {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut checked = 0;
    for group in FIXTURE_GROUPS {
        let dir = fixtures_root.join(group);
        for entry in fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {group}: {e}")) {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("xml") {
                continue;
            }
            let input = fs::read(&path).expect("read fixture xml");
            let result = beans_xml::parse_bytes(&input);
            let json = serde_json::to_string(&result).expect("serialize");
            let round_tripped: beans_xml::ParseResult =
                serde_json::from_str(&json).expect("deserializes back");
            let json_again = serde_json::to_string(&round_tripped).expect("re-serialize");
            assert_eq!(
                json,
                json_again,
                "ParseResult round-trip drifted for {}",
                path.display()
            );
            checked += 1;
        }
    }
    println!("round-trip: {checked} fixture(s) checked");
    assert!(checked > 0, "no fixtures found for the round-trip check");
}

/// I5 (build plan): validates every fixture's serialized `ParseResult` JSON
/// against the committed `schema/beans-xml.v1.json` with a real JSON-Schema
/// validator. `sb_u0_schema_matches_committed_snapshot` (`tests/u0_model.rs`,
/// `--features schema`) only pins the schema *text* against a fresh
/// `schemars::schema_for!` run -- it says nothing about whether the schema
/// actually *describes* what `serde` really emits. A `schemars` attribute
/// that drifts from the matching `serde` one (e.g. a field marked
/// `#[serde(skip_serializing_if = "...")]` without a matching `schemars`
/// annotation, so the schema says `required` for a field that can be
/// absent) would pass every other test in this suite -- including the
/// snapshot pin, which only compares schemars' own output to itself --
/// while shipping a schema that rejects the crate's own real output. This
/// test is what actually catches that: it runs the whole corpus's genuine
/// serialized output through the published schema.
#[test]
fn fixture_corpus_serialized_json_matches_json_schema() {
    let schema_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/beans-xml.v1.json");
    let schema_text = fs::read_to_string(&schema_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", schema_path.display()));
    let schema: serde_json::Value = serde_json::from_str(&schema_text)
        .unwrap_or_else(|e| panic!("{} is valid json: {e}", schema_path.display()));
    let validator = jsonschema::validator_for(&schema)
        .unwrap_or_else(|e| panic!("{} is a valid JSON Schema: {e}", schema_path.display()));

    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut checked = 0;
    for group in FIXTURE_GROUPS {
        let dir = fixtures_root.join(group);
        for entry in fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {group}: {e}")) {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("xml") {
                continue;
            }
            let input = fs::read(&path).expect("read fixture xml");
            let actual = serde_json::to_value(beans_xml::parse_bytes(&input))
                .expect("serialize ParseResult");
            let errors: Vec<String> = validator
                .iter_errors(&actual)
                .map(|e| format!("{e} (at {})", e.instance_path))
                .collect();
            assert!(
                errors.is_empty(),
                "{}'s serialized ParseResult violates schema/beans-xml.v1.json: {errors:?}",
                path.display()
            );
            checked += 1;
        }
    }
    println!("schema validation: {checked} fixture(s) checked");
    assert!(
        checked > 0,
        "no fixtures found for the schema-validation check"
    );
}
