//! Unit **U4** — `<bean>` core attributes (SB-02) + the frozen bean-child
//! dispatch skeleton. Test design per the internal build plan's
//! U4 row: snapshot of a representative bean + a table of edge cases
//! (id-less bean, effective-name rule, duplicate id, class/parent/factory
//! absence with the `abstract` exemption, multiple `name` tokens, DTD
//! `singleton` normalization) + `UnknownElement` for an unrecognized
//! beans-ns child inside a `<bean>`.
//!
//! `bean::parse_bean`/`dispatch::*` are all `pub(crate)` — not visible from
//! this external integration-test binary — so every test here goes through
//! the public API (`beans_xml::parse`) only.

use beans_xml::{Bean, DiagCode, RefKind};

fn parse_ok(source: &str) -> beans_xml::BeansFile {
    beans_xml::parse(source).beans.expect("beans root")
}

fn only_bean(source: &str) -> Bean {
    let mut beans = parse_ok(source).beans;
    assert_eq!(beans.len(), 1, "expected exactly one top-level <bean>");
    beans.remove(0)
}

// ---------------------------------------------------------------------
// Snapshot: a representative <bean> exercising every SB-02 core attribute.
// ---------------------------------------------------------------------

#[test]
fn sb02_representative_bean_snapshot() {
    let source = concat!(
        "<beans>",
        "<bean id=\"widgetService\" name=\"widgetSvc, legacyWidgetSvc\" ",
        "class=\"com.example.WidgetService\" scope=\"singleton\" ",
        "lazy-init=\"true\" primary=\"true\" autowire=\"byName\" ",
        "autowire-candidate=\"false\" depends-on=\"dataSource, cacheManager\" ",
        "init-method=\"init\" destroy-method=\"destroy\">",
        "<description>Wires the widget service.</description>",
        "</bean>",
        "</beans>"
    );
    let bean = only_bean(source);

    assert_eq!(
        bean.id.as_ref().map(|s| s.value.as_str()),
        Some("widgetService")
    );
    assert_eq!(
        bean.names
            .iter()
            .map(|n| n.value.as_str())
            .collect::<Vec<_>>(),
        vec!["widgetSvc", "legacyWidgetSvc"]
    );
    assert_eq!(
        bean.class.as_ref().map(|c| c.value.raw.as_str()),
        Some("com.example.WidgetService")
    );
    assert_eq!(bean.parent, None);
    assert_eq!(
        bean.scope.as_ref().map(|s| s.value.as_str()),
        Some("singleton")
    );
    assert!(!bean.abstract_);
    assert_eq!(bean.lazy_init, Some(true));
    assert!(bean.primary);
    assert_eq!(
        bean.autowire.as_ref().map(|s| s.value.as_str()),
        Some("byName")
    );
    assert_eq!(bean.autowire_candidate, Some(false));
    assert_eq!(
        bean.depends_on
            .iter()
            .map(|d| d.value.raw.as_str())
            .collect::<Vec<_>>(),
        vec!["dataSource", "cacheManager"]
    );
    assert!(bean
        .depends_on
        .iter()
        .all(|d| d.value.kind == RefKind::Bean));
    assert_eq!(bean.factory_bean, None);
    assert_eq!(bean.factory_method, None);
    assert_eq!(
        bean.init_method.as_ref().map(|s| s.value.as_str()),
        Some("init")
    );
    assert_eq!(
        bean.destroy_method.as_ref().map(|s| s.value.as_str()),
        Some("destroy")
    );
    assert_eq!(
        bean.description.as_ref().map(|s| s.value.as_str()),
        Some("Wires the widget service.")
    );
    assert!(bean.properties.is_empty());
    assert!(bean.constructor_args.is_empty());
    assert!(bean.lookup_methods.is_empty());
    assert!(bean.replaced_methods.is_empty());
    assert!(bean.qualifiers.is_empty());
    assert!(bean.decorators.is_empty());
    assert!(bean.meta.is_empty());

    // Span covers the whole element (opening tag through </bean>).
    assert_eq!(
        &source[bean.span.start as usize..bean.span.end as usize],
        concat!(
            "<bean id=\"widgetService\" name=\"widgetSvc, legacyWidgetSvc\" ",
            "class=\"com.example.WidgetService\" scope=\"singleton\" ",
            "lazy-init=\"true\" primary=\"true\" autowire=\"byName\" ",
            "autowire-candidate=\"false\" depends-on=\"dataSource, cacheManager\" ",
            "init-method=\"init\" destroy-method=\"destroy\">",
            "<description>Wires the widget service.</description>",
            "</bean>"
        )
    );

    let r = beans_xml::parse(source);
    assert!(
        r.diagnostics.is_empty(),
        "a fully-formed representative bean must not raise diagnostics: {:?}",
        r.diagnostics
    );
}

