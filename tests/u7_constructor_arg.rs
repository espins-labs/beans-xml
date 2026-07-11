//! Unit **U7** — `<constructor-arg>` (SB-05): wraps `InjectValue` with
//! `index`/`type`/`name` + `<meta>`. Test design per
//! the internal build plan's U7 row: snapshot + indexless
//! positional args + `type`/`name` + `value`/`ref` both specified →
//! `ConflictingValueAndRef` + `<meta>`.
//!
//! `bean::parse_bean`/`bean::BeanFrame`/`constructor_arg::finish_constructor_arg`
//! are `pub(crate)` — not visible from this external integration-test
//! binary — so every test here goes through the public API
//! (`beans_xml::parse`) only, the same convention `tests/u6_property.rs`
//! established.

use beans_xml::{ConstructorArg, DiagCode, InjectValue, RefKind};

fn parse_ok(source: &str) -> beans_xml::BeansFile {
    beans_xml::parse(source).beans.expect("beans root")
}

fn only_constructor_arg(source: &str) -> ConstructorArg {
    let beans = parse_ok(source);
    assert_eq!(
        beans.beans.len(),
        1,
        "expected exactly one top-level <bean>"
    );
    let mut args = beans.beans.into_iter().next().unwrap().constructor_args;
    assert_eq!(args.len(), 1, "expected exactly one <constructor-arg>");
    args.remove(0)
}

// ---------------------------------------------------------------------
// Snapshot: a representative <constructor-arg> exercising index + type +
// value= shorthand together.
// ---------------------------------------------------------------------

#[test]
fn sb05_indexed_typed_value_snapshot() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg index=\"0\" type=\"java.lang.String\" value=\"hello\"/>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    assert_eq!(arg.index, Some(0));
    assert_eq!(
        arg.type_ref.as_ref().map(|t| t.value.raw.as_str()),
        Some("java.lang.String")
    );
    assert!(arg.name.is_none());
    match &arg.value {
        InjectValue::Value(vl) => assert_eq!(vl.text.value, "hello"),
        other => panic!("expected Value, got {other:?}"),
    }
    assert!(arg.meta.is_empty());

    let r = beans_xml::parse(source);
    assert!(
        r.diagnostics.is_empty(),
        "a well-formed constructor-arg must not raise diagnostics: {:?}",
        r.diagnostics
    );
}

// ---------------------------------------------------------------------
// Indexless positional args: multiple <constructor-arg> with no index=,
// resolved in document order.
// ---------------------------------------------------------------------

#[test]
fn sb05_indexless_positional_args_preserved_in_order() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg value=\"first\"/>",
        "<constructor-arg value=\"second\"/>",
        "<constructor-arg value=\"third\"/>",
        "</bean></beans>"
    );
    let beans = parse_ok(source);
    let args = &beans.beans[0].constructor_args;
    assert_eq!(args.len(), 3);
    for arg in args {
        assert_eq!(arg.index, None, "no index= attribute was written");
    }
    match &args[0].value {
        InjectValue::Value(vl) => assert_eq!(vl.text.value, "first"),
        other => panic!("expected Value, got {other:?}"),
    }
    match &args[1].value {
        InjectValue::Value(vl) => assert_eq!(vl.text.value, "second"),
        other => panic!("expected Value, got {other:?}"),
    }
    match &args[2].value {
        InjectValue::Value(vl) => assert_eq!(vl.text.value, "third"),
        other => panic!("expected Value, got {other:?}"),
    }
}

#[test]
fn sb05_missing_or_non_numeric_index_falls_back_to_none() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg index=\"not-a-number\" value=\"x\"/>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    assert_eq!(arg.index, None);
    // No invented DiagCode for this shape (rule 4: never panic, no new
    // diagnostic for an edge case the spec's table doesn't call out).
    let r = beans_xml::parse(source);
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
}

#[test]
fn sb05_empty_index_attr_falls_back_to_none() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg index=\"\" value=\"x\"/>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    assert_eq!(arg.index, None);
    let r = beans_xml::parse(source);
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
}

#[test]
fn sb05_multi_digit_index_is_parsed() {
    // index="0" (the snapshot test's value) is the smallest representable
    // index and doesn't distinguish "parsed the digits" from "defaulted to
    // zero" — a multi-digit value pins down that the full u32 parse runs,
    // not just a truthiness/zero check.
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg index=\"10\" value=\"x\"/>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    assert_eq!(arg.index, Some(10));
}

