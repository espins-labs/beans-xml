//! Unit **U5b** — collections (SB-07) tests.
//!
//! `collection::parse_collection_value`/`parse_map_entry`/etc. are all
//! `pub(crate)`/private — a seam not visible from this external
//! integration-test binary, the same situation `tests/u5a_inject_value.rs`
//! documents for `inject_value`. The real U5b test suite — per-kind
//! snapshots, `<entry key-ref/value-ref>`, the `<key>` element form,
//! map-level `key-type` vs entry-level `value-type`, `merge=`, nested
//! collections with an inner ref, span coverage, and the `DEPTH_LIMIT`
//! proptests (both the boundary check and the structural nested-depth
//! walk) — lives there: `src/collection.rs`'s own `#[cfg(test)] mod tests`.
//!
//! This file exercises the one thing observable from *outside* the crate
//! at this stage: end-to-end, through the public `beans_xml::parse` API,
//! via `<property>` (U6, already wired into `inject_value::parse_inject_value_child`,
//! which as of this unit also recognizes collection element names) —
//! proving the wiring in `inject_value.rs`'s own match arm actually reaches
//! production code, not just this module's in-crate unit tests.

use beans_xml::{Collection, DiagCode, InjectValue};

fn only_property_value(source: &str) -> InjectValue {
    let beans = beans_xml::parse(source).beans.expect("beans root");
    assert_eq!(
        beans.beans.len(),
        1,
        "expected exactly one top-level <bean>"
    );
    let mut properties = beans.beans.into_iter().next().unwrap().properties;
    assert_eq!(properties.len(), 1, "expected exactly one <property>");
    properties.remove(0).value
}

#[test]
fn u5b_list_through_property_end_to_end() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"names\">",
        "<list><value>one</value><value>two</value></list>",
        "</property>",
        "</bean></beans>"
    );
    match only_property_value(source) {
        InjectValue::Collection(c) => match c.value {
            Collection::List { items, .. } => assert_eq!(items.len(), 2),
            other => panic!("expected List, got {other:?}"),
        },
        other => panic!("expected Collection, got {other:?}"),
    }
    let r = beans_xml::parse(source);
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
}

#[test]
fn u5b_map_with_entry_ref_forms_through_property_end_to_end() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"lookup\">",
        "<map>",
        "<entry key=\"first\" value-ref=\"otherBean\"/>",
        "</map>",
        "</property>",
        "</bean></beans>"
    );
    match only_property_value(source) {
        InjectValue::Collection(c) => match c.value {
            Collection::Map { entries, .. } => {
                assert_eq!(entries.len(), 1);
                match &entries[0].value {
                    InjectValue::Ref(r) => assert_eq!(r.value.raw, "otherBean"),
                    other => panic!("expected Ref, got {other:?}"),
                }
            }
            other => panic!("expected Map, got {other:?}"),
        },
        other => panic!("expected Collection, got {other:?}"),
    }
}

#[test]
fn u5b_props_through_property_end_to_end() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"settings\">",
        "<props><prop key=\"driver\">com.example.Driver</prop></props>",
        "</property>",
        "</bean></beans>"
    );
    match only_property_value(source) {
        InjectValue::Collection(c) => match c.value {
            Collection::Props { entries, .. } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].key.value, "driver");
                assert_eq!(entries[0].value.text.value, "com.example.Driver");
            }
            other => panic!("expected Props, got {other:?}"),
        },
        other => panic!("expected Collection, got {other:?}"),
    }
}

#[test]
fn u5b_nested_collection_with_inner_bean_ref_through_property() {
    // Quartz-wiring-shaped fixture: a <map> entry's value is itself a
    // <list> holding a <ref> — the exact shape the spec's SB-07 row calls
    // out ("internal ref recursion") and warns must not be missed
    // ("quartz `triggers <list><ref>` must not be missing here").
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"triggers\">",
        "<map><entry key=\"g1\"><list><ref bean=\"triggerBean\"/></list></entry></map>",
        "</property>",
        "</bean></beans>"
    );
    match only_property_value(source) {
        InjectValue::Collection(c) => match c.value {
            Collection::Map { entries, .. } => match &entries[0].value {
                InjectValue::Collection(inner) => match &inner.value {
                    Collection::List { items, .. } => match &items[0] {
                        InjectValue::Ref(r) => assert_eq!(r.value.raw, "triggerBean"),
                        other => panic!("expected Ref, got {other:?}"),
                    },
                    other => panic!("expected List, got {other:?}"),
                },
                other => panic!("expected nested Collection, got {other:?}"),
            },
            other => panic!("expected Map, got {other:?}"),
        },
        other => panic!("expected Collection, got {other:?}"),
    }
    let r = beans_xml::parse(source);
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
}

#[test]
fn u5b_depth_limit_is_reachable_through_property_list_recursion() {
    // Mirrors `tests/u6_property.rs`'s own
    // `sb04_property_inner_bean_recursion_is_bounded_by_depth_limit` — same
    // regression shape, but through nested `<list>` self-recursion instead
    // of nested inner `<bean>`s, proving `crate::collection`'s own
    // `DEPTH_LIMIT` guard is actually wired into the real recursive descent
    // (not just exercised directly against `parse_collection_value` in
    // `src/collection.rs`'s own unit tests).
    let levels = (beans_xml::DEPTH_LIMIT + 10) as usize;
    let mut source = String::from(concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"deep\">"
    ));
    for _ in 0..levels {
        source.push_str("<list>");
    }
    source.push_str("<value>leaf</value>");
    for _ in 0..levels {
        source.push_str("</list>");
    }
    source.push_str("</property></bean></beans>");

    let r = beans_xml::parse(&source);
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::NestingLimitExceeded),
        "expected NestingLimitExceeded once <list> nesting through <property> exceeds DEPTH_LIMIT"
    );
}