// ---------------------------------------------------------------------
// id-less bean + effective-name rule (id, else first `name` token).
// ---------------------------------------------------------------------

/// Mirrors `Bean::names`'s own doc comment: "effective registered name =
/// id, else the first names token" — a consumer-side derivation, not a stored field. Pinning
/// it here as a local helper is what actually tests that `id`/`names` are
/// populated with everything a consumer needs to compute it correctly.
fn effective_name(bean: &Bean) -> Option<&str> {
    bean.id
        .as_ref()
        .map(|s| s.value.as_str())
        .or_else(|| bean.names.first().map(|n| n.value.as_str()))
}

#[test]
fn sb02_bean_without_id_still_parses_with_name_only() {
    let bean = only_bean("<beans><bean name=\"aliasOnly\" class=\"com.example.Widget\"/></beans>");
    assert_eq!(bean.id, None);
    assert_eq!(
        bean.names
            .iter()
            .map(|n| n.value.as_str())
            .collect::<Vec<_>>(),
        vec!["aliasOnly"]
    );
}

#[test]
fn sb02_effective_name_prefers_id_over_name() {
    let bean = only_bean(
        "<beans><bean id=\"realId\" name=\"alias\" class=\"com.example.Widget\"/></beans>",
    );
    assert_eq!(effective_name(&bean), Some("realId"));
}

#[test]
fn sb02_effective_name_falls_back_to_first_name_token_when_id_absent() {
    let bean = only_bean(
        "<beans><bean name=\"firstAlias, secondAlias\" class=\"com.example.Widget\"/></beans>",
    );
    assert_eq!(effective_name(&bean), Some("firstAlias"));
}

#[test]
fn sb02_effective_name_is_none_when_neither_id_nor_name_present() {
    let bean = only_bean("<beans><bean class=\"com.example.Widget\"/></beans>");
    assert_eq!(effective_name(&bean), None);
}

// ---------------------------------------------------------------------
// Multiple `name` tokens — comma/semicolon/whitespace-separated, id never
// duplicated into `names`.
// ---------------------------------------------------------------------

#[test]
fn sb02_multiple_names_split_on_comma_semicolon_and_whitespace() {
    let source = "<beans><bean id=\"a\" name=\"b, c;d e\" class=\"com.example.Widget\"/></beans>";
    let bean = only_bean(source);
    assert_eq!(
        bean.names
            .iter()
            .map(|n| n.value.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "c", "d", "e"]
    );
    // id is never duplicated into names (model contract).
    assert!(!bean.names.iter().any(|n| n.value == "a"));
}

#[test]
fn sb02_name_token_spans_slice_back_to_the_source() {
    let source = "<beans><bean id=\"a\" name=\"b, c\" class=\"com.example.Widget\"/></beans>";
    let bean = only_bean(source);
    for token in &bean.names {
        assert_eq!(
            &source[token.span.start as usize..token.span.end as usize],
            token.value.as_str()
        );
    }
}

