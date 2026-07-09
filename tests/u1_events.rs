//! Unit **U1** — event/recovery layer tests.
//!
//! Every type and function this unit introduces (`XmlElement`, `XmlNode`,
//! `XmlAttr`, `TreeResult`, `build_tree`, and the recovery-rule helpers in
//! `src/events.rs`) is `pub(crate)` — a seam not visible from this
//! external integration-test binary, the same situation
//! `tests/u0_model.rs` documents for `BeansFileCtx`/`BeanCtx`
//! (`src/model.rs`'s own `#[cfg(test)] mod tests` doc comment). The real
//! U1 test suite — the recovery-rule table, the span-slice-equals-
//! event-text table, and the panic-free proptests — lives there:
//! `src/events.rs`'s own `#[cfg(test)] mod tests`.
//!
//! This file exists so the unit still has a `tests/<unit>.rs` file per
//! this crate's `AGENTS.md` naming convention, and to smoke-test the one
//! thing observable from *outside* the crate: the public entry points
//! stay panic-free on the exact recovery-rule fixtures U1 introduces.
//!
//! Updated for U3 (root + header + dispatch skeleton landed, per this
//! file's own previous doc comment: "forcing an intentional update the
//! moment U3 wires `parse`/`parse_bytes` up to `build_tree`"): now that
//! `parse`/`parse_bytes` do call `build_tree`, the exact per-fixture
//! diagnostic shape is a function of each fixture's own root element name
//! (some are `<beans>`-rooted, some aren't) crossed with whatever
//! recovery diagnostics that fixture's malformed markup produces — that
//! full cross-product is already covered by `src/events.rs`'s own
//! recovery-rule table (root-agnostic) and `tests/u3_root_dispatch.rs`'s
//! root-detect table (recovery-agnostic). This file goes back to being a
//! pure panic-free smoke test rather than re-deriving that cross-product.

#[test]
fn u1_public_entry_points_stay_panic_free_on_recovery_rule_fixtures() {
    let fixtures: &[&str] = &[
        "<beans><bean id=\"a\"></beans>",      // unclosed tag
        "<beans></foo></beans>",               // orphan close tag
        "<bean id=\"a\" id=\"b\"/>",           // duplicate attribute
        "<beans><bean/><!zzz><bean/></beans>", // non-XML residue
        "<value>&badentity;</value>",          // unresolved entity
        "<value>${unterminated</value>",       // unterminated ${}
        "<value>#{unterminated</value>",       // unterminated #{}
    ];
    for fixture in fixtures {
        let r1 = beans_xml::parse(fixture);
        assert_eq!(r1.encoding.as_deref(), Some("UTF-8"));

        let r2 = beans_xml::parse_bytes(fixture.as_bytes());
        assert_eq!(r2.beans.is_some(), r1.beans.is_some());
        assert!(r2.encoding.is_some());
    }
}
