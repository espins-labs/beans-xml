//! Unit **I1** — class-reference collection conformance (SB-12), per the
//! internal build plan's own I1 row: **not construction, verification/
//! aggregation**. Every `ClassRef` site is filled by its owning leaf unit
//! (U4 `class=`, U7 `constructor-arg@type`, P6 `qualifier@type`, U5b
//! `value-type`/`key-type`, U5a `<value type=>`) — this file has no field
//! of its own to own. What it cross-asserts, in one fixture that exercises
//! every site **simultaneously**, is exactly the build plan's I1 mandate:
//!
//! 1. Every `ClassRef` collection site actually populates, including the
//!    edge shapes the spec's own table calls out: a **non-FQN "alias" raw**
//!    string preserved verbatim (no validation/resolution — `ClassRef`'s own
//!    doc comment: "FQN 원문, 별칭·제네릭 해석은 소비자"), an **inner-class
//!    `$`** name, and **array/generic** suffixes (`Foo[]`, `List<Bar>`) —
//!    all just opaque text as far as this parser is concerned.
//! 2. **Exclusion**: `ScanFilter::expression` (scan-filter pattern) and
//!    `ReplacedMethod::arg_types` (both the `match=` and, per the arg-type
//!    text-content ruling, the text-content form) must never be `ClassRef`
//!    — they are plain `String` patterns. The type system already forbids
//!    literally storing a `ClassRef` there, so what this asserts is the
//!    *behavioral* tell: both fields accept content a `ClassRef` site could
//!    never legally hold (non-FQN wildcard/regex/pointcut syntax; for
//!    `arg_types`, a whitespace-only `match=` silently folding into its
//!    sibling text body instead of being recorded or diagnosed) completely
//!    unmodified — proof they were never funneled through `ClassRef`
//!    construction.
//!
//! Every symbol here is public (`beans_xml::parse`, `model` re-exports) —
//! pure end-to-end, same convention every other `tests/i*.rs` file follows
//! (see e.g. `tests/i6_routing_recursion.rs`'s own doc comment).

use beans_xml::{Bean, Collection, InjectValue};

fn parse_ok(source: &str) -> beans_xml::BeansFile {
    let result = beans_xml::parse(source);
    assert!(
        result.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    result.beans.expect("beans root")
}

fn find_bean<'a>(beans_file: &'a beans_xml::BeansFile, id: &str) -> &'a Bean {
    beans_file
        .beans
        .iter()
        .find(|b| b.id.as_ref().map(|s| s.value.as_str()) == Some(id))
        .unwrap_or_else(|| panic!("no <bean id=\"{id}\"> in fixture"))
}

/// The I1 fixture: one document exercising every `ClassRef` site the spec's
/// SB-12 row names, plus the two exclusion sites, all at once.
const SOURCE: &str = concat!(
    "<beans xmlns:context=\"http://www.springframework.org/schema/context\">",
    // --- Exclusion site 1: scan-filter expression — a wildcard/regex/
    // pointcut pattern, not a class name, stays a raw String.
    r#"<context:component-scan base-package="com.example.scan">"#,
    r#"<context:include-filter type="aspectj" expression="com.example..*Service"/>"#,
    r#"<context:exclude-filter type="regex" expression=".*\$Internal"/>"#,
    "</context:component-scan>",
    // --- The main bean: every ClassRef site at once.
    r#"<bean id="fullService" class="com.example.Outer$WidgetServiceImpl">"#,
    // constructor-arg@type: inner-class $, generic, array — all raw text.
    // (Attribute values are never entity-decoded by this crate — invariant
    // #4, "span slice == decoded-but-unresolved". A literal `<`/`>` inside
    // a quoted value round-trips exactly as written here too, but that
    // spelling is not well-formed XML — a real corpus must use the
    // entity-encoded `&lt;`/`&gt;` spelling, pinned separately below as its
    // own raw, unresolved value since this crate never resolves entities.)
    r#"<constructor-arg index="0" type="com.example.Outer$Inner" value="x"/>"#,
    r#"<constructor-arg index="1" type="java.util.List<com.example.Widget>" value="y"/>"#,
    r#"<constructor-arg index="2" type="com.example.Widget[]" value="z"/>"#,
    r#"<constructor-arg index="3" type="java.util.List&lt;com.example.Widget&gt;" value="w"/>"#,
    // qualifier@type: another inner-class $ name.
    r#"<qualifier type="com.example.Qualifiers$Custom" value="main"/>"#,
    // collection value-type (list) + <value type=> nested inside it.
    r#"<property name="items">"#,
    r#"<list value-type="com.example.ListItem">"#,
    r#"<value type="com.example.ValueClass">literal text</value>"#,
    "</list>",
    "</property>",
    // collection value-type (set).
    r#"<property name="codes"><set value-type="com.example.SetItem"/></property>"#,
    // collection value-type (array collection, not to be confused with the
    // ctor-arg's array *type name* above).
    r#"<property name="rows"><array value-type="com.example.ArrayItem"/></property>"#,
    // collection key-type + value-type (map), plus a per-entry value-type
    // override distinct from the map-wide one.
    r#"<property name="lookup">"#,
    r#"<map key-type="java.lang.String" value-type="com.example.MapValue">"#,
    r#"<entry key="a" value="1"/>"#,
    r#"<entry key="b" value="2" value-type="com.example.EntryOverride"/>"#,
    "</map>",
    "</property>",
    // --- Exclusion site 2: replaced-method arg-type — the `match=` form,
    // the arg-type text-content ruling's text-content form, and a
    // whitespace-only `match=` that must fall back to its sibling text
    // body instead of being recorded as-is (real Spring's own
    // `StringUtils.hasText` precedence — a `ClassRef` site has no such
    // attribute/text-body fallback dance, which is the behavioral tell
    // that this field was never funneled through `ClassRef` construction).
    r#"<replaced-method name="computeValue" replacer="replacerBean">"#,
    r#"<arg-type match="java.lang.String"/>"#,
    "<arg-type>com.example.ArgType</arg-type>",
    "<arg-type match=\"   \">com.example.WhitespaceMatchFallsBack</arg-type>",
    "</replaced-method>",
    "</bean>",
    r#"<bean id="replacerBean" class="com.example.ReplacerImpl"/>"#,
    // --- Edge case: a non-FQN "alias" raw class name — preserved verbatim,
    // no FQN validation/resolution.
    r#"<bean id="aliasedBean" class="MyServiceAlias"/>"#,
    "</beans>",
);

