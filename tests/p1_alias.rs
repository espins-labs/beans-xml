//! Unit **P1** — `<alias name= alias=>` (SB-03), plus a confirming table
//! case for name-attribute multi-value tokenization into `Bean::names`
//! (already wired by U4's `split_name_tokens` — `tests/u4_bean_core.rs`
//! owns the exhaustive comma/semicolon/whitespace/id-exclusion coverage;
//! this file's one case is the spec's SB-03 edge-case-table row, not a
//! duplicate of U4's own suite). Test design per
//! the internal build plan's P1 row ("test: table (multiple aliases)")
//! plus the task's own three named cases: multiple aliases, an alias
//! pointing at a bean this file never defines (spec's "reference to a bean
//! in another file"), and a multi-token `name=` attribute.
//!
//! `dispatch::parse_alias` is `pub(crate)` — not visible from this
//! external integration-test binary — so every test here goes through the
//! public API (`beans_xml::parse`) only, the same convention
//! `tests/p3_import.rs`/`tests/u4_bean_core.rs` established.

use beans_xml::Alias;

fn parse_ok(source: &str) -> beans_xml::BeansFile {
    beans_xml::parse(source).beans.expect("beans root")
}

// ---------------------------------------------------------------------
// Table: <alias name= alias=>.
// ---------------------------------------------------------------------

#[test]
fn sb03_single_alias_is_collected() {
    let source = r#"<beans><alias name="dataSource" alias="ds"/></beans>"#;
    let beans = parse_ok(source);
    assert_eq!(beans.aliases.len(), 1);
    let alias = &beans.aliases[0].value;
    assert_eq!(alias.name.value, "dataSource");
    assert_eq!(alias.alias.value, "ds");
}

#[test]
fn sb03_multiple_aliases_are_all_collected_in_order() {
    let source = concat!(
        "<beans>",
        r#"<alias name="dataSource" alias="ds"/>"#,
        r#"<alias name="dataSource" alias="mainDataSource"/>"#,
        r#"<alias name="widgetService" alias="widgetSvc"/>"#,
        "</beans>"
    );
    let beans = parse_ok(source);
    assert_eq!(beans.aliases.len(), 3);
    let names: Vec<&str> = beans
        .aliases
        .iter()
        .map(|a| a.value.alias.value.as_str())
        .collect();
    assert_eq!(names, vec!["ds", "mainDataSource", "widgetSvc"]);
    // Two aliases pointing at the same target name are both preserved —
    // no dedup, no diagnostic (an alias fan-out is legal Spring config).
    // Checked per-index (not just the alias= side) so a mis-read `name=`
    // on a non-first alias — e.g. the third alias's "widgetService" —
    // would fail this assertion instead of slipping through unverified.
    let target_names: Vec<&str> = beans
        .aliases
        .iter()
        .map(|a| a.value.name.value.as_str())
        .collect();
    assert_eq!(
        target_names,
        vec!["dataSource", "dataSource", "widgetService"]
    );
}

#[test]
fn sb03_alias_points_at_a_bean_this_file_never_defines() {
    // Spec's named edge case: a `<beans>` assembled via `<import>` from
    // several files may alias a bean this file itself never declares — the
    // parser records the raw `name=` target verbatim and attempts no
    // existence check (cross-file resolution is a consumer's job, spec's
    // "references are raw only" policy). No `<bean id="dataSourceFromOtherFile">`
    // appears anywhere in this document.
    let source = concat!(
        "<beans>",
        r#"<import resource="classpath:com/example/other-context.xml"/>"#,
        r#"<alias name="dataSourceFromOtherFile" alias="ds"/>"#,
        "</beans>"
    );
    let beans = parse_ok(source);
    assert_eq!(beans.beans.len(), 0, "this file defines no <bean> at all");
    assert_eq!(beans.aliases.len(), 1);
    let alias = &beans.aliases[0].value;
    assert_eq!(alias.name.value, "dataSourceFromOtherFile");
    assert_eq!(alias.alias.value, "ds");
}

