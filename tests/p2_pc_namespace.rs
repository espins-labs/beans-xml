//! Unit **P2** — `p:`/`c:`-namespace attribute normalization (SB-08):
//! `p:foo="v"` → `Property{name: foo, Value}`; `p:foo-ref="b"` →
//! `Property{name: foo, Ref}`; `c:_0-ref="b"` → `ConstructorArg{index: 0,
//! Ref}`; `c:name-ref` → `ConstructorArg{name, Ref}`; `c:_0="lit"`/
//! `c:name="lit"` → `Value`. Test design per
//! the internal build plan's P2 row: table (`c:_0` index vs
//! `c:name` name, `-ref` suffix, literal, span preserved).
//!
//! `bean::parse_bean`/`bean::normalize_pc_attr` are `pub(crate)` — not
//! visible from this external integration-test binary — so every test here
//! goes through the public API (`beans_xml::parse`) only, the same
//! convention `tests/u6_property.rs`/`tests/u7_constructor_arg.rs`
//! established.

use beans_xml::{DiagCode, InjectValue, RefKind};

const P_XMLNS: &str = "xmlns:p=\"http://www.springframework.org/schema/p\"";
const C_XMLNS: &str = "xmlns:c=\"http://www.springframework.org/schema/c\"";

fn parse_ok(source: &str) -> beans_xml::BeansFile {
    beans_xml::parse(source).beans.expect("beans root")
}

fn only_bean(source: &str) -> beans_xml::Bean {
    let beans = parse_ok(source);
    assert_eq!(
        beans.beans.len(),
        1,
        "expected exactly one top-level <bean>"
    );
    beans.beans.into_iter().next().unwrap()
}

// ---------------------------------------------------------------------
// Table: p:foo="literal" -> Property{name: foo, Value}.
// ---------------------------------------------------------------------

#[test]
fn sb08_p_namespace_literal_attribute_becomes_value_property() {
    let source = format!(
        "<beans {P_XMLNS}><bean id=\"a\" class=\"com.example.Widget\" p:label=\"hello\"/></beans>"
    );
    let bean = only_bean(&source);
    assert_eq!(
        bean.properties.len(),
        1,
        "properties: {:?}",
        bean.properties
    );
    let property = &bean.properties[0];
    assert_eq!(property.name.value, "label");
    match &property.value {
        InjectValue::Value(vl) => assert_eq!(vl.text.value, "hello"),
        other => panic!("expected Value, got {other:?}"),
    }
    assert!(property.meta.is_empty());

    let diagnostics = beans_xml::parse(&source).diagnostics;
    assert!(
        diagnostics.is_empty(),
        "well-formed p:label= must not raise diagnostics: {diagnostics:?}"
    );
}

// ---------------------------------------------------------------------
// Table: p:foo-ref="bean" -> Property{name: foo, Ref}.
// ---------------------------------------------------------------------

