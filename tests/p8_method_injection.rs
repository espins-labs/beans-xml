//! Unit **P8** — method injection: `<lookup-method name= bean=>` and
//! `<replaced-method name= replacer=>` + nested `<arg-type match=>`
//! (SB-06b, `Bean::lookup_methods`/`Bean::replaced_methods`).
//!
//! `bean::parse_lookup_method`/`bean::parse_replaced_method`/
//! `dispatch_bean_child` are all `pub(crate)` — a seam not visible from
//! this external integration-test binary, the same situation
//! `tests/p6_qualifier_meta_decorator.rs`'s own doc comment documents. The
//! real P8 test table — name present/absent, `bean=`/`replacer=` present/
//! empty/missing (all three routes to `RefWithoutTarget`, element always
//! preserved, no edge emitted), multiple `<arg-type match=>` children, a
//! degenerate `<arg-type>` missing `match=`, and `ReplacedMethod::name`'s
//! required-by-model empty-string fallback — lives in `src/bean.rs`'s own
//! `#[cfg(test)] mod tests`.
//!
//! This file exercises the one thing observable from *outside* the crate at
//! this stage: end-to-end, through the public `beans_xml::parse` API,
//! proving `dispatch_bean_child`'s `"lookup-method"`/`"replaced-method"`
//! arms actually reach `bean::parse_lookup_method`/
//! `bean::parse_replaced_method` in production, not just in that module's
//! own in-crate unit tests.

use beans_xml::DiagCode;

fn only_bean(source: &str) -> beans_xml::Bean {
    let mut beans = beans_xml::parse(source).beans.expect("beans root").beans;
    assert_eq!(beans.len(), 1, "expected exactly one top-level <bean>");
    beans.remove(0)
}

#[test]
fn sb06b_lookup_method_with_name_and_bean_lands_in_bean_lookup_methods_end_to_end() {
    let source = concat!(
        "<beans>",
        r#"<bean id="myBean" class="com.example.MyService">"#,
        r#"<lookup-method name="createCommand" bean="commandBean"/>"#,
        "</bean>",
        "</beans>"
    );
    let bean = only_bean(source);
    assert_eq!(bean.lookup_methods.len(), 1);
    let lookup_method = &bean.lookup_methods[0].value;
    assert_eq!(
        lookup_method.name.as_ref().map(|n| n.value.as_str()),
        Some("createCommand")
    );
    assert_eq!(
        lookup_method.bean.as_ref().map(|b| b.value.raw.as_str()),
        Some("commandBean")
    );
    assert!(beans_xml::parse(source).diagnostics.is_empty());
}

#[test]
fn sb06b_lookup_method_missing_bean_is_ref_without_target_end_to_end() {
    let source = concat!(
        "<beans>",
        r#"<bean id="myBean" class="com.example.MyService">"#,
        r#"<lookup-method name="createCommand"/>"#,
        "</bean>",
        "</beans>"
    );
    let bean = only_bean(source);
    assert_eq!(
        bean.lookup_methods.len(),
        1,
        "element must still be preserved even with no usable bean= target"
    );
    assert_eq!(bean.lookup_methods[0].value.bean, None);
    let diagnostics = beans_xml::parse(source).diagnostics;
    assert!(diagnostics
        .iter()
        .any(|d| d.code == DiagCode::RefWithoutTarget));
}

#[test]
fn sb06b_replaced_method_with_replacer_and_arg_types_lands_in_bean_replaced_methods_end_to_end() {
    let source = concat!(
        "<beans>",
        r#"<bean id="myBean" class="com.example.MyService">"#,
        r#"<replaced-method name="computeValue" replacer="replacerBean">"#,
        r#"<arg-type match="java.lang.String"/>"#,
        r#"<arg-type match="int"/>"#,
        "</replaced-method>",
        "</bean>",
        "</beans>"
    );
    let bean = only_bean(source);
    assert_eq!(bean.replaced_methods.len(), 1);
    let replaced_method = &bean.replaced_methods[0].value;
    assert_eq!(replaced_method.name.value, "computeValue");
    assert_eq!(
        replaced_method
            .replacer
            .as_ref()
            .map(|r| r.value.raw.as_str()),
        Some("replacerBean")
    );
    assert_eq!(
        replaced_method
            .arg_types
            .iter()
            .map(|a| a.value.as_str())
            .collect::<Vec<_>>(),
        vec!["java.lang.String", "int"]
    );
    assert!(beans_xml::parse(source).diagnostics.is_empty());
}

#[test]
fn sb06b_replaced_method_missing_replacer_is_ref_without_target_end_to_end() {
    let source = concat!(
        "<beans>",
        r#"<bean id="myBean" class="com.example.MyService">"#,
        r#"<replaced-method name="computeValue"/>"#,
        "</bean>",
        "</beans>"
    );
    let bean = only_bean(source);
    assert_eq!(
        bean.replaced_methods.len(),
        1,
        "element must still be preserved even with no usable replacer= target"
    );
    assert_eq!(bean.replaced_methods[0].value.replacer, None);
    let diagnostics = beans_xml::parse(source).diagnostics;
    assert!(diagnostics
        .iter()
        .any(|d| d.code == DiagCode::RefWithoutTarget));
}

#[test]
fn sb06b_lookup_and_replaced_method_coexist_end_to_end() {
    let source = concat!(
        "<beans>",
        r#"<bean id="myBean" class="com.example.MyService">"#,
        r#"<lookup-method name="createCommand" bean="commandBean"/>"#,
        r#"<replaced-method name="computeValue" replacer="replacerBean"/>"#,
        "</bean>",
        "</beans>"
    );
    let bean = only_bean(source);
    assert_eq!(bean.lookup_methods.len(), 1);
    assert_eq!(bean.replaced_methods.len(), 1);
    assert!(beans_xml::parse(source).diagnostics.is_empty());
}