#[test]
fn i1_bean_class_populates_including_inner_class_dollar_and_non_fqn_alias_raw() {
    let beans_file = parse_ok(SOURCE);

    let full_service = find_bean(&beans_file, "fullService");
    assert_eq!(
        full_service.class.as_ref().map(|c| c.value.raw.as_str()),
        Some("com.example.Outer$WidgetServiceImpl"),
        "class= must land verbatim, inner-class $ included"
    );

    let aliased = find_bean(&beans_file, "aliasedBean");
    assert_eq!(
        aliased.class.as_ref().map(|c| c.value.raw.as_str()),
        Some("MyServiceAlias"),
        "a non-FQN alias-shaped class= is still a ClassRef, stored raw with no validation"
    );
}

#[test]
fn i1_constructor_arg_type_populates_including_inner_class_generic_and_array() {
    let beans_file = parse_ok(SOURCE);
    let full_service = find_bean(&beans_file, "fullService");
    assert_eq!(full_service.constructor_args.len(), 4);

    let raws: Vec<&str> = full_service
        .constructor_args
        .iter()
        .map(|arg| {
            arg.type_ref
                .as_ref()
                .map(|t| t.value.raw.as_str())
                .expect("every ctor-arg in this fixture declares type=")
        })
        .collect();
    assert_eq!(
        raws,
        vec![
            "com.example.Outer$Inner",
            "java.util.List<com.example.Widget>",
            "com.example.Widget[]",
            "java.util.List&lt;com.example.Widget&gt;",
        ],
        "inner-class $, generic <>, and array [] suffixes must all pass through verbatim — \
         including the entity-encoded generic spelling a well-formed corpus must actually use, \
         which must land raw and unresolved (invariant #4: this crate never resolves entities)"
    );
}

#[test]
fn i1_qualifier_type_populates() {
    let beans_file = parse_ok(SOURCE);
    let full_service = find_bean(&beans_file, "fullService");
    assert_eq!(full_service.qualifiers.len(), 1);
    assert_eq!(
        full_service.qualifiers[0]
            .type_
            .as_ref()
            .map(|t| t.value.raw.as_str()),
        Some("com.example.Qualifiers$Custom")
    );
}

#[test]
fn i1_collection_value_type_and_value_type_child_populate() {
    let beans_file = parse_ok(SOURCE);
    let full_service = find_bean(&beans_file, "fullService");

    let items_prop = full_service
        .properties
        .iter()
        .find(|p| p.name.value == "items")
        .expect("<property name=\"items\">");
    let InjectValue::Collection(collection) = &items_prop.value else {
        panic!("expected a Collection value for 'items'");
    };
    let Collection::List {
        items, value_type, ..
    } = &collection.value
    else {
        panic!("expected a List collection for 'items'");
    };
    assert_eq!(
        value_type.as_ref().map(|t| t.value.raw.as_str()),
        Some("com.example.ListItem"),
        "list value-type= must populate"
    );
    assert_eq!(items.len(), 1);
    let InjectValue::Value(value_lit) = &items[0] else {
        panic!("expected a <value> literal inside the list");
    };
    assert_eq!(
        value_lit.value_type.as_ref().map(|t| t.value.raw.as_str()),
        Some("com.example.ValueClass"),
        "<value type=...> must populate"
    );
    assert_eq!(value_lit.text.value, "literal text");
}

