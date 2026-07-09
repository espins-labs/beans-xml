//! Unit **P4** — `context:component-scan` (SB-10). Test design per the
//! internal build plan's P4 row: a table covering multiple `base-package`
//! values, all four `ScanFilter` types (annotation/assignable/regex/
//! aspectj), and `use-default-filters="false"`.
//!
//! `dispatch::parse_component_scan` is `pub(crate)` — not visible from this
//! external integration-test binary — so every test here goes through the
//! public API (`beans_xml::parse`) only, the same convention
//! `tests/p3_import.rs` established.

use beans_xml::ComponentScan;

const CONTEXT_NS: &str = r#"xmlns:context="http://www.springframework.org/schema/context""#;

fn parse_ok(source: &str) -> beans_xml::BeansFile {
    beans_xml::parse(source).beans.expect("beans root")
}

/// Parses `source` (expected to declare exactly one top-level
/// `<context:component-scan>`) and returns that one `ComponentScan` (span
/// dropped — callers that care about span assert on the outer `Spanned`
/// directly).
fn only_scan(source: &str) -> ComponentScan {
    let beans = parse_ok(source);
    assert_eq!(
        beans.component_scans.len(),
        1,
        "expected exactly one <context:component-scan>"
    );
    beans.component_scans.into_iter().next().unwrap().value
}

// ---------------------------------------------------------------------
// base-package: single, multiple (comma/whitespace/semicolon-separated).
// ---------------------------------------------------------------------

#[test]
fn sb10_single_base_package() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example.scan"/></beans>"#
    );
    let scan = only_scan(&source);
    assert_eq!(scan.base_packages.len(), 1);
    assert_eq!(scan.base_packages[0].value, "com.example.scan");
}

#[test]
fn sb10_multiple_base_packages_comma_separated() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example.a,com.example.b"/></beans>"#
    );
    let scan = only_scan(&source);
    assert_eq!(scan.base_packages.len(), 2);
    assert_eq!(scan.base_packages[0].value, "com.example.a");
    assert_eq!(scan.base_packages[1].value, "com.example.b");
}

#[test]
fn sb10_multiple_base_packages_whitespace_and_semicolon_mixed() {
    // Spring's own ComponentScanBeanDefinitionParser tokenizes base-package
    // against ",; \t\n" — comma, semicolon, and whitespace all separate.
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example.a, com.example.b; com.example.c"/></beans>"#
    );
    let scan = only_scan(&source);
    assert_eq!(scan.base_packages.len(), 3);
    assert_eq!(scan.base_packages[0].value, "com.example.a");
    assert_eq!(scan.base_packages[1].value, "com.example.b");
    assert_eq!(scan.base_packages[2].value, "com.example.c");
}

#[test]
fn sb10_base_package_span_slices_to_its_own_token() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example.a,com.example.b"/></beans>"#
    );
    let scan = only_scan(&source);
    let second = &scan.base_packages[1];
    let slice = &source[second.span.start as usize..second.span.end as usize];
    assert_eq!(slice, "com.example.b");
}

#[test]
fn sb10_missing_base_package_attr_is_empty() {
    let source = format!(r#"<beans {CONTEXT_NS}><context:component-scan/></beans>"#);
    let scan = only_scan(&source);
    assert!(scan.base_packages.is_empty());
}

// ---------------------------------------------------------------------
// use-default-filters.
// ---------------------------------------------------------------------

#[test]
fn sb10_use_default_filters_false() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example" use-default-filters="false"/></beans>"#
    );
    let scan = only_scan(&source);
    assert_eq!(scan.use_default_filters, Some(false));
}

#[test]
fn sb10_use_default_filters_true_explicit() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example" use-default-filters="true"/></beans>"#
    );
    let scan = only_scan(&source);
    assert_eq!(scan.use_default_filters, Some(true));
}

#[test]
fn sb10_use_default_filters_absent_is_none() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example"/></beans>"#
    );
    let scan = only_scan(&source);
    assert_eq!(scan.use_default_filters, None);
}

// ---------------------------------------------------------------------
// Filter types: all four (annotation/assignable/regex/aspectj).
// ---------------------------------------------------------------------

#[test]
fn sb10_include_filter_annotation_type() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example">
        <context:include-filter type="annotation" expression="com.example.MyAnnotation"/>
        </context:component-scan></beans>"#
    );
    let scan = only_scan(&source);
    assert_eq!(scan.include_filters.len(), 1);
    assert_eq!(scan.include_filters[0].filter_type.value, "annotation");
    assert_eq!(
        scan.include_filters[0].expression.value,
        "com.example.MyAnnotation"
    );
    assert!(scan.exclude_filters.is_empty());
}

#[test]
fn sb10_exclude_filter_assignable_type() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example">
        <context:exclude-filter type="assignable" expression="com.example.BaseService"/>
        </context:component-scan></beans>"#
    );
    let scan = only_scan(&source);
    assert_eq!(scan.exclude_filters.len(), 1);
    assert_eq!(scan.exclude_filters[0].filter_type.value, "assignable");
    assert_eq!(
        scan.exclude_filters[0].expression.value,
        "com.example.BaseService"
    );
    assert!(scan.include_filters.is_empty());
}

#[test]
fn sb10_include_filter_regex_type() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example">
        <context:include-filter type="regex" expression="com\.example\..*Service"/>
        </context:component-scan></beans>"#
    );
    let scan = only_scan(&source);
    assert_eq!(scan.include_filters.len(), 1);
    assert_eq!(scan.include_filters[0].filter_type.value, "regex");
    assert_eq!(
        scan.include_filters[0].expression.value,
        r"com\.example\..*Service"
    );
}

