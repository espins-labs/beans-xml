//! Unit **U5a** — `InjectValue` core (SB-06) tests.
//!
//! `inject_value::parse_inject_value_child`/`build_value_lit`/etc. are all
//! `pub(crate)` — a seam not visible from this external integration-test
//! binary, the same situation `tests/u1_events.rs` and
//! `tests/u2_encoding.rs` document for `events`/`encoding`. The real U5a
//! test suite — per-variant snapshots, the inner-bean structural snapshot,
//! and the depth/`DEPTH_LIMIT` proptests — lives there:
//! `src/inject_value.rs`'s own `#[cfg(test)] mod tests`.
//!
//! This file exists so the unit still has a `tests/<unit>.rs` file per this
//! crate's `AGENTS.md` naming convention, and to smoke-test the one thing
//! observable from *outside* the crate at this stage: since U6/U7
//! (`<property>`/`<constructor-arg>`, the units that will actually call
//! into `inject_value`) haven't landed yet, every SB-06-shaped element
//! (`<ref>`, `<idref>`, `<null/>`, a stray inner `<bean>`) appearing where
//! `bean::parse_bean`'s own dispatch doesn't yet recognize it still parses
//! without panicking — it just falls through to `UnknownElement` (a
//! beans-ns element `parse_bean` doesn't have a dispatch arm for yet) or
//! the decorator catch-all, rather than being silently dropped or crashing.

use beans_xml::DiagCode;

#[test]
fn u5a_public_entry_points_stay_panic_free_on_sb06_shaped_fixtures() {
    let fixtures: &[&str] = &[
        // <ref>/<idref>/<null> directly inside a <bean> — not yet
        // recognized by parse_bean's own dispatch (that's U6/U7's wiring),
        // so these currently surface as UnknownElement, not a crash.
        "<beans><bean id=\"a\" class=\"com.example.Widget\"><ref bean=\"b\"/></bean></beans>",
        "<beans><bean id=\"a\" class=\"com.example.Widget\"><idref bean=\"b\"/></bean></beans>",
        "<beans><bean id=\"a\" class=\"com.example.Widget\"><null/></bean></beans>",
        // A stray inner <bean> not wrapped in <property>/<constructor-arg>.
        "<beans><bean id=\"a\" class=\"com.example.Widget\"><bean class=\"com.example.Inner\"/></bean></beans>",
        // <value type="..."> stray content.
        "<beans><bean id=\"a\" class=\"com.example.Widget\"><value type=\"java.lang.Integer\">1</value></bean></beans>",
    ];
    for fixture in fixtures {
        let r1 = beans_xml::parse(fixture);
        assert!(r1.beans.is_some(), "fixture must still parse: {fixture}");

        let r2 = beans_xml::parse_bytes(fixture.as_bytes());
        assert_eq!(r2.beans.is_some(), r1.beans.is_some());

        // Every element name here is inside the beans namespace but not
        // (yet) a recognized child of <bean> — UnknownElement, not silently
        // dropped or a panic.
        assert!(
            r1.diagnostics
                .iter()
                .any(|d| d.code == DiagCode::UnknownElement),
            "expected UnknownElement for {fixture}: {:?}",
            r1.diagnostics
        );
    }
}

#[test]
fn u5a_depth_limit_constant_is_exported_and_matches_spec() {
    assert_eq!(beans_xml::DEPTH_LIMIT, 256);
}
