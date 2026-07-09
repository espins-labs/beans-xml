//! Unit **U3** — root detection + `BeansFile` header (SB-01) + the frozen
//! root-child dispatch skeleton. Test design per
//! the internal build plan's U3 row:
//! (a) root-detect table (`beans`/`pom.xml`/`web.xml`/`sqlMap`-shaped roots),
//! (b) invariant #7 proptest (`is_beans_doc(b) == parse_bytes(b).beans.is_some()`,
//!     including the `=false` branch over non-beans roots and junk input),
//! (c) `default-*` table.
//!
//! `dispatch::is_beans_root`/`NsScope`/`parse_beans_body` and the six
//! handler-fn stubs are all `pub(crate)` — not visible from this external
//! integration-test binary — so every test here goes through the public
//! API (`parse`/`parse_bytes`/`is_beans_doc`) only, which is exactly
//! SB-01's own public contract.

use beans_xml::DiagCode;

// ---------------------------------------------------------------------
// (a) Root-detect table.
// ---------------------------------------------------------------------

#[test]
fn sb01_beans_root_is_recognized() {
    let r = beans_xml::parse("<beans><bean id=\"a\"/></beans>");
    assert!(r.beans.is_some());
    assert!(
        !r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::NotBeansRoot),
        "a genuine <beans> root must not produce NotBeansRoot: {:?}",
        r.diagnostics
    );
    assert!(beans_xml::is_beans_doc(b"<beans><bean id=\"a\"/></beans>"));
}

#[test]
fn sb01_self_closed_beans_root_is_recognized() {
    let r = beans_xml::parse("<beans/>");
    assert!(r.beans.is_some());
    assert!(beans_xml::is_beans_doc(b"<beans/>"));
}

#[test]
fn sb01_pom_xml_shaped_root_is_not_beans() {
    // Representative synthetic shape of a Maven pom.xml root — not a real
    // pom.xml, just the same root element name (spec's fixture policy:
    // patterns learned, content synthetic).
    let source = r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
        <modelVersion>4.0.0</modelVersion>
    </project>"#;
    let r = beans_xml::parse(source);
    assert!(r.beans.is_none());
    assert_eq!(
        r.diagnostics.first().map(|d| &d.code),
        Some(&DiagCode::NotBeansRoot)
    );
    assert!(!beans_xml::is_beans_doc(source.as_bytes()));
}

#[test]
fn sb01_web_xml_shaped_root_is_not_beans() {
    let source = r#"<web-app xmlns="http://java.sun.com/xml/ns/j2ee" version="2.4">
        <display-name>example</display-name>
    </web-app>"#;
    let r = beans_xml::parse(source);
    assert!(r.beans.is_none());
    assert_eq!(
        r.diagnostics.first().map(|d| &d.code),
        Some(&DiagCode::NotBeansRoot)
    );
    assert!(!beans_xml::is_beans_doc(source.as_bytes()));
}

#[test]
fn sb01_sqlmap_shaped_root_is_not_beans() {
    // Representative synthetic shape of a MyBatis sqlMap root.
    let source = r#"<sqlMap namespace="com.example.WidgetMapper">
        <select id="selectWidget">select 1</select>
    </sqlMap>"#;
    let r = beans_xml::parse(source);
    assert!(r.beans.is_none());
    assert_eq!(
        r.diagnostics.first().map(|d| &d.code),
        Some(&DiagCode::NotBeansRoot)
    );
    assert!(!beans_xml::is_beans_doc(source.as_bytes()));
}

#[test]
fn sb01_no_root_element_at_all_is_not_beans() {
    let r = beans_xml::parse("just some text, no markup at all");
    assert!(r.beans.is_none());
    assert_eq!(
        r.diagnostics.first().map(|d| &d.code),
        Some(&DiagCode::NotBeansRoot)
    );
    assert!(!beans_xml::is_beans_doc(
        b"just some text, no markup at all"
    ));
}