#[test]
fn sb08_p_namespace_ref_suffix_attribute_becomes_ref_property() {
    let source = format!(
        "<beans {P_XMLNS}><bean id=\"a\" class=\"com.example.Widget\" p:collaborator-ref=\"target\"/></beans>"
    );
    let bean = only_bean(&source);
    assert_eq!(
        bean.properties.len(),
        1,
        "properties: {:?}",
        bean.properties
    );
    let property = &bean.properties[0];
    assert_eq!(property.name.value, "collaborator");
    match &property.value {
        InjectValue::Ref(bean_ref) => {
            assert_eq!(bean_ref.value.raw, "target");
            assert_eq!(bean_ref.value.kind, RefKind::Bean);
        }
        other => panic!("expected Ref, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Table: c:_0="literal" -> ConstructorArg{index: Some(0), Value}.
// ---------------------------------------------------------------------

#[test]
fn sb08_c_namespace_index_literal_attribute_becomes_value_arg() {
    let source = format!(
        "<beans {C_XMLNS}><bean id=\"a\" class=\"com.example.Widget\" c:_0=\"42\"/></beans>"
    );
    let bean = only_bean(&source);
    assert_eq!(
        bean.constructor_args.len(),
        1,
        "constructor_args: {:?}",
        bean.constructor_args
    );
    let arg = &bean.constructor_args[0];
    assert_eq!(arg.index, Some(0));
    assert_eq!(arg.name, None);
    match &arg.value {
        InjectValue::Value(vl) => assert_eq!(vl.text.value, "42"),
        other => panic!("expected Value, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Table: c:_0-ref="bean" -> ConstructorArg{index: Some(0), Ref}.
// ---------------------------------------------------------------------

#[test]
fn sb08_c_namespace_index_ref_suffix_attribute_becomes_ref_arg() {
    let source = format!(
        "<beans {C_XMLNS}><bean id=\"a\" class=\"com.example.Widget\" c:_0-ref=\"target\"/></beans>"
    );
    let bean = only_bean(&source);
    assert_eq!(bean.constructor_args.len(), 1);
    let arg = &bean.constructor_args[0];
    assert_eq!(arg.index, Some(0));
    assert_eq!(arg.name, None);
    match &arg.value {
        InjectValue::Ref(bean_ref) => {
            assert_eq!(bean_ref.value.raw, "target");
            assert_eq!(bean_ref.value.kind, RefKind::Bean);
        }
        other => panic!("expected Ref, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Table: c:name="literal" -> ConstructorArg{name: Some(name), Value}.
// ---------------------------------------------------------------------

#[test]
fn sb08_c_namespace_named_literal_attribute_becomes_value_arg() {
    let source = format!(
        "<beans {C_XMLNS}><bean id=\"a\" class=\"com.example.Widget\" c:count=\"7\"/></beans>"
    );
    let bean = only_bean(&source);
    assert_eq!(bean.constructor_args.len(), 1);
    let arg = &bean.constructor_args[0];
    assert_eq!(arg.index, None);
    assert_eq!(arg.name.as_ref().map(|s| s.value.as_str()), Some("count"));
    match &arg.value {
        InjectValue::Value(vl) => assert_eq!(vl.text.value, "7"),
        other => panic!("expected Value, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Table: c:name-ref="bean" -> ConstructorArg{name: Some(name), Ref}.
// ---------------------------------------------------------------------

#[test]
fn sb08_c_namespace_named_ref_suffix_attribute_becomes_ref_arg() {
    let source = format!(
        "<beans {C_XMLNS}><bean id=\"a\" class=\"com.example.Widget\" c:dataSource-ref=\"ds\"/></beans>"
    );
    let bean = only_bean(&source);
    assert_eq!(bean.constructor_args.len(), 1);
    let arg = &bean.constructor_args[0];
    assert_eq!(arg.index, None);
    assert_eq!(
        arg.name.as_ref().map(|s| s.value.as_str()),
        Some("dataSource")
    );
    match &arg.value {
        InjectValue::Ref(bean_ref) => {
            assert_eq!(bean_ref.value.raw, "ds");
            assert_eq!(bean_ref.value.kind, RefKind::Bean);
        }
        other => panic!("expected Ref, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Span preserved: Property/ConstructorArg.span covers the whole attribute
// (name="value"), and name.span covers just the local name (prefix/-ref
// stripped) — both slice back to the source.
// ---------------------------------------------------------------------

#[test]
fn sb08_p_property_span_and_name_span_slice_back_to_the_source() {
    let source = format!(
        "<beans {P_XMLNS}><bean id=\"a\" class=\"com.example.Widget\" p:label=\"hello\"/></beans>"
    );
    let bean = only_bean(&source);
    let property = &bean.properties[0];
    assert_eq!(
        &source[property.span.start as usize..property.span.end as usize],
        "p:label=\"hello\""
    );
    assert_eq!(
        &source[property.name.span.start as usize..property.name.span.end as usize],
        "label"
    );
}

#[test]
fn sb08_p_ref_property_span_and_name_span_slice_back_to_the_source() {
    let source = format!(
        "<beans {P_XMLNS}><bean id=\"a\" class=\"com.example.Widget\" p:collaborator-ref=\"target\"/></beans>"
    );
    let bean = only_bean(&source);
    let property = &bean.properties[0];
    assert_eq!(
        &source[property.span.start as usize..property.span.end as usize],
        "p:collaborator-ref=\"target\""
    );
    assert_eq!(
        &source[property.name.span.start as usize..property.name.span.end as usize],
        "collaborator"
    );
}

// ---------------------------------------------------------------------
// Non-ASCII (Korean) local name — regression for the hand-rolled byte-offset
// arithmetic in `normalize_pc_attr`/`local_name_span` (`local_start =
// name_span.start + prefix.len() + 1`, `value.span.end + 1`): the doc
// comment on `normalize_pc_attr` argues this is safe because `:`/`-` never
// occur mid-UTF-8-sequence, but nothing exercised a multi-byte name to
// prove it. `라벨` is 6 bytes / 2 chars, so a char-count-based offset bug
// would slice mid-character and this test would fail (or panic on a
// non-char-boundary string index) instead of asserting green.
// ---------------------------------------------------------------------

#[test]
fn sb08_p_namespace_non_ascii_local_name_span_slices_back_to_the_source() {
    let source = format!(
        "<beans {P_XMLNS}><bean id=\"a\" class=\"com.example.Widget\" p:라벨=\"hello\"/></beans>"
    );
    let bean = only_bean(&source);
    assert_eq!(
        bean.properties.len(),
        1,
        "properties: {:?}",
        bean.properties
    );
    let property = &bean.properties[0];
    assert_eq!(property.name.value, "라벨");
    assert_eq!(
        &source[property.span.start as usize..property.span.end as usize],
        "p:라벨=\"hello\""
    );
    assert_eq!(
        &source[property.name.span.start as usize..property.name.span.end as usize],
        "라벨"
    );
    match &property.value {
        InjectValue::Value(vl) => assert_eq!(vl.text.value, "hello"),
        other => panic!("expected Value, got {other:?}"),
    }
}

#[test]
fn sb08_c_namespace_non_ascii_named_ref_local_name_span_slices_back_to_the_source() {
    let source = format!(
        "<beans {C_XMLNS}><bean id=\"a\" class=\"com.example.Widget\" c:이름-ref=\"target\"/></beans>"
    );
    let bean = only_bean(&source);
    assert_eq!(
        bean.constructor_args.len(),
        1,
        "constructor_args: {:?}",
        bean.constructor_args
    );
    let arg = &bean.constructor_args[0];
    assert_eq!(arg.index, None);
    let name = arg.name.as_ref().expect("named arg");
    assert_eq!(name.value, "이름");
    assert_eq!(
        &source[arg.span.start as usize..arg.span.end as usize],
        "c:이름-ref=\"target\""
    );
    assert_eq!(
        &source[name.span.start as usize..name.span.end as usize],
        "이름"
    );
    match &arg.value {
        InjectValue::Ref(bean_ref) => {
            assert_eq!(bean_ref.value.raw, "target");
            assert_eq!(bean_ref.value.kind, RefKind::Bean);
        }
        other => panic!("expected Ref, got {other:?}"),
    }
}

#[test]
fn sb08_c_named_arg_span_and_name_span_slice_back_to_the_source() {
    let source = format!(
        "<beans {C_XMLNS}><bean id=\"a\" class=\"com.example.Widget\" c:dataSource-ref=\"ds\"/></beans>"
    );
    let bean = only_bean(&source);
    let arg = &bean.constructor_args[0];
    assert_eq!(
        &source[arg.span.start as usize..arg.span.end as usize],
        "c:dataSource-ref=\"ds\""
    );
    let name = arg.name.as_ref().expect("named arg");
    assert_eq!(
        &source[name.span.start as usize..name.span.end as usize],
        "dataSource"
    );
}

#[test]
fn sb08_c_index_arg_span_slices_back_to_the_source() {
    let source = format!(
        "<beans {C_XMLNS}><bean id=\"a\" class=\"com.example.Widget\" c:_0-ref=\"target\"/></beans>"
    );
    let bean = only_bean(&source);
    let arg = &bean.constructor_args[0];
    assert_eq!(
        &source[arg.span.start as usize..arg.span.end as usize],
        "c:_0-ref=\"target\""
    );
    assert_eq!(arg.index, Some(0));
    assert!(arg.name.is_none());
}

// ---------------------------------------------------------------------
// Multi-digit index (`c:_12`) — not just the single-digit `_0` shape.
// ---------------------------------------------------------------------

#[test]
fn sb08_c_namespace_multi_digit_index_parses_correctly() {
    let source = format!(
        "<beans {C_XMLNS}><bean id=\"a\" class=\"com.example.Widget\" c:_12=\"x\"/></beans>"
    );
    let bean = only_bean(&source);
    assert_eq!(bean.constructor_args[0].index, Some(12));
}

// ---------------------------------------------------------------------
// A malformed index-shaped local name (`_0a`, not all-digit after `_`)
// falls back to the named form rather than silently guessing at an index.
// ---------------------------------------------------------------------

#[test]
fn sb08_c_namespace_non_numeric_underscore_prefixed_name_falls_back_to_named_form() {
    let source = format!(
        "<beans {C_XMLNS}><bean id=\"a\" class=\"com.example.Widget\" c:_0a=\"x\"/></beans>"
    );
    let bean = only_bean(&source);
    let arg = &bean.constructor_args[0];
    assert_eq!(arg.index, None);
    assert_eq!(arg.name.as_ref().map(|s| s.value.as_str()), Some("_0a"));
}

// ---------------------------------------------------------------------
// p:/c: entries join the SAME Vec<Property>/Vec<ConstructorArg> as the
// element forms (<property>/<constructor-arg>), not a separate list.
// ---------------------------------------------------------------------

#[test]
fn sb08_pc_entries_join_the_same_vecs_as_element_forms() {
    let source = format!(
        concat!(
            "<beans {p} {c}>",
            "<bean id=\"a\" class=\"com.example.Widget\" p:label=\"hello\" c:_1=\"x\">",
            "<property name=\"other\" value=\"world\"/>",
            "<constructor-arg index=\"0\" value=\"first\"/>",
            "</bean></beans>"
        ),
        p = P_XMLNS,
        c = C_XMLNS,
    );
    let bean = only_bean(&source);

    assert_eq!(
        bean.properties.len(),
        2,
        "properties: {:?}",
        bean.properties
    );
    let names: Vec<&str> = bean
        .properties
        .iter()
        .map(|p| p.name.value.as_str())
        .collect();
    assert!(names.contains(&"label"));
    assert!(names.contains(&"other"));

    assert_eq!(
        bean.constructor_args.len(),
        2,
        "constructor_args: {:?}",
        bean.constructor_args
    );
    let indices: Vec<Option<u32>> = bean.constructor_args.iter().map(|a| a.index).collect();
    assert!(indices.contains(&Some(0)));
    assert!(indices.contains(&Some(1)));
}

// ---------------------------------------------------------------------
// Raw-prefix fallback: p:/c: attributes still normalize even when no
// xmlns:p/xmlns:c declaration is in scope (same "resolved URI, or raw
// prefix" fallback `dispatch::is_context_ns`/`is_util_ns` already apply).
// ---------------------------------------------------------------------

#[test]
fn sb08_pc_namespace_normalizes_without_an_xmlns_declaration() {
    let source =
        "<beans><bean id=\"a\" class=\"com.example.Widget\" p:label=\"hello\" c:_0=\"x\"/></beans>";
    let bean = only_bean(source);
    assert_eq!(bean.properties.len(), 1);
    assert_eq!(bean.properties[0].name.value, "label");
    assert_eq!(bean.constructor_args.len(), 1);
    assert_eq!(bean.constructor_args[0].index, Some(0));
}

// ---------------------------------------------------------------------
// Unprefixed attributes are untouched by the p/c hook (no accidental
// Property/ConstructorArg from ordinary <bean> attributes).
// ---------------------------------------------------------------------

#[test]
fn sb08_unprefixed_attributes_do_not_become_properties_or_args() {
    let source = "<beans><bean id=\"a\" class=\"com.example.Widget\" scope=\"singleton\"/></beans>";
    let bean = only_bean(source);
    assert!(bean.properties.is_empty());
    assert!(bean.constructor_args.is_empty());
}

// ---------------------------------------------------------------------
// An xmlns:p/xmlns:c declaration attribute itself is not mistaken for a
// p:/c:-namespace property/ctor-arg.
// ---------------------------------------------------------------------

#[test]
fn sb08_xmlns_declaration_attributes_are_not_normalized() {
    let source =
        format!("<beans {P_XMLNS} {C_XMLNS}><bean id=\"a\" class=\"com.example.Widget\"/></beans>");
    let bean = only_bean(&source);
    assert!(bean.properties.is_empty());
    assert!(bean.constructor_args.is_empty());
}

// ---------------------------------------------------------------------
// Wrong-URI negative: a `p:`/`c:` prefix bound to some other, non-Spring
// URI must not normalize — guards the resolved-URI discriminator
// (`is_p_ns`/`is_c_ns`'s `ns == P_NS_URI`/`ns == C_NS_URI` check), which
// otherwise had no test pinning that an *actually declared but wrong* URI
// is rejected (only the "no declaration at all" raw-prefix-fallback case,
// `sb08_pc_namespace_normalizes_without_an_xmlns_declaration` above, was
// covered).
// ---------------------------------------------------------------------

#[test]
fn sb08_p_prefix_bound_to_a_non_spring_uri_does_not_normalize() {
    let source = "<beans xmlns:p=\"http://example.com/other\"><bean id=\"a\" \
                  class=\"com.example.Widget\" p:label=\"hello\"/></beans>";
    let bean = only_bean(source);
    assert!(
        bean.properties.is_empty(),
        "p: bound to a non-Spring URI must not become a Property: {:?}",
        bean.properties
    );
}

#[test]
fn sb08_c_prefix_bound_to_a_non_spring_uri_does_not_normalize() {
    let source = "<beans xmlns:c=\"http://example.com/other\"><bean id=\"a\" \
                  class=\"com.example.Widget\" c:_0=\"42\"/></beans>";
    let bean = only_bean(source);
    assert!(
        bean.constructor_args.is_empty(),
        "c: bound to a non-Spring URI must not become a ConstructorArg: {:?}",
        bean.constructor_args
    );
}

// ---------------------------------------------------------------------
// Empty p:foo-ref="" still emits the Property (additive diagnostic
// policy) with an opaque Null value, plus RefWithoutTarget.
// ---------------------------------------------------------------------

#[test]
fn sb08_p_namespace_empty_ref_suffix_emits_ref_without_target_but_still_pushes_property() {
    let source = format!(
        "<beans {P_XMLNS}><bean id=\"a\" class=\"com.example.Widget\" p:collaborator-ref=\"\"/></beans>"
    );
    let result = beans_xml::parse(&source);
    let beans = result.beans.expect("beans root");
    let bean = beans.beans.into_iter().next().expect("one bean");
    assert_eq!(bean.properties.len(), 1);
    assert_eq!(bean.properties[0].name.value, "collaborator");
    assert!(matches!(bean.properties[0].value, InjectValue::Null(_)));
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == DiagCode::RefWithoutTarget));
}