/// Regression guard for `split_name_tokens`'s byte-safety claim (its own
/// doc comment argues separator bytes never land mid-character, so
/// byte-offset arithmetic is safe on non-ASCII tokens like Korean bean
/// names) — mixes Korean and ASCII tokens so a later token's offset only
/// comes out right if the earlier multi-byte token's *byte* length (not
/// its *char* count) was added to the running index.
#[test]
fn sb02_name_token_spans_slice_back_to_the_source_with_korean_multibyte_tokens() {
    let source =
        "<beans><bean id=\"a\" name=\"가나다, b, 라마\" class=\"com.example.Widget\"/></beans>";
    let bean = only_bean(source);
    assert_eq!(
        bean.names
            .iter()
            .map(|n| n.value.as_str())
            .collect::<Vec<_>>(),
        vec!["가나다", "b", "라마"]
    );
    for token in &bean.names {
        assert_eq!(
            &source[token.span.start as usize..token.span.end as usize],
            token.value.as_str()
        );
    }
}

#[test]
fn sb02_name_with_redundant_separators_produces_no_empty_tokens() {
    let source = "<beans><bean name=\" a ,, b \" class=\"com.example.Widget\"/></beans>";
    let bean = only_bean(source);
    assert_eq!(
        bean.names
            .iter()
            .map(|n| n.value.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
}

// ---------------------------------------------------------------------
// Duplicate bean id within a single <beans> block — both preserved, plus
// DuplicateBeanId.
// ---------------------------------------------------------------------

#[test]
fn sb02_duplicate_bean_id_within_one_beans_block_is_diagnosed_but_both_preserved() {
    let source = concat!(
        "<beans>",
        "<bean id=\"dup\" class=\"com.example.First\"/>",
        "<bean id=\"dup\" class=\"com.example.Second\"/>",
        "</beans>"
    );
    let r = beans_xml::parse(source);
    let beans = r.beans.expect("beans root");
    assert_eq!(
        beans.beans.len(),
        2,
        "both duplicate-id beans must be preserved"
    );
    assert_eq!(
        beans.beans[0].class.as_ref().map(|c| c.value.raw.as_str()),
        Some("com.example.First")
    );
    assert_eq!(
        beans.beans[1].class.as_ref().map(|c| c.value.raw.as_str()),
        Some("com.example.Second")
    );
    let dup_diags: Vec<_> = r
        .diagnostics
        .iter()
        .filter(|d| d.code == DiagCode::DuplicateBeanId)
        .collect();
    assert_eq!(
        dup_diags.len(),
        1,
        "exactly one DuplicateBeanId diagnostic: {:?}",
        r.diagnostics
    );
}

#[test]
fn sb02_distinct_bean_ids_within_one_beans_block_are_not_diagnosed() {
    let source = concat!(
        "<beans>",
        "<bean id=\"a\" class=\"com.example.First\"/>",
        "<bean id=\"b\" class=\"com.example.Second\"/>",
        "</beans>"
    );
    let r = beans_xml::parse(source);
    assert!(
        !r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::DuplicateBeanId),
        "distinct ids must not be flagged: {:?}",
        r.diagnostics
    );
}

// ---------------------------------------------------------------------
// class/parent/factory-bean absence — BeanWithoutClassOrParent, with the
// `abstract` template exemption.
// ---------------------------------------------------------------------

#[test]
fn sb02_bean_without_class_parent_or_factory_bean_is_diagnosed() {
    let r = beans_xml::parse("<beans><bean id=\"noClass\"/></beans>");
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::BeanWithoutClassOrParent),
        "expected BeanWithoutClassOrParent: {:?}",
        r.diagnostics
    );
}

#[test]
fn sb02_abstract_bean_without_class_parent_or_factory_bean_is_exempt() {
    let r = beans_xml::parse("<beans><bean id=\"template\" abstract=\"true\"/></beans>");
    assert!(
        !r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::BeanWithoutClassOrParent),
        "an abstract template bean must be exempt: {:?}",
        r.diagnostics
    );
    let bean = only_bean("<beans><bean id=\"template\" abstract=\"true\"/></beans>");
    assert!(bean.abstract_);
}