#[test]
fn i1_set_and_array_collection_value_type_populate() {
    let beans_file = parse_ok(SOURCE);
    let full_service = find_bean(&beans_file, "fullService");

    let codes_prop = full_service
        .properties
        .iter()
        .find(|p| p.name.value == "codes")
        .expect("<property name=\"codes\">");
    let InjectValue::Collection(collection) = &codes_prop.value else {
        panic!("expected a Collection value for 'codes'");
    };
    let Collection::Set { value_type, .. } = &collection.value else {
        panic!("expected a Set collection for 'codes'");
    };
    assert_eq!(
        value_type.as_ref().map(|t| t.value.raw.as_str()),
        Some("com.example.SetItem")
    );

    let rows_prop = full_service
        .properties
        .iter()
        .find(|p| p.name.value == "rows")
        .expect("<property name=\"rows\">");
    let InjectValue::Collection(collection) = &rows_prop.value else {
        panic!("expected a Collection value for 'rows'");
    };
    let Collection::Array { value_type, .. } = &collection.value else {
        panic!("expected an Array collection for 'rows'");
    };
    assert_eq!(
        value_type.as_ref().map(|t| t.value.raw.as_str()),
        Some("com.example.ArrayItem")
    );
}

#[test]
fn i1_map_key_type_and_value_type_populate_map_wide_and_per_entry_are_distinct() {
    let beans_file = parse_ok(SOURCE);
    let full_service = find_bean(&beans_file, "fullService");

    let lookup_prop = full_service
        .properties
        .iter()
        .find(|p| p.name.value == "lookup")
        .expect("<property name=\"lookup\">");
    let InjectValue::Collection(collection) = &lookup_prop.value else {
        panic!("expected a Collection value for 'lookup'");
    };
    let Collection::Map {
        entries,
        key_type,
        value_type,
        ..
    } = &collection.value
    else {
        panic!("expected a Map collection for 'lookup'");
    };
    assert_eq!(
        key_type.as_ref().map(|t| t.value.raw.as_str()),
        Some("java.lang.String"),
        "map key-type= must populate"
    );
    assert_eq!(
        value_type.as_ref().map(|t| t.value.raw.as_str()),
        Some("com.example.MapValue"),
        "map-wide value-type= must populate"
    );
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].value_type, None,
        "an entry with no value-type= of its own must not inherit the map-wide one"
    );
    assert_eq!(
        entries[1].value_type.as_ref().map(|t| t.value.raw.as_str()),
        Some("com.example.EntryOverride"),
        "a per-entry value-type= is distinct from (and overrides display of) the map-wide one"
    );
}

#[test]
fn i1_scan_filter_expression_is_excluded_stays_raw_string_never_classref() {
    let beans_file = parse_ok(SOURCE);
    assert_eq!(beans_file.component_scans.len(), 1);
    let scan = &beans_file.component_scans[0].value;

    assert_eq!(scan.include_filters.len(), 1);
    assert_eq!(
        scan.include_filters[0].expression.value, "com.example..*Service",
        "a wildcard scan-filter pattern (not a legal FQN) must pass through unmodified — \
         proof it is a raw String field, not funneled through ClassRef construction"
    );

    assert_eq!(scan.exclude_filters.len(), 1);
    assert_eq!(
        scan.exclude_filters[0].expression.value, ".*\\$Internal",
        "a regex pattern containing a literal $ (not an inner-class name) must also pass \
         through unmodified"
    );
}

#[test]
fn i1_replaced_method_arg_type_is_excluded_stays_raw_string_never_classref() {
    let beans_file = parse_ok(SOURCE);
    let full_service = find_bean(&beans_file, "fullService");
    assert_eq!(full_service.replaced_methods.len(), 1);

    let arg_types: Vec<&str> = full_service.replaced_methods[0]
        .value
        .arg_types
        .iter()
        .map(|a| a.value.as_str())
        .collect();
    assert_eq!(
        arg_types,
        vec![
            "java.lang.String",
            "com.example.ArgType",
            "com.example.WhitespaceMatchFallsBack",
        ],
        "match= form, the arg-type text-content ruling's text-content form, and a \
         whitespace-only match= must all land as plain strings in document order — the third \
         entry in particular is the tell that this field was never routed through ClassRef \
         construction: a ClassRef site has no attribute/text-body fallback precedence, so a \
         whitespace-only match= falling back to its sibling text body (rather than being \
         recorded verbatim, or raising a diagnostic) only makes sense for a lenient pattern \
         field like arg_types"
    );
}