#[test]
fn sb01_beans_root_with_namespace_prefix_is_still_recognized() {
    // "NS prefix present or absent" edge case (spec SB-01 row): the beans root element
    // itself carries an explicit namespace prefix, bound to the genuine
    // Spring beans schema URI, instead of the far more common unprefixed
    // default-namespace form.
    let source = concat!(
        "<spring:beans xmlns:spring=\"http://www.springframework.org/schema/beans\">",
        "<spring:bean id=\"a\" class=\"com.example.Widget\"/>",
        "</spring:beans>"
    );
    let r = beans_xml::parse(source);
    assert!(r.beans.is_some(), "prefixed beans root must still parse");
    assert!(beans_xml::is_beans_doc(source.as_bytes()));
}

#[test]
fn sb01_beans_local_name_under_an_unrelated_namespace_is_not_beans() {
    // The mirror image of the previous case: a "beans"-named local element
    // whose declared namespace is genuinely something else entirely must
    // *not* count, even though the bare local name matches.
    let source = r#"<foo:beans xmlns:foo="urn:not-spring-at-all"/>"#;
    let r = beans_xml::parse(source);
    assert!(r.beans.is_none());
    assert!(!beans_xml::is_beans_doc(source.as_bytes()));
}

#[test]
fn sb01_doctype_and_comment_preceding_root_do_not_defeat_detection() {
    let source = "<!-- a leading comment --><beans><bean id=\"a\"/></beans>";
    let r = beans_xml::parse(source);
    assert!(r.beans.is_some());
    assert!(beans_xml::is_beans_doc(source.as_bytes()));
}

#[test]
fn sb01_bom_preceding_root_does_not_defeat_detection() {
    let mut bytes = vec![0xEFu8, 0xBB, 0xBF];
    bytes.extend_from_slice(b"<beans><bean id=\"a\"/></beans>");
    let r = beans_xml::parse_bytes(&bytes);
    assert!(r.beans.is_some());
    assert!(beans_xml::is_beans_doc(&bytes));
}

// ---------------------------------------------------------------------
// NotBeansRoot diagnostic shape.
// ---------------------------------------------------------------------

#[test]
fn sb01_not_beans_root_diagnostic_carries_the_root_span() {
    let source = "<project/>";
    let r = beans_xml::parse(source);
    assert!(r.beans.is_none());
    let diag = r
        .diagnostics
        .iter()
        .find(|d| d.code == DiagCode::NotBeansRoot)
        .expect("NotBeansRoot diagnostic present");
    assert_eq!(diag.span, Some(beans_xml::ByteSpan { start: 0, end: 10 }));
}

// ---------------------------------------------------------------------
// (b) Invariant #7: is_beans_doc(b) == parse_bytes(b).beans.is_some().
// ---------------------------------------------------------------------

#[test]
fn sb01_invariant_7_holds_on_a_beans_root() {
    let bytes = b"<beans><bean id=\"a\"/></beans>";
    assert_eq!(
        beans_xml::is_beans_doc(bytes),
        beans_xml::parse_bytes(bytes).beans.is_some()
    );
}

#[test]
fn sb01_invariant_7_holds_on_non_beans_roots_and_junk() {
    let fixtures: &[&[u8]] = &[
        b"<project/>",
        b"<web-app/>",
        b"<sqlMap/>",
        b"<foo/>",
        b"not even xml",
        b"",
        b"<>",
        b"<beans",
    ];
    for bytes in fixtures {
        assert_eq!(
            beans_xml::is_beans_doc(bytes),
            beans_xml::parse_bytes(bytes).beans.is_some(),
            "invariant #7 broke for fixture {:?}",
            String::from_utf8_lossy(bytes)
        );
    }
}

#[test]
fn sb01_invariant_7_oversize_input_agrees_via_false_and_none() {
    let big = vec![b' '; beans_xml::MAX_INPUT_BYTES + 1];
    assert!(!beans_xml::is_beans_doc(&big));
    assert!(beans_xml::parse_bytes(&big).beans.is_none());
    assert_eq!(
        beans_xml::is_beans_doc(&big),
        beans_xml::parse_bytes(&big).beans.is_some()
    );
}

