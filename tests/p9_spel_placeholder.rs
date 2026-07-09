//! Unit **P9** — `${}`/`#{}` extraction (SB-13). M0b landed the minimal
//! scope (single-candidate `#{beanRef}`/`${prop}` extraction); M1c-expr
//! added the heavier harvesting on top (nested-key/nested-SpEL-ref
//! collection, the quote-aware nested-brace carry-over fix) — see
//! `src/inject_value.rs`'s own `extract_from_slice` doc comment for the
//! exact rule.
//!
//! Per build plan's P9 row, this augments U5a's single `ValueLit` builder
//! (`inject_value::build_value_lit`) rather than a separate post-walk —
//! `src/inject_value.rs`'s own `#[cfg(test)] mod tests` carries the real
//! per-case extraction table (dollar/hash/nested/CDATA/literal-`$`-or-`#`/
//! quote-aware/unterminated), directly against `build_value_lit` and its
//! two call sites (`value_lit_from_element`/`value_lit_from_attr`) — a seam
//! not visible from this external integration-test binary, the same
//! situation `tests/u5a_inject_value.rs`'s own doc comment documents.
//!
//! This file exercises the one thing only observable from *outside* the
//! crate: that the extraction actually reaches `ValueLit.placeholders`/
//! `spel_refs` **through every production call site** — `<property
//! value=>`, a `<constructor-arg>` literal, a `<list>` item, a p-namespace
//! literal attribute, and a `<prop key=>` entry — proving the "single
//! builder, no separate walk" design actually pays off uniformly rather
//! than only working for the one call site a unit test happened to cover.
//! Also carries this unit's ALSO-fix: the U1 `check_unterminated_
//! placeholders` quote-awareness fix, observed end-to-end here as "no
//! `UnterminatedPlaceholder` diagnostic" rather than through
//! `events::scan_braced_expr` directly (`src/events.rs`'s own test module
//! owns that seam).

use beans_xml::{Collection, DiagCode, InjectValue};

fn parse_ok(source: &str) -> beans_xml::BeansFile {
    beans_xml::parse(source).beans.expect("beans root")
}

fn only_bean(source: &str) -> beans_xml::Bean {
    let mut beans = parse_ok(source).beans;
    assert_eq!(beans.len(), 1, "expected exactly one top-level <bean>");
    beans.remove(0)
}