// ---------------------------------------------------------------------
// type= and name= attributes.
// ---------------------------------------------------------------------

#[test]
fn sb05_type_and_name_attrs_are_captured() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg name=\"label\" type=\"java.lang.String\" value=\"hi\"/>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    assert_eq!(arg.name.as_ref().map(|n| n.value.as_str()), Some("label"));
    assert_eq!(
        arg.type_ref.as_ref().map(|t| t.value.raw.as_str()),
        Some("java.lang.String")
    );
}

#[test]
fn sb05_empty_type_attr_yields_no_class_ref() {
    // Invariant #5: ClassRef.raw is never empty — a present-but-empty
    // type= must not manufacture one.
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg type=\"\" value=\"x\"/>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    assert!(arg.type_ref.is_none());
}

// ---------------------------------------------------------------------
// Nested child forms: <value type="...">, <ref bean=...>, inner <bean>.
// ---------------------------------------------------------------------

#[test]
fn sb05_value_child_with_type_snapshot() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg><value type=\"java.lang.Integer\">42</value></constructor-arg>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    match &arg.value {
        InjectValue::Value(vl) => {
            assert_eq!(vl.text.value, "42");
            assert_eq!(
                vl.value_type.as_ref().map(|t| t.value.raw.as_str()),
                Some("java.lang.Integer")
            );
        }
        other => panic!("expected Value, got {other:?}"),
    }
}

#[test]
fn sb05_ref_child_element_snapshot() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg><ref bean=\"otherBean\"/></constructor-arg>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    match &arg.value {
        InjectValue::Ref(r) => assert_eq!(r.value.raw, "otherBean"),
        other => panic!("expected Ref, got {other:?}"),
    }
}

#[test]
fn sb05_nested_inner_bean_snapshot() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg>",
        "<bean id=\"innerBean\" class=\"com.example.Gadget\"/>",
        "</constructor-arg>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    match &arg.value {
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
// ref= shorthand attribute.
// ---------------------------------------------------------------------

#[test]
fn sb05_ref_attr_shorthand_snapshot() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg ref=\"otherBean\"/>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    match &arg.value {
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
// value= and ref= specified simultaneously -> ConflictingValueAndRef
// (ref-vs-value edge case, per the build plan's U7 row).
// ---------------------------------------------------------------------

#[test]
fn sb05_value_and_ref_both_specified_is_conflicting_value_and_ref() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg value=\"literal\" ref=\"otherBean\"/>",
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
    // Both the bean and the constructor-arg are still preserved (lenient
    // parser, diagnostic is additive) — some deterministic value is still
    // produced (value= wins, per the resolution precedence).
    let arg = only_constructor_arg(source);
    match &arg.value {
        InjectValue::Value(vl) => assert_eq!(vl.text.value, "literal"),
        other => panic!("expected Value (value= wins precedence), got {other:?}"),
    }
}

#[test]
fn sb05_value_only_is_not_conflicting_value_and_ref() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg value=\"literal\"/>",
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
fn sb05_value_attr_wins_over_value_shaped_child() {
    // Locks in the arg's own precedence chain (value= attr > ref= attr >
    // first value-shaped child > Null): a shorthand value= attribute
    // suppresses a value-shaped child entirely, even though in isolation
    // that child would resolve to a Ref.
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg value=\"literal\"><ref bean=\"otherBean\"/></constructor-arg>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    match &arg.value {
        InjectValue::Value(vl) => assert_eq!(vl.text.value, "literal"),
        other => panic!("expected Value (value= wins over child), got {other:?}"),
    }
    let r = beans_xml::parse(source);
    assert!(
        !r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::ConflictingValueAndRef),
        "value= with a value-shaped child (no ref=) must not be diagnosed: {:?}",
        r.diagnostics
    );
}

#[test]
fn sb05_ref_attr_wins_over_value_shaped_child() {
    // Same precedence chain, one step down: ref= (with no value=) still
    // wins over a value-shaped child.
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg ref=\"otherBean\"><value>literal</value></constructor-arg>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    match &arg.value {
        InjectValue::Ref(r) => assert_eq!(r.value.raw, "otherBean"),
        other => panic!("expected Ref (ref= wins over child), got {other:?}"),
    }
}

#[test]
fn sb05_ref_only_is_not_conflicting_value_and_ref() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg ref=\"otherBean\"/>",
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
// Nothing at all resolved: bare <constructor-arg/> -> InjectValue::Null.
// ---------------------------------------------------------------------

