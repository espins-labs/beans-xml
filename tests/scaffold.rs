//! Scaffold smoke test — asserts the public API compiles and the gates run.
//! Real unit tests (`sb_<nn>_*` / per-unit files) land with each build-plan unit.
//!
//! Updated for U3 (root + header + dispatch skeleton landed): `<beans/>`
//! now genuinely parses instead of the old universal `NotBeansRoot` stub —
//! see `tests/u3_root_dispatch.rs` for the real SB-01 test suite.

#[test]
fn scaffold_api_surface() {
    // parse / parse_bytes never panic and return a ParseResult. A genuine
    // `<beans/>` root now parses successfully (U3 wired the real pipeline).
    let r = beans_xml::parse("<beans/>");
    assert!(r.beans.is_some(), "a <beans/> root must parse successfully");
    assert!(r.diagnostics.is_empty());

    // is_beans_doc ↔ parse_bytes consistency (invariant #7).
    let bytes = b"<beans/>";
    assert_eq!(
        beans_xml::is_beans_doc(bytes),
        beans_xml::parse_bytes(bytes).beans.is_some()
    );
    assert!(beans_xml::is_beans_doc(bytes));

    // A non-<beans> root is rejected via NotBeansRoot, not a panic.
    let non_beans = beans_xml::parse("<project/>");
    assert!(non_beans.beans.is_none());
    assert_eq!(
        non_beans.diagnostics.first().map(|d| &d.code),
        Some(&beans_xml::DiagCode::NotBeansRoot)
    );

    // oversize input is rejected before decoding
    let big = vec![b' '; beans_xml::MAX_INPUT_BYTES + 1];
    let over = beans_xml::parse_bytes(&big);
    assert!(matches!(
        over.diagnostics.first().map(|d| &d.code),
        Some(beans_xml::DiagCode::OversizeInput)
    ));

    assert_eq!(beans_xml::DEPTH_LIMIT, 256);
    assert!(!beans_xml::version().is_empty());
}
