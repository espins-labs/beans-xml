//! Unit **U6** — `<property>` (SB-04): wraps `InjectValue` with `name` +
//! `<meta>`. Test design per the internal build plan's U6 row:
//! snapshot + `value`/`ref` both specified → `ConflictingValueAndRef` +
//! `<meta>`.
//!
//! `bean::parse_bean`/`bean::BeanFrame`/`property::finish_property` are
//! `pub(crate)` — not visible from this external integration-test binary —
//! so every test here goes through the public API (`beans_xml::parse`) only,
//! the same convention `tests/u4_bean_core.rs` established.

use beans_xml::{DiagCode, InjectValue, Property, RefKind};

fn parse_ok(source: &str) -> beans_xml::BeansFile {
    beans_xml::parse(source).beans.expect("beans root")
}

fn only_property(source: &str) -> Property {
    let beans = parse_ok(source);
    assert_eq!(
        beans.beans.len(),
        1,
        "expected exactly one top-level <bean>"
    );
    let mut properties = beans.beans.into_iter().next().unwrap().properties;
    assert_eq!(properties.len(), 1, "expected exactly one <property>");
    properties.remove(0)
}

// ---------------------------------------------------------------------
// Snapshot: a representative <property> exercising the value= shorthand.
// ---------------------------------------------------------------------

#[test]
fn sb04_value_attr_shorthand_snapshot() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"label\" value=\"hello\"/>",
        "</bean></beans>"
    );
    let property = only_property(source);
    assert_eq!(property.name.value, "label");
    match &property.value {
        InjectValue::Value(vl) => assert_eq!(vl.text.value, "hello"),
        other => panic!("expected Value, got {other:?}"),
    }
    assert!(property.meta.is_empty());

    let r = beans_xml::parse(source);
    assert!(
        r.diagnostics.is_empty(),
        "a well-formed value= property must not raise diagnostics: {:?}",
        r.diagnostics
    );
}

// ---------------------------------------------------------------------
// Snapshot: ref= shorthand.
// ---------------------------------------------------------------------

#[test]
fn sb04_ref_attr_shorthand_snapshot() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"collaborator\" ref=\"otherBean\"/>",
        "</bean></beans>"
    );
    let property = only_property(source);
    assert_eq!(property.name.value, "collaborator");
    match &property.value {
        InjectValue::Ref(r) => {
            assert_eq!(r.value.raw, "otherBean");
            assert_eq!(r.value.kind, RefKind::Bean);
        }
        other => panic!("expected Ref, got {other:?}"),
    }

    let r = beans_xml::parse(source);
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
}

// ---------------------------------------------------------------------
// Nested child forms: <value type="...">, <ref bean=...>, inner <bean>.
// ---------------------------------------------------------------------

#[test]
fn sb04_value_child_with_type_snapshot() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"count\"><value type=\"java.lang.Integer\">42</value></property>",
        "</bean></beans>"
    );
    let property = only_property(source);
    match &property.value {
        InjectValue::Value(vl) => {
            assert_eq!(vl.text.value, "42");
            assert_eq!(
                vl.value_type.as_ref().map(|t| t.value.raw.as_str()),
                Some("java.lang.Integer")
            );
        }
        other => panic!("expected Value, got {other:?}"),
    }
    let r = beans_xml::parse(source);
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
}

#[test]
fn sb04_ref_child_element_snapshot() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"collaborator\"><ref bean=\"otherBean\"/></property>",
        "</bean></beans>"
    );
    let property = only_property(source);
    match &property.value {
        InjectValue::Ref(r) => assert_eq!(r.value.raw, "otherBean"),
        other => panic!("expected Ref, got {other:?}"),
    }
}

#[test]
fn sb04_nested_inner_bean_snapshot() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"inner\">",
        "<bean id=\"innerBean\" class=\"com.example.Gadget\"/>",
        "</property>",
        "</bean></beans>"
    );
    let property = only_property(source);
    match &property.value {
        InjectValue::Inner(bean) => {
            assert_eq!(
                bean.class.as_ref().map(|c| c.value.raw.as_str()),
                Some("com.example.Gadget")
            );
        }
        other => panic!("expected Inner, got {other:?}"),
    }
    let r = beans_xml::parse(source);
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
}

// ---------------------------------------------------------------------
// value= and ref= specified simultaneously -> ConflictingValueAndRef.
// ---------------------------------------------------------------------

#[test]
fn sb04_value_and_ref_both_specified_is_conflicting_value_and_ref() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"x\" value=\"literal\" ref=\"otherBean\"/>",
        "</bean></beans>"
    );
    let r = beans_xml::parse(source);
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::ConflictingValueAndRef),
        "expected ConflictingValueAndRef: {:?}",
        r.diagnostics
    );
    // Both beans/properties are still preserved (lenient parser, diagnostic
    // is additive, never a reason to drop the property) — some
    // deterministic value is still produced.
    let property = only_property(source);
    assert_eq!(property.name.value, "x");
}

#[test]
fn sb04_value_only_is_not_conflicting_value_and_ref() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"x\" value=\"literal\"/>",
        "</bean></beans>"
    );
    let r = beans_xml::parse(source);
    assert!(
        !r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::ConflictingValueAndRef),
        "value= alone must not be diagnosed: {:?}",
        r.diagnostics
    );
}

