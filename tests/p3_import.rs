//! Unit **P3** — `<import resource=...>` (SB-09). Test design per
//! the internal build plan's P3 row: a table covering every
//! [`ImportKind`] variant, plus the spec's named edge cases — a general
//! `classpath*:` glob, `file:`, and a `${}` placeholder preserved raw
//! (never evaluated) inside the resource string.
//!
//! `dispatch::parse_import` is `pub(crate)` — not visible from this
//! external integration-test binary — so every test here goes through the
//! public API (`beans_xml::parse`) only, the same convention
//! `tests/u4_bean_core.rs`/`tests/u6_property.rs` established.

use beans_xml::{Import, ImportKind};

fn parse_ok(source: &str) -> beans_xml::BeansFile {
    beans_xml::parse(source).beans.expect("beans root")
}

/// Parses `source` (expected to declare exactly one top-level `<import>`)
/// and returns that one `Import` (span dropped — callers that care about
/// span assert on the outer `Spanned` directly).
fn only_import(source: &str) -> Import {
    let beans = parse_ok(source);
    assert_eq!(beans.imports.len(), 1, "expected exactly one <import>");
    beans.imports.into_iter().next().unwrap().value
}

// ---------------------------------------------------------------------
// Table: one case per ImportKind variant.
// ---------------------------------------------------------------------