fn value_lit(value: &InjectValue) -> &beans_xml::ValueLit {
    match value {
        InjectValue::Value(vl) => vl,
        other => panic!("expected InjectValue::Value, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// `<property value=>` — the most common real-world SB-13 site.
// ---------------------------------------------------------------------

#[test]
fn sb13_property_value_attr_placeholder_extracted_end_to_end() {
    let source = concat!(
        "<beans>",
        r#"<bean id="a" class="com.example.Widget">"#,
        r#"<property name="url" value="${db.url}"/>"#,
        "</bean>",
        "</beans>"
    );
    let bean = only_bean(source);
    let vl = value_lit(&bean.properties[0].value);
    assert_eq!(
        vl.placeholders
            .iter()
            .map(|p| p.value.as_str())
            .collect::<Vec<_>>(),
        vec!["db.url"]
    );
    assert!(beans_xml::parse(source).diagnostics.is_empty());
}

#[test]
fn sb13_nested_placeholder_and_spel_harvested_end_to_end() {
    // M1c-expr: nested `${a.${b}}` and a richer `#{beanA.m(#{other})}`
    // SpEL form, reaching `ValueLit` through the real `<property value=>`
    // production call site.
    let source = concat!(
        "<beans>",
        r#"<bean id="a" class="com.example.Widget">"#,
        r#"<property name="url" value="${a.${b}}"/>"#,
        r#"<property name="target"><value>#{beanA.m(#{other})}</value></property>"#,
        "</bean>",
        "</beans>"
    );
    let bean = only_bean(source);
    let url_vl = value_lit(&bean.properties[0].value);
    assert_eq!(
        url_vl
            .placeholders
            .iter()
            .map(|p| p.value.as_str())
            .collect::<Vec<_>>(),
        vec!["a.${b}", "b"]
    );
    let target_vl = value_lit(&bean.properties[1].value);
    assert_eq!(
        target_vl
            .spel_refs
            .iter()
            .map(|s| s.value.as_str())
            .collect::<Vec<_>>(),
        vec!["beanA", "other"]
    );
}

#[test]
fn sb13_property_value_child_spel_ref_extracted_end_to_end() {
    let source = concat!(
        "<beans>",
        r#"<bean id="a" class="com.example.Widget">"#,
        r#"<property name="target"><value>#{targetBean.pick()}</value></property>"#,
        "</bean>",
        "</beans>"
    );
    let bean = only_bean(source);
    let vl = value_lit(&bean.properties[0].value);
    assert_eq!(
        vl.spel_refs
            .iter()
            .map(|s| s.value.as_str())
            .collect::<Vec<_>>(),
        vec!["targetBean"]
    );
}

// ---------------------------------------------------------------------
// `<constructor-arg value=>`.
// ---------------------------------------------------------------------

#[test]
fn sb13_constructor_arg_value_attr_placeholder_extracted_end_to_end() {
    let source = concat!(
        "<beans>",
        r#"<bean id="a" class="com.example.Widget">"#,
        r#"<constructor-arg value="${timeout}"/>"#,
        "</bean>",
        "</beans>"
    );
    let bean = only_bean(source);
    let vl = value_lit(&bean.constructor_args[0].value);
    assert_eq!(
        vl.placeholders
            .iter()
            .map(|p| p.value.as_str())
            .collect::<Vec<_>>(),
        vec!["timeout"]
    );
}

// ---------------------------------------------------------------------
// `<list>` item — collection literal (U5b) reuses the same builder.
// ---------------------------------------------------------------------

#[test]
fn sb13_list_item_value_placeholder_extracted_end_to_end() {
    let source = concat!(
        "<beans>",
        r#"<bean id="a" class="com.example.Widget">"#,
        r#"<property name="hosts"><list>"#,
        r#"<value>${host1}</value>"#,
        r#"<value>${host2}</value>"#,
        "</list></property>",
        "</bean>",
        "</beans>"
    );
    let bean = only_bean(source);
    let InjectValue::Collection(collection) = &bean.properties[0].value else {
        panic!("expected InjectValue::Collection");
    };
    let Collection::List { items, .. } = &collection.value else {
        panic!("expected Collection::List");
    };
    let keys: Vec<&str> = items
        .iter()
        .map(|item| value_lit(item).placeholders[0].value.as_str())
        .collect();
    assert_eq!(keys, vec!["host1", "host2"]);
}

// ---------------------------------------------------------------------
// `<prop key=>` entry inside `<props>`.
// ---------------------------------------------------------------------

#[test]
fn sb13_prop_entry_value_placeholder_extracted_end_to_end() {
    let source = concat!(
        "<beans>",
        r#"<bean id="a" class="com.example.Widget">"#,
        r#"<property name="settings"><props>"#,
        r#"<prop key="driver">${db.driver}</prop>"#,
        "</props></property>",
        "</bean>",
        "</beans>"
    );
    let bean = only_bean(source);
    let InjectValue::Collection(collection) = &bean.properties[0].value else {
        panic!("expected InjectValue::Collection");
    };
    let Collection::Props { entries, .. } = &collection.value else {
        panic!("expected Collection::Props");
    };
    assert_eq!(
        entries[0]
            .value
            .placeholders
            .iter()
            .map(|p| p.value.as_str())
            .collect::<Vec<_>>(),
        vec!["db.driver"]
    );
}

// ---------------------------------------------------------------------
// p-namespace literal attribute (P2) — "centralized lock" build-plan case.
// ---------------------------------------------------------------------

#[test]
fn sb13_p_namespace_literal_attr_placeholder_extracted_end_to_end() {
    let source = concat!(
        "<beans xmlns:p=\"http://www.springframework.org/schema/p\">",
        r#"<bean id="a" class="com.example.Widget" p:url="${db.url}"/>"#,
        "</beans>"
    );
    let bean = only_bean(source);
    assert_eq!(bean.properties.len(), 1);
    let vl = value_lit(&bean.properties[0].value);
    assert_eq!(
        vl.placeholders
            .iter()
            .map(|p| p.value.as_str())
            .collect::<Vec<_>>(),
        vec!["db.url"]
    );
}

// ---------------------------------------------------------------------
// Also-fix: U1's quote-aware unterminated-placeholder scan, end-to-end.
// ---------------------------------------------------------------------

#[test]
fn sb13_quote_aware_spel_string_literal_brace_no_unterminated_diagnostic_end_to_end() {
    for source in [
        r##"<beans><bean id="a" class="com.example.Widget"><property name="p" value="#{'{'}"/></bean></beans>"##,
        r##"<beans><bean id="a" class="com.example.Widget"><property name="p" value="#{map['{']}"/></bean></beans>"##,
    ] {
        let result = beans_xml::parse(source);
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| d.code == DiagCode::UnterminatedPlaceholder),
            "quote-aware brace literal must not be flagged for {source}: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn sb13_dollar_placeholder_with_apostrophe_default_no_unterminated_diagnostic_end_to_end() {
    // Quote-as-string-delimiter is a SpEL (`#{}`) notion only. A `${}`
    // property placeholder's default value can contain an apostrophe or
    // double quote as an ordinary literal character (Spring's
    // `${prop:default}` syntax) — that must not be mistaken for a SpEL
    // string-literal opener, which would swallow the real closing `}` and
    // both misdiagnose UnterminatedPlaceholder and drop the placeholder key.
    let source = concat!(
        "<beans>",
        r#"<bean id="a" class="com.example.Widget">"#,
        r#"<property name="p" value="${admin.name:O'Reilly}"/>"#,
        "</bean>",
        "</beans>"
    );
    let result = beans_xml::parse(source);
    assert!(
        result.diagnostics.is_empty(),
        "balanced ${{}} with an apostrophe default must not be diagnosed: {:?}",
        result.diagnostics
    );
    let bean = only_bean(source);
    let vl = value_lit(&bean.properties[0].value);
    assert_eq!(
        vl.placeholders
            .iter()
            .map(|p| p.value.as_str())
            .collect::<Vec<_>>(),
        vec!["admin.name:O'Reilly"]
    );
}

#[test]
fn sb13_genuinely_unterminated_placeholder_still_diagnosed_end_to_end() {
    let source = concat!(
        "<beans>",
        r#"<bean id="a" class="com.example.Widget">"#,
        r#"<property name="p" value="${unterminated"/>"#,
        "</bean>",
        "</beans>"
    );
    let result = beans_xml::parse(source);
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == DiagCode::UnterminatedPlaceholder));
    let bean = only_bean(source);
    let vl = value_lit(&bean.properties[0].value);
    assert!(vl.placeholders.is_empty());
    assert_eq!(vl.text.value, "${unterminated");
}

// ---------------------------------------------------------------------
// Literal `$`/`#` without a brace — never extracted.
// ---------------------------------------------------------------------

#[test]
fn sb13_literal_dollar_and_hash_not_extracted_end_to_end() {
    let source = concat!(
        "<beans>",
        r#"<bean id="a" class="com.example.Widget">"#,
        r#"<property name="p" value="costs $100, see #ticket-42"/>"#,
        "</bean>",
        "</beans>"
    );
    let bean = only_bean(source);
    let vl = value_lit(&bean.properties[0].value);
    assert!(vl.placeholders.is_empty());
    assert!(vl.spel_refs.is_empty());
    assert!(beans_xml::parse(source).diagnostics.is_empty());
}

// ---------------------------------------------------------------------
// Multi-segment `<value>` text (comment-split / CDATA-mixed) — invariant
// #4 (span slice == decoded text) must hold for every harvested
// `placeholders`/`spel_refs` entry, not just the single-text-node case.
// Regression coverage for the cold-review finding that `extract_from_slice`
// used to compute offsets against `dispatch::element_text`'s merged
// (byte-discontiguous) string.
// ---------------------------------------------------------------------

#[test]
fn sb13_placeholder_after_comment_split_reslices_to_its_own_key_end_to_end() {
    // A comment between two `${}` openers splits `<value>`'s text into two
    // separate raw text runs (events.rs never puts a comment into the
    // tree) — `${x}` and `${y}` never become byte-adjacent in the source,
    // so the second placeholder's span must land on "y" in the source,
    // not several bytes earlier inside the comment.
    let source = "<beans><bean id=\"a\" class=\"com.example.Widget\">\
        <property name=\"p\"><value>${x}<!-- c -->${y}</value></property>\
        </bean></beans>";
    let bean = only_bean(source);
    let vl = value_lit(&bean.properties[0].value);
    let keys: Vec<&str> = vl.placeholders.iter().map(|p| p.value.as_str()).collect();
    assert_eq!(keys, vec!["x", "y"]);
    for p in &vl.placeholders {
        let reslice = &source[p.span.start as usize..p.span.end as usize];
        assert_eq!(
            reslice, p.value,
            "placeholder span must reslice to its own decoded key (invariant #4), got {reslice:?}"
        );
    }
}

#[test]
fn sb13_placeholder_inside_cdata_after_plain_text_reslices_to_its_own_key_end_to_end() {
    // Plain text followed by a CDATA section is two raw text runs too —
    // the `<![CDATA[`/`]]>` delimiters are real source bytes that never
    // appear in either run's decoded `value`, so a placeholder inside the
    // CDATA run must not have its span computed as an offset into the
    // *merged* text (which would land inside the `<![CDATA[` delimiter
    // itself).
    let source = "<beans><bean id=\"a\" class=\"com.example.Widget\">\
        <property name=\"p\"><value>pre<![CDATA[${key}]]></value></property>\
        </bean></beans>";
    let bean = only_bean(source);
    let vl = value_lit(&bean.properties[0].value);
    assert_eq!(vl.placeholders.len(), 1);
    let p = &vl.placeholders[0];
    assert_eq!(p.value, "key");
    let reslice = &source[p.span.start as usize..p.span.end as usize];
    assert_eq!(
        reslice, p.value,
        "placeholder span must reslice to its own decoded key (invariant #4), got {reslice:?}"
    );
}

#[test]
fn sb13_placeholder_split_by_comment_layer_agreement_no_spurious_harvest_end_to_end() {
    // Layer-agreement regression: `${a<!-- -->}` splits into raw text runs
    // "${a" (no closing brace in that run) and "}" (no opener). U1
    // (`check_unterminated_placeholders`, per-raw-text-run) already flags
    // the first run `UnterminatedPlaceholder`; P9 must agree that nothing
    // well-formed was harvested here rather than stitching the two runs
    // back together into a spurious closed `"a"` placeholder.
    let source = "<beans><bean id=\"a\" class=\"com.example.Widget\">\
        <property name=\"p\"><value>${a<!-- -->}</value></property>\
        </bean></beans>";
    let result = beans_xml::parse(source);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.code == DiagCode::UnterminatedPlaceholder),
        "U1 must still flag the unterminated opener: {:?}",
        result.diagnostics
    );
    let bean = only_bean(source);
    let vl = value_lit(&bean.properties[0].value);
    assert!(
        vl.placeholders.is_empty(),
        "P9 must not harvest a placeholder U1 already called unterminated: {:?}",
        vl.placeholders
    );
}

