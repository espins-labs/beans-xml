//! Unit **P6** — `<bean>` sub-elements: `<qualifier>` + nested
//! `<attribute>`, `<meta>`, `<description>`, and the decorator slice of
//! `Bean::decorators` (SB-02b, "P6 qualifier/description/meta/decorator
//! [U4, decorator←P7]").
//!
//! `bean::parse_qualifier`/`bean::parse_meta`/`dispatch_bean_child` are all
//! `pub(crate)` — a seam not visible from this external integration-test
//! binary, the same situation `tests/p7_namespaced.rs`'s own doc comment
//! documents. The real P6 test suite — qualifier+attribute snapshot, meta
//! snapshot, description snapshot, the scoped-proxy decorator preserved
//! alongside all three, plus edge cases (empty `type=`, missing
//! `key=`/`value=`, multiple entries) — lives in `src/bean.rs`'s own
//! `#[cfg(test)] mod tests`.
//!
//! This file exercises the one thing observable from *outside* the crate at
//! this stage: end-to-end, through the public `beans_xml::parse` API,
//! proving `dispatch_bean_child`'s `"qualifier"`/`"meta"` arms actually
//! reach `bean::parse_qualifier`/`bean::parse_meta` in production, not just
//! in that module's own in-crate unit tests.

fn only_bean(source: &str) -> beans_xml::Bean {
    let mut beans = beans_xml::parse(source).beans.expect("beans root").beans;
    assert_eq!(beans.len(), 1, "expected exactly one top-level <bean>");
    beans.remove(0)
}

#[test]
fn sb02b_qualifier_with_attribute_lands_in_bean_qualifiers_end_to_end() {
    let source = concat!(
        "<beans>",
        r#"<bean id="myBean" class="com.example.MyService">"#,
        r#"<qualifier type="com.example.Genuine" value="main">"#,
        r#"<attribute key="priority" value="high"/>"#,
        "</qualifier>",
        "</bean>",
        "</beans>"
    );
    let bean = only_bean(source);
    assert_eq!(bean.qualifiers.len(), 1);
    let qualifier = &bean.qualifiers[0];
    assert_eq!(
        qualifier.type_.as_ref().map(|t| t.value.raw.as_str()),
        Some("com.example.Genuine")
    );
    assert_eq!(
        qualifier.value.as_ref().map(|v| v.value.as_str()),
        Some("main")
    );
    assert_eq!(qualifier.attributes.len(), 1);
    assert_eq!(qualifier.attributes[0].key.value, "priority");
    assert_eq!(qualifier.attributes[0].value.value, "high");
    assert!(beans_xml::parse(source).diagnostics.is_empty());
}

#[test]
fn sb02b_meta_lands_in_bean_meta_end_to_end() {
    let source = concat!(
        "<beans>",
        r#"<bean id="myBean" class="com.example.MyService">"#,
        r#"<meta key="buildTool" value="maven"/>"#,
        "</bean>",
        "</beans>"
    );
    let bean = only_bean(source);
    assert_eq!(bean.meta.len(), 1);
    assert_eq!(bean.meta[0].key.value, "buildTool");
    assert_eq!(bean.meta[0].value.value, "maven");
    assert!(beans_xml::parse(source).diagnostics.is_empty());
}

#[test]
fn sb02b_description_qualifier_meta_and_scoped_proxy_decorator_all_coexist_end_to_end() {
    let source = concat!(
        "<beans>",
        r#"<bean id="myBean" class="com.example.MyService" "#,
        r#"xmlns:aop="http://www.springframework.org/schema/aop">"#,
        "<description>Handles example widgets.</description>",
        r#"<qualifier type="com.example.Genuine" value="main"/>"#,
        r#"<meta key="buildTool" value="maven"/>"#,
        r#"<aop:scoped-proxy proxy-target-class="true"/>"#,
        "</bean>",
        "</beans>"
    );
    let bean = only_bean(source);
    assert_eq!(
        bean.description.as_ref().map(|d| d.value.as_str()),
        Some("Handles example widgets.")
    );
    assert_eq!(bean.qualifiers.len(), 1);
    assert_eq!(bean.meta.len(), 1);
    assert_eq!(bean.decorators.len(), 1);
    assert_eq!(bean.decorators[0].local, "scoped-proxy");
    assert!(beans_xml::parse(source).diagnostics.is_empty());
}