#[test]
fn sb09_classpath_prefix_is_classpath_kind() {
    let import =
        only_import(r#"<beans><import resource="classpath:com/example/other.xml"/></beans>"#);
    assert_eq!(import.kind, ImportKind::Classpath);
    assert_eq!(import.resource.value, "classpath:com/example/other.xml");
}

#[test]
fn sb09_classpath_star_prefix_is_classpath_star_kind() {
    let import =
        only_import(r#"<beans><import resource="classpath*:com/example/other.xml"/></beans>"#);
    assert_eq!(import.kind, ImportKind::ClasspathStar);
    assert_eq!(import.resource.value, "classpath*:com/example/other.xml");
}

#[test]
fn sb09_bare_relative_path_is_relative_kind() {
    let import = only_import(r#"<beans><import resource="services-context.xml"/></beans>"#);
    assert_eq!(import.kind, ImportKind::Relative);
    assert_eq!(import.resource.value, "services-context.xml");
}

#[test]
fn sb09_relative_path_with_parent_dir_segments_is_relative_kind() {
    let import = only_import(r#"<beans><import resource="../config/other-context.xml"/></beans>"#);
    assert_eq!(import.kind, ImportKind::Relative);
    assert_eq!(import.resource.value, "../config/other-context.xml");
}

#[test]
fn sb09_http_url_is_url_kind() {
    let import =
        only_import(r#"<beans><import resource="http://example.com/context.xml"/></beans>"#);
    assert_eq!(import.kind, ImportKind::Url);
    assert_eq!(import.resource.value, "http://example.com/context.xml");
}

#[test]
fn sb09_missing_resource_attr_is_other_kind() {
    // No `resource=` at all — malformed against the Spring XSD (which
    // requires it), but this crate never panics/errors on it (rule 4):
    // infallible empty-string fallback, classified as the "matches none
    // of the recognized shapes" total fallback.
    let source = "<beans><import/></beans>";
    let beans = parse_ok(source);
    let spanned_import = &beans.imports[0];
    let import = &spanned_import.value;
    assert_eq!(import.kind, ImportKind::Other);
    assert_eq!(import.resource.value, "");
    // Documented fallback: the empty spanned string carries `element`'s
    // own span (no `resource=` attribute to take a span from), same as
    // `parse_import`'s doc comment describes.
    assert_eq!(import.resource.span, spanned_import.span);
}

#[test]
fn sb09_explicit_empty_resource_attr_is_other_kind() {
    // `resource=""` present but empty — a different span source than the
    // missing-attribute fallback above (the attribute's own value span,
    // not `element.span`), reaching the same `ImportKind::Other`.
    let source = r#"<beans><import resource=""/></beans>"#;
    let beans = parse_ok(source);
    let spanned_import = &beans.imports[0];
    let import = &spanned_import.value;
    assert_eq!(import.kind, ImportKind::Other);
    assert_eq!(import.resource.value, "");
    // The attribute's own (empty) value span sits strictly inside the
    // `<import .../>` element span, not equal to it.
    assert!(import.resource.span.start >= spanned_import.span.start);
    assert!(import.resource.span.end <= spanned_import.span.end);
    assert_ne!(import.resource.span, spanned_import.span);
}

// ---------------------------------------------------------------------
// Spec-named edge cases.
// ---------------------------------------------------------------------

#[test]
fn sb09_classpath_star_general_glob_is_preserved_raw() {
    let import = only_import(
        r#"<beans><import resource="classpath*:com/example/**/context-*.xml"/></beans>"#,
    );
    assert_eq!(import.kind, ImportKind::ClasspathStar);
    assert_eq!(
        import.resource.value,
        "classpath*:com/example/**/context-*.xml"
    );
}

#[test]
fn sb09_url_scheme_accepts_extended_scheme_chars() {
    // `has_url_scheme`'s own RFC 3986 scheme grammar accepts `+`/`-`/`.`
    // after the leading letter, not just alphanumerics — a real-world
    // scheme like `git+ssh:`/`view-source:` must still classify as `Url`,
    // not fall through to `Relative` for lack of exercising those extra
    // characters.
    let import =
        only_import(r#"<beans><import resource="x+y-z.w://example.com/repo.xml"/></beans>"#);
    assert_eq!(import.kind, ImportKind::Url);
    assert_eq!(import.resource.value, "x+y-z.w://example.com/repo.xml");
}

#[test]
fn sb09_file_scheme_is_url_kind() {
    let import = only_import(r#"<beans><import resource="file:/etc/app/context.xml"/></beans>"#);
    assert_eq!(import.kind, ImportKind::Url);
    assert_eq!(import.resource.value, "file:/etc/app/context.xml");
}

#[test]
fn sb09_placeholder_inside_classpath_resource_is_preserved_raw() {
    // `${}` is collected/preserved, never evaluated (spec non-goal) — the
    // placeholder text lands verbatim inside `resource.value`, and plays
    // no part in classification (the `classpath:` prefix still wins).
    let import = only_import(r#"<beans><import resource="classpath:${env}/context.xml"/></beans>"#);
    assert_eq!(import.kind, ImportKind::Classpath);
    assert_eq!(import.resource.value, "classpath:${env}/context.xml");
}

#[test]
fn sb09_placeholder_inside_relative_resource_is_preserved_raw() {
    let import = only_import(r#"<beans><import resource="${config.dir}/services.xml"/></beans>"#);
    assert_eq!(import.kind, ImportKind::Relative);
    assert_eq!(import.resource.value, "${config.dir}/services.xml");
}

// ---------------------------------------------------------------------
// Multiple imports + span sanity.
// ---------------------------------------------------------------------

#[test]
fn sb09_multiple_imports_are_all_collected_in_order() {
    let source = concat!(
        "<beans>",
        "<import resource=\"classpath:a.xml\"/>",
        "<import resource=\"classpath*:b/**/c.xml\"/>",
        "<import resource=\"rel.xml\"/>",
        "</beans>"
    );
    let beans = parse_ok(source);
    assert_eq!(beans.imports.len(), 3);
    assert_eq!(beans.imports[0].value.kind, ImportKind::Classpath);
    assert_eq!(beans.imports[1].value.kind, ImportKind::ClasspathStar);
    assert_eq!(beans.imports[2].value.kind, ImportKind::Relative);
}

#[test]
fn sb09_import_span_covers_the_import_element_and_resource_span_is_the_attr_value() {
    let source = r#"<beans><import resource="classpath:a.xml"/></beans>"#;
    let beans = parse_ok(source);
    let spanned_import = &beans.imports[0];
    let slice = &source[spanned_import.span.start as usize..spanned_import.span.end as usize];
    assert_eq!(slice, r#"<import resource="classpath:a.xml"/>"#);

    let resource = &spanned_import.value.resource;
    let resource_slice = &source[resource.span.start as usize..resource.span.end as usize];
    assert_eq!(resource_slice, "classpath:a.xml");
}