proptest::proptest! {
    #[test]
    fn sb01_invariant_7_proptest_arbitrary_bytes(
        bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..500)
    ) {
        let lhs = beans_xml::is_beans_doc(&bytes);
        let rhs = beans_xml::parse_bytes(&bytes).beans.is_some();
        proptest::prop_assert_eq!(lhs, rhs);
    }

    #[test]
    fn sb01_invariant_7_proptest_non_beans_root_shapes(
        root_name in "[a-zA-Z][a-zA-Z0-9_-]{0,15}",
        self_closed in proptest::prelude::any::<bool>(),
    ) {
        // A generator biased toward non-`beans` root names (`beans` itself
        // is already covered by the dedicated positive tests above) plus
        // pom.xml/web.xml-style shapes — the `=false` branch of invariant
        // #7 build plan explicitly calls out.
        let source = if self_closed {
            format!("<{root_name}/>")
        } else {
            format!("<{root_name}><child/></{root_name}>")
        };
        let bytes = source.as_bytes();
        let is_doc = beans_xml::is_beans_doc(bytes);
        let parsed_some = beans_xml::parse_bytes(bytes).beans.is_some();
        proptest::prop_assert_eq!(is_doc, parsed_some);
        if root_name != "beans" {
            proptest::prop_assert!(!is_doc, "root name {:?} must not be treated as beans", root_name);
        }
    }

    #[test]
    fn sb01_invariant_7_proptest_arbitrary_unicode_str(s in ".{0,300}") {
        let bytes = s.as_bytes();
        let lhs = beans_xml::is_beans_doc(bytes);
        let rhs = beans_xml::parse_bytes(bytes).beans.is_some();
        proptest::prop_assert_eq!(lhs, rhs);
    }
}

// ---------------------------------------------------------------------
// (c) default-* table.
// ---------------------------------------------------------------------

#[test]
fn sb01_default_attrs_all_present_are_read() {
    let source = concat!(
        "<beans profile=\"dev,test\" default-lazy-init=\"true\" ",
        "default-autowire=\"byName\" default-init-method=\"init\" ",
        "default-destroy-method=\"destroy\" default-merge=\"false\" ",
        "default-autowire-candidates=\"*Service\">",
        "</beans>"
    );
    let beans = beans_xml::parse(source).beans.expect("beans root");
    assert_eq!(
        beans.profile.as_ref().map(|s| s.value.as_str()),
        Some("dev,test")
    );
    assert_eq!(beans.default_lazy_init, Some(true));
    assert_eq!(
        beans.default_autowire.as_ref().map(|s| s.value.as_str()),
        Some("byName")
    );
    assert_eq!(
        beans.default_init_method.as_ref().map(|s| s.value.as_str()),
        Some("init")
    );
    assert_eq!(
        beans
            .default_destroy_method
            .as_ref()
            .map(|s| s.value.as_str()),
        Some("destroy")
    );
    assert_eq!(beans.default_merge, Some(false));
    assert_eq!(
        beans
            .default_autowire_candidates
            .as_ref()
            .map(|s| s.value.as_str()),
        Some("*Service")
    );
}

#[test]
fn sb01_default_attrs_all_absent_are_none() {
    let beans = beans_xml::parse("<beans></beans>")
        .beans
        .expect("beans root");
    assert_eq!(beans.profile, None);
    assert_eq!(beans.default_lazy_init, None);
    assert_eq!(beans.default_autowire, None);
    assert_eq!(beans.default_init_method, None);
    assert_eq!(beans.default_destroy_method, None);
    assert_eq!(beans.default_merge, None);
    assert_eq!(beans.default_autowire_candidates, None);
}