#[test]
fn sb02_bean_with_only_parent_is_not_diagnosed() {
    let r = beans_xml::parse("<beans><bean id=\"child\" parent=\"base\"/></beans>");
    assert!(
        !r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::BeanWithoutClassOrParent),
        "class inherited from parent must be exempt: {:?}",
        r.diagnostics
    );
    let bean = only_bean("<beans><bean id=\"child\" parent=\"base\"/></beans>");
    assert_eq!(bean.parent.as_ref().unwrap().value.raw, "base");
    assert_eq!(bean.parent.as_ref().unwrap().value.kind, RefKind::Bean);
}

#[test]
fn sb02_bean_with_only_factory_bean_is_not_diagnosed() {
    let source =
        "<beans><bean id=\"widget\" factory-bean=\"widgetFactory\" factory-method=\"create\"/></beans>";
    let r = beans_xml::parse(source);
    assert!(
        !r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::BeanWithoutClassOrParent),
        "class inherited from factory-bean must be exempt: {:?}",
        r.diagnostics
    );
    let bean = only_bean(source);
    assert_eq!(
        bean.factory_bean.as_ref().map(|f| f.value.raw.as_str()),
        Some("widgetFactory")
    );
    assert_eq!(
        bean.factory_bean.as_ref().unwrap().value.kind,
        RefKind::Bean
    );
    assert_eq!(
        bean.factory_method.as_ref().map(|m| m.value.as_str()),
        Some("create")
    );
}

#[test]
fn sb02_empty_class_attribute_is_treated_as_absent_not_an_empty_class_ref() {
    // Invariant #5 (ClassRef.raw never empty): a present-but-empty class=
    // must not produce a ClassRef with an empty raw string.
    let bean = only_bean("<beans><bean id=\"x\" class=\"\" parent=\"base\"/></beans>");
    assert_eq!(bean.class, None);
}

#[test]
fn sb02_empty_class_attribute_with_no_parent_or_factory_bean_is_bean_without_class_or_parent() {
    // Distinct from `sb02_empty_class_attribute_is_treated_as_absent_not_an_empty_class_ref`
    // above (which pairs empty class= with parent= to isolate the ClassRef
    // invariant in isolation): here nothing at all supplies a usable class,
    // so `BeanWithoutClassOrParent` itself must also fire, the same as a
    // fully absent class= attribute would.
    let source = "<beans><bean id=\"x\" class=\"\"/></beans>";
    let r = beans_xml::parse(source);
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::BeanWithoutClassOrParent),
        "expected BeanWithoutClassOrParent for empty class= with no parent/factory-bean: {:?}",
        r.diagnostics
    );
    let bean = only_bean(source);
    assert_eq!(bean.class, None);
}

// ---------------------------------------------------------------------
// DTD singleton normalization.
// ---------------------------------------------------------------------

#[test]
fn sb02_dtd_singleton_true_normalizes_to_scope_singleton() {
    let bean = only_bean(
        "<beans><bean id=\"a\" class=\"com.example.Widget\" singleton=\"true\"/></beans>",
    );
    assert_eq!(
        bean.scope.as_ref().map(|s| s.value.as_str()),
        Some("singleton")
    );
}

#[test]
fn sb02_dtd_singleton_false_normalizes_to_scope_prototype() {
    let bean = only_bean(
        "<beans><bean id=\"a\" class=\"com.example.Widget\" singleton=\"false\"/></beans>",
    );
    assert_eq!(
        bean.scope.as_ref().map(|s| s.value.as_str()),
        Some("prototype")
    );
}

#[test]
fn sb02_no_scope_or_singleton_attribute_is_none() {
    let bean = only_bean("<beans><bean id=\"a\" class=\"com.example.Widget\"/></beans>");
    assert_eq!(bean.scope, None);
}

#[test]
fn sb02_scope_attribute_present_is_used_verbatim_not_normalized() {
    let bean = only_bean(
        "<beans><bean id=\"a\" class=\"com.example.Widget\" scope=\"prototype\"/></beans>",
    );
    assert_eq!(
        bean.scope.as_ref().map(|s| s.value.as_str()),
        Some("prototype")
    );
}