// ---------------------------------------------------------------------
// Entity-split harvesting regression (M1c fix): `&amp;` (and any other
// entity/character reference) arrives as its own raw text run between two
// ordinary text runs — byte-contiguous with both, unlike the comment-/
// CDATA-split cases just above. An expression spanning across one
// (`#{flagA &amp;&amp; flagB}`, `${url:x&amp;y}`) must still harvest as one
// whole expression, and must not spuriously diagnose
// `UnterminatedPlaceholder` on the truncated pieces either. Fixed by
// coalescing byte-contiguous text runs before scanning, in both U1's
// `check_unterminated_placeholders` (`events::build_tree`) and this unit's
// own `extract_placeholders_and_spel_refs` (`inject_value.rs`).
// ---------------------------------------------------------------------

#[test]
fn sb13_entity_split_spel_ref_harvested_whole_in_element_text_end_to_end() {
    let source = "<beans><bean id=\"a\" class=\"com.example.Widget\">\
        <property name=\"p\"><value>#{flagA &amp;&amp; flagB}</value></property>\
        </bean></beans>";
    let result = beans_xml::parse(source);
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|d| d.code == DiagCode::UnterminatedPlaceholder),
        "an entity-split but well-formed expression must not be flagged unterminated: {:?}",
        result.diagnostics
    );
    let bean = only_bean(source);
    let vl = value_lit(&bean.properties[0].value);
    assert_eq!(
        vl.spel_refs
            .iter()
            .map(|s| s.value.as_str())
            .collect::<Vec<_>>(),
        vec!["flagA"],
        "entity-split #{{}} must still harvest exactly one spel_ref"
    );
}