#[test]
fn sb01_default_lazy_init_false_is_distinct_from_absent() {
    let beans = beans_xml::parse("<beans default-lazy-init=\"false\"></beans>")
        .beans
        .expect("beans root");
    assert_eq!(beans.default_lazy_init, Some(false));
}

#[test]
fn sb01_default_init_method_explicit_empty_string_is_some_empty_not_none() {
    // Model contract (BeansFile::default_init_method doc comment): an
    // explicit empty-string attribute (suppresses inheritance) is kept
    // distinct from the attribute being absent entirely.
    let beans = beans_xml::parse("<beans default-init-method=\"\"></beans>")
        .beans
        .expect("beans root");
    assert_eq!(
        beans.default_init_method.as_ref().map(|s| s.value.as_str()),
        Some("")
    );
}

#[test]
fn sb01_default_lazy_init_unrecognized_value_is_none() {
    // The XSD-legal "default" literal (or any other unrecognized value)
    // is not confidently true or false — treated the same as absent.
    let beans = beans_xml::parse("<beans default-lazy-init=\"default\"></beans>")
        .beans
        .expect("beans root");
    assert_eq!(beans.default_lazy_init, None);
}

#[test]
fn sb01_profile_attr_is_kept_raw_unsplit() {
    let beans = beans_xml::parse("<beans profile=\"dev, !prod , test\"></beans>")
        .beans
        .expect("beans root");
    assert_eq!(
        beans.profile.as_ref().map(|s| s.value.as_str()),
        Some("dev, !prod , test")
    );
}

// ---------------------------------------------------------------------
// `<description>` header child (SB-01 scope alongside profile/default-*).
// ---------------------------------------------------------------------

#[test]
fn sb01_description_child_text_is_captured() {
    let source = "<beans><description>Widget module wiring.</description></beans>";
    let beans = beans_xml::parse(source).beans.expect("beans root");
    assert_eq!(
        beans.description.as_ref().map(|s| s.value.as_str()),
        Some("Widget module wiring.")
    );
}

#[test]
fn sb01_description_span_slices_back_to_the_text() {
    let source = "<beans><description>hello</description></beans>";
    let beans = beans_xml::parse(source).beans.expect("beans root");
    let desc = beans.description.expect("description present");
    assert_eq!(
        &source[desc.span.start as usize..desc.span.end as usize],
        "hello"
    );
}

#[test]
fn sb01_absent_description_is_none() {
    let beans = beans_xml::parse("<beans></beans>")
        .beans
        .expect("beans root");
    assert_eq!(beans.description, None);
}

// ---------------------------------------------------------------------
// BeansFile.span covers the whole document (subtree start/end).
// ---------------------------------------------------------------------

#[test]
fn sb01_beans_span_covers_the_full_document() {
    let source = "<beans><bean id=\"a\"/></beans>";
    let beans = beans_xml::parse(source).beans.expect("beans root");
    assert_eq!(
        beans.span,
        beans_xml::ByteSpan {
            start: 0,
            end: source.len() as u32,
        }
    );
}

// ---------------------------------------------------------------------
// Dispatch skeleton sanity: an unrecognized element inside the beans
// namespace itself is UnknownElement, not silently dropped or mistaken
// for a NamespacedElement. A genuinely different namespace does not
// trigger UnknownElement (it's the NamespacedElement leaf's territory,
// P7 — this only pins that U3's own catch-all path doesn't misfire).
// ---------------------------------------------------------------------

#[test]
fn sb01_unrecognized_beans_ns_child_is_unknown_element() {
    let source = "<beans><totally-made-up/></beans>";
    let r = beans_xml::parse(source);
    assert!(r.beans.is_some());
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::UnknownElement),
        "expected UnknownElement for an unrecognized beans-ns child: {:?}",
        r.diagnostics
    );
}

#[test]
fn sb01_foreign_namespace_child_is_not_unknown_element() {
    let source = concat!(
        "<beans xmlns:aop=\"http://www.springframework.org/schema/aop\">",
        "<aop:config/>",
        "</beans>"
    );
    let r = beans_xml::parse(source);
    assert!(r.beans.is_some());
    assert!(
        !r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::UnknownElement),
        "a foreign namespace element must not be flagged UnknownElement: {:?}",
        r.diagnostics
    );
}