#[test]
fn sb02_scope_attribute_wins_over_singleton_when_both_present() {
    let bean = only_bean(concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\" ",
        "scope=\"request\" singleton=\"true\"/></beans>"
    ));
    assert_eq!(
        bean.scope.as_ref().map(|s| s.value.as_str()),
        Some("request")
    );
}

#[test]
fn sb02_singleton_with_unrecognized_value_is_none() {
    let bean = only_bean(
        "<beans><bean id=\"a\" class=\"com.example.Widget\" singleton=\"maybe\"/></beans>",
    );
    assert_eq!(bean.scope, None);
}

// ---------------------------------------------------------------------
// depends-on: multi-token, RefKind::Bean, never an empty raw.
// ---------------------------------------------------------------------

#[test]
fn sb02_depends_on_multiple_tokens_all_ref_kind_bean() {
    let bean = only_bean(concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\" ",
        "depends-on=\"dataSource,cacheManager\"/></beans>"
    ));
    assert_eq!(
        bean.depends_on
            .iter()
            .map(|d| d.value.raw.as_str())
            .collect::<Vec<_>>(),
        vec!["dataSource", "cacheManager"]
    );
    assert!(bean
        .depends_on
        .iter()
        .all(|d| d.value.kind == RefKind::Bean));
}

/// Same byte-safety regression guard as
/// `sb02_name_token_spans_slice_back_to_the_source_with_korean_multibyte_tokens`,
/// for `depends-on=` — every existing `depends-on` test here is ASCII-only,
/// leaving the shared `split_name_tokens` byte-offset arithmetic untested
/// on this attribute.
#[test]
fn sb02_depends_on_korean_multibyte_tokens_slice_back_to_the_source() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\" ",
        "depends-on=\"가나다, cacheManager, 라마바\"/></beans>"
    );
    let bean = only_bean(source);
    assert_eq!(
        bean.depends_on
            .iter()
            .map(|d| d.value.raw.as_str())
            .collect::<Vec<_>>(),
        vec!["가나다", "cacheManager", "라마바"]
    );
    assert!(bean
        .depends_on
        .iter()
        .all(|d| d.value.kind == RefKind::Bean));
    for dep in &bean.depends_on {
        assert_eq!(
            &source[dep.span.start as usize..dep.span.end as usize],
            dep.value.raw.as_str()
        );
    }
}

#[test]
fn sb02_depends_on_absent_is_an_empty_vec() {
    let bean = only_bean("<beans><bean id=\"a\" class=\"com.example.Widget\"/></beans>");
    assert!(bean.depends_on.is_empty());
}

// ---------------------------------------------------------------------
// parent= empty value: RefWithoutTarget instead of an empty BeanRef.
// ---------------------------------------------------------------------

#[test]
fn sb02_empty_parent_attribute_is_ref_without_target_not_an_empty_bean_ref() {
    let source = "<beans><bean id=\"a\" parent=\"\"/></beans>";
    let r = beans_xml::parse(source);
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::RefWithoutTarget),
        "expected RefWithoutTarget for empty parent=: {:?}",
        r.diagnostics
    );
    let bean = only_bean(source);
    assert_eq!(bean.parent, None);
}

// ---------------------------------------------------------------------
// UnknownElement for an unrecognized element inside the beans namespace,
// directly under a <bean>.
// ---------------------------------------------------------------------

#[test]
fn sb02_unrecognized_beans_ns_child_of_bean_is_unknown_element() {
    let source =
        "<beans><bean id=\"a\" class=\"com.example.Widget\"><totally-made-up/></bean></beans>";
    let r = beans_xml::parse(source);
    assert!(r.beans.is_some());
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::UnknownElement),
        "expected UnknownElement for an unrecognized beans-ns child of <bean>: {:?}",
        r.diagnostics
    );
}