#[test]
fn sb04_ref_only_is_not_conflicting_value_and_ref() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"x\" ref=\"otherBean\"/>",
        "</bean></beans>"
    );
    let r = beans_xml::parse(source);
    assert!(
        !r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::ConflictingValueAndRef),
        "ref= alone must not be diagnosed: {:?}",
        r.diagnostics
    );
}

// ---------------------------------------------------------------------
// <meta key= value=> children.
// ---------------------------------------------------------------------

#[test]
fn sb04_meta_children_are_collected() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"x\" value=\"literal\">",
        "<meta key=\"docs\" value=\"see wiki\"/>",
        "<meta key=\"owner\" value=\"team-a\"/>",
        "</property>",
        "</bean></beans>"
    );
    let property = only_property(source);
    assert_eq!(property.meta.len(), 2);
    assert_eq!(property.meta[0].key.value, "docs");
    assert_eq!(property.meta[0].value.value, "see wiki");
    assert_eq!(property.meta[1].key.value, "owner");
    assert_eq!(property.meta[1].value.value, "team-a");
    match &property.value {
        InjectValue::Value(vl) => assert_eq!(vl.text.value, "literal"),
        other => panic!("expected Value, got {other:?}"),
    }
}

#[test]
fn sb04_no_meta_children_is_an_empty_vec() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"x\" value=\"literal\"/>",
        "</bean></beans>"
    );
    let property = only_property(source);
    assert!(property.meta.is_empty());
}

#[test]
fn sb04_meta_element_with_its_own_xmlns_redeclaration_is_not_treated_as_meta() {
    // Per standard XML namespace scoping, a `xmlns` declaration on `<meta>`
    // itself applies to that element's own name — a `<meta>` that
    // redeclares its own default namespace away from the beans namespace
    // is no longer a beans-namespace `<meta>` at all, and must not be
    // resolved against the *container* `<property>`'s scope alone
    // (the defect this fix closes, mirroring `collection.rs`'s
    // `<entry>`/`<key>` and `bean.rs`'s `<attribute>` overlay).
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"x\">",
        "<meta key=\"k\" value=\"v\" xmlns=\"http://example.com/other\"/>",
        "</property>",
        "</bean></beans>"
    );
    let property = only_property(source);
    assert!(
        property.meta.is_empty(),
        "a <meta> redeclaring its own xmlns away from the beans namespace \
         must not be collected: {:?}",
        property.meta
    );
}

// ---------------------------------------------------------------------
// span: <property> element extent, or the p:-namespace attribute's own
// span (that half is P2's territory — this only pins the element-form
// span).
// ---------------------------------------------------------------------

#[test]
fn sb04_property_span_covers_the_whole_element() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"x\" value=\"literal\"/>",
        "</bean></beans>"
    );
    let property = only_property(source);
    assert_eq!(
        &source[property.span.start as usize..property.span.end as usize],
        "<property name=\"x\" value=\"literal\"/>"
    );
}

// ---------------------------------------------------------------------
// Multiple <property> children on the same <bean> are all preserved, in
// document order.
// ---------------------------------------------------------------------

#[test]
fn sb04_multiple_properties_preserved_in_order() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"first\" value=\"1\"/>",
        "<property name=\"second\" value=\"2\"/>",
        "</bean></beans>"
    );
    let beans = parse_ok(source);
    let properties = &beans.beans[0].properties;
    assert_eq!(properties.len(), 2);
    assert_eq!(properties[0].name.value, "first");
    assert_eq!(properties[1].name.value, "second");
}

// ---------------------------------------------------------------------
// Deep bean -> property -> inner-bean -> property -> ... recursion must be
// bounded by `DEPTH_LIMIT`, never stack-overflow. This is the regression
// case for the cold-review finding that `parse_property` used to hardcode
// `depth = 0` into `parse_inject_value_child`, resetting the nesting
// counter at every `<property>` level and defeating the guard entirely for
// any recursion that goes through this unit's own call site.
// ---------------------------------------------------------------------

#[test]
fn sb04_property_inner_bean_recursion_is_bounded_by_depth_limit() {
    // One level past `DEPTH_LIMIT` worth of bean -> property -> inner-bean
    // nesting: enough to prove the guard actually fires through the public
    // `<property>` path (a hardcoded depth=0 caller would never trip it,
    // however deep this goes, and would instead stack-overflow the process
    // well before reaching this many levels).
    let levels = (beans_xml::DEPTH_LIMIT + 10) as usize;
    let mut source = String::from("<beans>");
    for _ in 0..levels {
        source.push_str("<bean class=\"com.example.Widget\"><property name=\"p\">");
    }
    source.push_str("<bean class=\"com.example.Leaf\"/>");
    for _ in 0..levels {
        source.push_str("</property></bean>");
    }
    source.push_str("</beans>");

    // Must not panic/abort (rule 4: parse never Err, never panics) and must
    // actually downgrade once the limit is reached.
    let r = beans_xml::parse(&source);
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::NestingLimitExceeded),
        "expected NestingLimitExceeded once recursion through <property> exceeds DEPTH_LIMIT"
    );
}