// ---------------------------------------------------------------------
// Child-declared xmlns: a child element carrying its own `xmlns`/
// `xmlns:*` declaration (rather than inheriting one from the root) must
// be resolved using that declaration, exactly like the root itself
// (`is_beans_root`) already is.
// ---------------------------------------------------------------------

#[test]
fn sb01_child_declared_beans_ns_unrecognized_element_is_unknown_element() {
    // The prefix `spring` is undeclared at the root; it's declared on the
    // child itself. Per standard XML namespace scoping this still resolves
    // to the real beans URI, so the unrecognized local name must surface
    // as UnknownElement, not be silently dropped.
    let source = concat!(
        "<beans><spring:totally-made-up ",
        "xmlns:spring=\"http://www.springframework.org/schema/beans\"/>",
        "</beans>"
    );
    let r = beans_xml::parse(source);
    assert!(r.beans.is_some());
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::UnknownElement),
        "a child-declared beans-ns unrecognized element must be UnknownElement: {:?}",
        r.diagnostics
    );
}

#[test]
fn sb01_child_declared_foreign_default_ns_bean_is_not_treated_as_beans_bean() {
    // A child that redeclares the default `xmlns` to a foreign URI right
    // on itself must not be routed as a first-class beans `<bean>` (which
    // would silently reserve it and produce no diagnostic at all) even
    // though its bare local name is "bean".
    let source = r#"<beans><bean xmlns="urn:x-not-spring" id="a"/></beans>"#;
    let r = beans_xml::parse(source);
    assert!(r.beans.is_some());
    assert!(
        !r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::UnknownElement),
        "a foreign-namespace element must fall to NamespacedElement, not UnknownElement: {:?}",
        r.diagnostics
    );
}

// ---------------------------------------------------------------------
// `<bean>` reservation: a plain first-class `<bean>` child must not
// produce an UnknownElement diagnostic (it's reserved in the dispatch
// match for U4, not routed through the beans-ns catch-all arm).
// ---------------------------------------------------------------------

#[test]
fn sb01_plain_bean_child_produces_no_unknown_element_diagnostic() {
    let source = "<beans><bean id=\"a\" class=\"com.example.Widget\"/></beans>";
    let r = beans_xml::parse(source);
    assert!(r.beans.is_some());
    assert!(
        !r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::UnknownElement),
        "a plain first-class <bean> must not be UnknownElement: {:?}",
        r.diagnostics
    );
}

// ---------------------------------------------------------------------
// First-class context:/util: routing: exercised directly (not just via a
// foreign-namespace negative test) so a regression that dropped these
// arms back into the NamespacedElement catch-all would be caught here.
// ---------------------------------------------------------------------

#[test]
fn sb01_context_component_scan_child_produces_no_unknown_element_diagnostic() {
    let source = concat!(
        "<beans xmlns:context=\"http://www.springframework.org/schema/context\">",
        "<context:component-scan base-package=\"com.example\"/>",
        "</beans>"
    );
    let r = beans_xml::parse(source);
    assert!(r.beans.is_some());
    assert!(
        r.diagnostics.is_empty(),
        "a first-class context:component-scan must not raise diagnostics: {:?}",
        r.diagnostics
    );
}

#[test]
fn sb01_util_properties_child_produces_no_unknown_element_diagnostic() {
    let source = concat!(
        "<beans xmlns:util=\"http://www.springframework.org/schema/util\">",
        "<util:properties id=\"props\"/>",
        "</beans>"
    );
    let r = beans_xml::parse(source);
    assert!(r.beans.is_some());
    assert!(
        r.diagnostics.is_empty(),
        "a first-class util:properties must not raise diagnostics: {:?}",
        r.diagnostics
    );
}