#[test]
fn sb03_missing_name_or_alias_attr_falls_back_to_empty_spanned_string() {
    // Malformed against the Spring XSD (which requires both), but this
    // crate never panics/errors on it (rule 4) — same infallible-fallback
    // policy `parse_import` documents for a missing `resource=`.
    let source = "<beans><alias alias=\"onlyAlias\"/></beans>";
    let beans = parse_ok(source);
    assert_eq!(beans.aliases.len(), 1);
    let spanned_alias = &beans.aliases[0];
    let alias = &spanned_alias.value;
    assert_eq!(alias.name.value, "");
    assert_eq!(alias.alias.value, "onlyAlias");
    // The missing attribute's fallback span is the whole `<alias>` element.
    assert_eq!(alias.name.span, spanned_alias.span);
}

#[test]
fn sb03_both_attrs_missing_falls_back_to_empty_spanned_strings_for_both() {
    // Doc comment's other absent-attribute shape: neither `name=` nor
    // `alias=` present at all (`<alias/>`, fully bare against the XSD).
    // Same infallible-fallback policy as the single-missing-attribute
    // case — no panic, no diagnostic, both sides fall back independently.
    let source = "<beans><alias/></beans>";
    let beans = parse_ok(source);
    assert_eq!(beans.aliases.len(), 1);
    let spanned_alias = &beans.aliases[0];
    let alias = &spanned_alias.value;
    assert_eq!(alias.name.value, "");
    assert_eq!(alias.alias.value, "");
    assert_eq!(alias.name.span, spanned_alias.span);
    assert_eq!(alias.alias.span, spanned_alias.span);
}

#[test]
fn sb03_present_but_empty_attrs_are_kept_empty_with_no_diagnostic() {
    // Doc comment's other case: the attribute is *present* with an empty
    // value (`name=""`/`alias=""`), likened to `resolve_scope`'s
    // present-but-empty `scope=` treatment — `Alias.name`/`Alias.alias`
    // have no "never empty" invariant to uphold, so this is kept as an
    // empty string with no diagnostic pushed, same as the fully-absent
    // case in outcome but distinct in span: since the attribute is
    // present, its span is the (empty) attribute-value span, not a
    // fallback to the whole element's span.
    let source = r#"<beans><alias name="" alias=""/></beans>"#;
    let result = beans_xml::parse(source);
    assert_eq!(result.diagnostics.len(), 0);
    let beans = result.beans.expect("beans root");
    assert_eq!(beans.aliases.len(), 1);
    let spanned_alias = &beans.aliases[0];
    let alias = &spanned_alias.value;
    assert_eq!(alias.name.value, "");
    assert_eq!(alias.alias.value, "");
}

#[test]
fn sb03_alias_span_covers_the_alias_element_and_attr_spans_are_the_attr_values() {
    let source = r#"<beans><alias name="dataSource" alias="ds"/></beans>"#;
    let beans = parse_ok(source);
    let spanned_alias = &beans.aliases[0];
    let slice = &source[spanned_alias.span.start as usize..spanned_alias.span.end as usize];
    assert_eq!(slice, r#"<alias name="dataSource" alias="ds"/>"#);

    let alias: &Alias = &spanned_alias.value;
    let name_slice = &source[alias.name.span.start as usize..alias.name.span.end as usize];
    assert_eq!(name_slice, "dataSource");
    let alias_slice = &source[alias.alias.span.start as usize..alias.alias.span.end as usize];
    assert_eq!(alias_slice, "ds");
}

// ---------------------------------------------------------------------
// Confirming case: name-attribute multi-value tokenization (already wired
// by U4 — `tests/u4_bean_core.rs` owns the exhaustive suite). This is the
// spec's SB-03 edge-case-table row, kept here so P1's own table is
// self-contained per the build plan.
// ---------------------------------------------------------------------

#[test]
fn sb03_multi_token_name_attr_splits_into_bean_names_excluding_id() {
    let source = concat!(
        "<beans>",
        r#"<bean id="widgetService" name="widgetSvc, legacyWidgetSvc;thirdAlias otherAlias" "#,
        r#"class="com.example.WidgetService"/>"#,
        "</beans>"
    );
    let beans = parse_ok(source);
    assert_eq!(beans.beans.len(), 1);
    let bean = &beans.beans[0];
    let tokens: Vec<&str> = bean.names.iter().map(|n| n.value.as_str()).collect();
    assert_eq!(
        tokens,
        vec!["widgetSvc", "legacyWidgetSvc", "thirdAlias", "otherAlias"]
    );
    // id is never duplicated into names (model contract, `Bean::names`'s
    // own doc comment).
    assert!(!bean.names.iter().any(|n| n.value == "widgetService"));
}