#[test]
fn sb02_foreign_namespace_child_of_bean_is_not_unknown_element() {
    // A decorator from a foreign namespace (e.g. aop:scoped-proxy) must
    // fall to the decorator catch-all, not UnknownElement — P6/P7's
    // territory, this only pins that U4's own catch-all path doesn't
    // misfire on it.
    let source = concat!(
        "<beans xmlns:aop=\"http://www.springframework.org/schema/aop\">",
        "<bean id=\"a\" class=\"com.example.Widget\"><aop:scoped-proxy/></bean>",
        "</beans>"
    );
    let r = beans_xml::parse(source);
    assert!(r.beans.is_some());
    assert!(
        !r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::UnknownElement),
        "a foreign-namespace decorator must not be UnknownElement: {:?}",
        r.diagnostics
    );
}

// ---------------------------------------------------------------------
// Reserved (not-yet-wired) property/constructor-arg element names must not
// misfire as UnknownElement either, even though U6/U7 haven't landed yet.
// ---------------------------------------------------------------------

#[test]
fn sb02_reserved_property_and_constructor_arg_elements_do_not_misfire_as_unknown_element() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\">",
        "<property name=\"x\" value=\"1\"/>",
        "<constructor-arg value=\"2\"/>",
        "</bean></beans>"
    );
    let r = beans_xml::parse(source);
    assert!(r.beans.is_some());
    assert!(
        !r.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::UnknownElement),
        "reserved <property>/<constructor-arg> must not be UnknownElement yet: {:?}",
        r.diagnostics
    );
}

// ---------------------------------------------------------------------
// autowire-candidate / lazy-init distinct-from-absent bool handling
// (mirrors U3's default_lazy_init table design, applied to Bean's own
// per-bean flags).
// ---------------------------------------------------------------------

#[test]
fn sb02_lazy_init_false_is_distinct_from_absent() {
    let bean = only_bean(
        "<beans><bean id=\"a\" class=\"com.example.Widget\" lazy-init=\"false\"/></beans>",
    );
    assert_eq!(bean.lazy_init, Some(false));
}

#[test]
fn sb02_lazy_init_absent_is_none() {
    let bean = only_bean("<beans><bean id=\"a\" class=\"com.example.Widget\"/></beans>");
    assert_eq!(bean.lazy_init, None);
}

#[test]
fn sb02_autowire_candidate_false_is_distinct_from_absent() {
    let bean = only_bean(concat!(
        "<beans><bean id=\"a\" class=\"com.example.Widget\" ",
        "autowire-candidate=\"false\"/></beans>"
    ));
    assert_eq!(bean.autowire_candidate, Some(false));
}

#[test]
fn sb02_autowire_candidate_absent_is_none() {
    let bean = only_bean("<beans><bean id=\"a\" class=\"com.example.Widget\"/></beans>");
    assert_eq!(bean.autowire_candidate, None);
}

#[test]
fn sb02_abstract_and_primary_default_to_false_when_absent() {
    let bean = only_bean("<beans><bean id=\"a\" class=\"com.example.Widget\"/></beans>");
    assert!(!bean.abstract_);
    assert!(!bean.primary);
}

// ---------------------------------------------------------------------
// p:/c:-namespace attributes on <bean> don't break core parsing even
// though normalization itself is P2's job (the prefixed-attribute hook is
// a no-op today) — exercises the hook slot without asserting its (not yet
// implemented) output.
// ---------------------------------------------------------------------

#[test]
fn sb02_pc_namespace_attributes_do_not_break_core_attribute_parsing() {
    let source = concat!(
        "<beans xmlns:p=\"http://www.springframework.org/schema/p\">",
        "<bean id=\"a\" class=\"com.example.Widget\" p:name=\"hello\"/>",
        "</beans>"
    );
    let bean = only_bean(source);
    assert_eq!(bean.id.as_ref().map(|s| s.value.as_str()), Some("a"));
    assert_eq!(
        bean.class.as_ref().map(|c| c.value.raw.as_str()),
        Some("com.example.Widget")
    );
}