#[test]
fn sb10_exclude_filter_aspectj_type() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example">
        <context:exclude-filter type="aspectj" expression="com.example..*Legacy+"/>
        </context:component-scan></beans>"#
    );
    let scan = only_scan(&source);
    assert_eq!(scan.exclude_filters.len(), 1);
    assert_eq!(scan.exclude_filters[0].filter_type.value, "aspectj");
    assert_eq!(
        scan.exclude_filters[0].expression.value,
        "com.example..*Legacy+"
    );
}

#[test]
fn sb10_mixed_include_and_exclude_filters_in_document_order() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example">
        <context:include-filter type="annotation" expression="com.example.Service"/>
        <context:exclude-filter type="regex" expression="com\.example\.Skip.*"/>
        <context:include-filter type="assignable" expression="com.example.BaseRepo"/>
        </context:component-scan></beans>"#
    );
    let scan = only_scan(&source);
    assert_eq!(scan.include_filters.len(), 2);
    assert_eq!(scan.exclude_filters.len(), 1);
    assert_eq!(scan.include_filters[0].filter_type.value, "annotation");
    assert_eq!(scan.include_filters[1].filter_type.value, "assignable");
    assert_eq!(scan.exclude_filters[0].filter_type.value, "regex");
}

// ---------------------------------------------------------------------
// Span sanity + multiple component-scan elements.
// ---------------------------------------------------------------------

#[test]
fn sb10_scan_span_covers_the_component_scan_element() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example"/></beans>"#
    );
    let beans = parse_ok(&source);
    let spanned_scan = &beans.component_scans[0];
    let slice = &source[spanned_scan.span.start as usize..spanned_scan.span.end as usize];
    assert_eq!(
        slice,
        r#"<context:component-scan base-package="com.example"/>"#
    );
}

#[test]
fn sb10_multiple_component_scan_elements_both_collected_in_order() {
    let source = format!(
        r#"<beans {CONTEXT_NS}>
        <context:component-scan base-package="com.example.a"/>
        <context:component-scan base-package="com.example.b"/>
        </beans>"#
    );
    let beans = parse_ok(&source);
    assert_eq!(beans.component_scans.len(), 2);
    assert_eq!(
        beans.component_scans[0].value.base_packages[0].value,
        "com.example.a"
    );
    assert_eq!(
        beans.component_scans[1].value.base_packages[0].value,
        "com.example.b"
    );
}

// ---------------------------------------------------------------------
// Documented skip/None edge policies, locked by tests.
// ---------------------------------------------------------------------

#[test]
fn sb10_use_default_filters_non_boolean_value_is_none() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example" use-default-filters="yes"/></beans>"#
    );
    let scan = only_scan(&source);
    assert_eq!(scan.use_default_filters, None);
}

#[test]
fn sb10_custom_filter_type_preserved_verbatim() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example">
        <context:include-filter type="custom" expression="com.example.MyTypeFilter"/>
        </context:component-scan></beans>"#
    );
    let scan = only_scan(&source);
    assert_eq!(scan.include_filters.len(), 1);
    assert_eq!(scan.include_filters[0].filter_type.value, "custom");
}

#[test]
fn sb10_empty_base_package_attr_is_empty() {
    let source =
        format!(r#"<beans {CONTEXT_NS}><context:component-scan base-package=""/></beans>"#);
    let scan = only_scan(&source);
    assert!(scan.base_packages.is_empty());
}

#[test]
fn sb10_non_context_namespace_child_is_skipped() {
    // A child element from an entirely different namespace inside
    // <context:component-scan> is silently skipped, not collected as a
    // filter of either kind.
    let source = format!(
        r#"<beans {CONTEXT_NS} xmlns:aop="http://www.springframework.org/schema/aop">
        <context:component-scan base-package="com.example">
        <aop:include-filter/>
        </context:component-scan></beans>"#
    );
    let scan = only_scan(&source);
    assert!(scan.include_filters.is_empty());
    assert!(scan.exclude_filters.is_empty());
}

#[test]
fn sb10_unrecognized_context_child_is_skipped() {
    // A context:* child that isn't include-filter/exclude-filter is
    // silently skipped rather than collected or diagnosed.
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example">
        <context:annotation-config/>
        </context:component-scan></beans>"#
    );
    let scan = only_scan(&source);
    assert!(scan.include_filters.is_empty());
    assert!(scan.exclude_filters.is_empty());
}

#[test]
fn sb10_include_filter_declaring_default_namespace_on_itself_is_still_collected() {
    // The filter child declares `context` as its own default namespace
    // instead of relying on the component-scan element's declaration —
    // legal per standard XML namespace scoping, and must resolve the same
    // as the root-declared control below.
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example">
        <include-filter xmlns="http://www.springframework.org/schema/context" type="regex" expression="com\.example\..*"/>
        </context:component-scan></beans>"#
    );
    let scan = only_scan(&source);
    assert_eq!(scan.include_filters.len(), 1);
    assert_eq!(scan.include_filters[0].filter_type.value, "regex");
}

#[test]
fn sb10_include_filter_declaring_own_prefix_is_still_collected() {
    // Same as above but with a self-declared prefix instead of a default
    // namespace.
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:component-scan base-package="com.example">
        <c2:include-filter xmlns:c2="http://www.springframework.org/schema/context" type="annotation" expression="com.example.MyAnnotation"/>
        </context:component-scan></beans>"#
    );
    let scan = only_scan(&source);
    assert_eq!(scan.include_filters.len(), 1);
    assert_eq!(scan.include_filters[0].filter_type.value, "annotation");
}