#[test]
fn sb13_entity_split_dollar_placeholder_harvested_whole_in_ref_default_end_to_end() {
    // `${url:...&amp;b=2}` — a `${}` default value containing an entity
    // reference, the other repro shape from the same regression.
    let source = "<beans><bean id=\"a\" class=\"com.example.Widget\">\
        <property name=\"p\"><value>${url:http://x?a=1&amp;b=2}</value></property>\
        </bean></beans>";
    let result = beans_xml::parse(source);
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|d| d.code == DiagCode::UnterminatedPlaceholder),
        "an entity-split but well-formed ${{}} must not be flagged unterminated: {:?}",
        result.diagnostics
    );
    let bean = only_bean(source);
    let vl = value_lit(&bean.properties[0].value);
    assert_eq!(
        vl.placeholders
            .iter()
            .map(|p| p.value.as_str())
            .collect::<Vec<_>>(),
        vec!["url:http://x?a=1&amp;b=2"],
        "entity-split ${{}} must still harvest the whole key"
    );
}

#[test]
fn sb13_entity_split_dollar_placeholder_harvested_whole_in_plain_value_end_to_end() {
    // Same shape, but as the sole content of a plain `<value>` (not a
    // ref-default) — the `value=` shorthand-attribute form (single already-
    // decoded `Spanned<String>`, no entity splitting possible) is covered by
    // the attribute-form regression guard below; this is the element-text
    // path with no leading literal prefix before the placeholder.
    let source = "<beans><bean id=\"a\" class=\"com.example.Widget\">\
        <property name=\"p\"><value>${x&amp;y}</value></property>\
        </bean></beans>";
    let result = beans_xml::parse(source);
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|d| d.code == DiagCode::UnterminatedPlaceholder),
        "{:?}",
        result.diagnostics
    );
    let bean = only_bean(source);
    let vl = value_lit(&bean.properties[0].value);
    assert_eq!(
        vl.placeholders
            .iter()
            .map(|p| p.value.as_str())
            .collect::<Vec<_>>(),
        vec!["x&amp;y"]
    );
}

#[test]
fn sb13_entity_split_regression_attribute_form_unaffected() {
    // Regression guard: the `value=`/`ref=` shorthand-attribute form was
    // never broken by this bug in the first place (an attribute value is
    // one already-decoded `Spanned<String>`, never split by an entity
    // reference the way element text/CDATA runs are) — pin that it still
    // works exactly as before, so a fix to the element-text path can't
    // accidentally regress the attribute path.
    let source = "<beans><bean id=\"a\" class=\"com.example.Widget\">\
        <property name=\"p\" value=\"#{flagA &amp;&amp; flagB}\"/>\
        </bean></beans>";
    let result = beans_xml::parse(source);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let bean = only_bean(source);
    let vl = value_lit(&bean.properties[0].value);
    assert_eq!(
        vl.spel_refs
            .iter()
            .map(|s| s.value.as_str())
            .collect::<Vec<_>>(),
        vec!["flagA"]
    );
}