#[test]
fn sb05_bare_constructor_arg_with_nothing_falls_back_to_null() {
    // No value=/ref=/value-shaped child at all — `resolve_value`'s own
    // total fallback (last precedence step): an opaque `Null` at the
    // element's own span, never a panic, never a missing
    // `ConstructorArg.value`.
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg/>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    match &arg.value {
        InjectValue::Null(span) => assert_eq!(*span, arg.span),
        other => panic!("expected Null fallback, got {other:?}"),
    }
    let r = beans_xml::parse(source);
    assert!(
        r.diagnostics.is_empty(),
        "a bare <constructor-arg/> is not itself an error shape: {:?}",
        r.diagnostics
    );
}

// ---------------------------------------------------------------------
// <meta key= value=> children.
// ---------------------------------------------------------------------

#[test]
fn sb05_meta_children_are_collected() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg value=\"literal\">",
        "<meta key=\"docs\" value=\"see wiki\"/>",
        "<meta key=\"owner\" value=\"team-a\"/>",
        "</constructor-arg>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    assert_eq!(arg.meta.len(), 2);
    assert_eq!(arg.meta[0].key.value, "docs");
    assert_eq!(arg.meta[0].value.value, "see wiki");
    assert_eq!(arg.meta[1].key.value, "owner");
    assert_eq!(arg.meta[1].value.value, "team-a");
    match &arg.value {
        InjectValue::Value(vl) => assert_eq!(vl.text.value, "literal"),
        other => panic!("expected Value, got {other:?}"),
    }
}

#[test]
fn sb05_no_meta_children_is_an_empty_vec() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg value=\"literal\"/>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    assert!(arg.meta.is_empty());
}

#[test]
fn sb05_meta_element_with_its_own_xmlns_redeclaration_is_not_treated_as_meta() {
    // Per standard XML namespace scoping, a `xmlns` declaration on `<meta>`
    // itself applies to that element's own name — a `<meta>` that
    // redeclares its own default namespace away from the beans namespace
    // is no longer a beans-namespace `<meta>` at all, and must not be
    // resolved against the *container* `<constructor-arg>`'s scope alone
    // (the defect this fix closes, mirroring `collection.rs`'s
    // `<entry>`/`<key>` and `bean.rs`'s `<attribute>` overlay).
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg>",
        "<meta key=\"k\" value=\"v\" xmlns=\"http://example.com/other\"/>",
        "</constructor-arg>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    assert!(
        arg.meta.is_empty(),
        "a <meta> redeclaring its own xmlns away from the beans namespace \
         must not be collected: {:?}",
        arg.meta
    );
}

// ---------------------------------------------------------------------
// span: <constructor-arg> element extent.
// ---------------------------------------------------------------------

#[test]
fn sb05_constructor_arg_span_covers_the_whole_element() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<constructor-arg index=\"0\" value=\"literal\"/>",
        "</bean></beans>"
    );
    let arg = only_constructor_arg(source);
    assert_eq!(
        &source[arg.span.start as usize..arg.span.end as usize],
        "<constructor-arg index=\"0\" value=\"literal\"/>"
    );
}

// ---------------------------------------------------------------------
// Deep bean -> constructor-arg -> inner-bean -> constructor-arg -> ...
// recursion must be bounded by `DEPTH_LIMIT`, never stack-overflow — the
// same regression class `tests/u6_property.rs`'s matching test guards for
// `<property>`, exercised here for `<constructor-arg>`'s own call site into
// `inject_value::parse_inject_value_child`.
// ---------------------------------------------------------------------

#[test]
fn sb05_constructor_arg_inner_bean_recursion_is_bounded_by_depth_limit() {
    let levels = (beans_xml::DEPTH_LIMIT + 10) as usize;
    let mut source = String::from("<beans>");
    for _ in 0..levels {
        source.push_str("<bean class=\"com.example.Widget\"><constructor-arg>");
    }
    source.push_str("<bean class=\"com.example.Leaf\"/>");
    for _ in 0..levels {
        source.push_str("</constructor-arg></bean>");
    }
    source.push_str("</beans>");

    let r = beans_xml::parse(&source);
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::NestingLimitExceeded),
        "expected NestingLimitExceeded once recursion through <constructor-arg> exceeds DEPTH_LIMIT"
    );
}
