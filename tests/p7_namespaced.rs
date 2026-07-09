//! Unit **P7** — `NamespacedElement` + allowlisted ref harvest (SB-02c).
//!
//! `dispatch::parse_namespaced`/`bean::parse_decorator`/
//! `namespaced::build_namespaced_element` are all `pub(crate)` — a seam not
//! visible from this external integration-test binary, the same situation
//! `tests/u5a_inject_value.rs`/`tests/u5b_collection.rs` document. The real
//! P7 test suite — id-bearing snapshot, one case per `NS_REF_ALLOWLIST`
//! row, `pointcut-ref` exclusion, descendant recursion, the beans-ns `<ref>`
//! child exclusion, `attrs` preservation, and the `DEPTH_LIMIT` guard —
//! lives in `src/namespaced.rs`'s own `#[cfg(test)] mod tests`.
//!
//! This file exercises the one thing observable from *outside* the crate
//! at this stage: end-to-end, through the public `beans_xml::parse` API,
//! proving both call sites this unit wires
//! (`dispatch::parse_namespaced` → `BeansFile::namespaced`,
//! `bean::parse_decorator` → `Bean::decorators`) actually reach
//! `namespaced::build_namespaced_element` in production, not just in that
//! module's own in-crate unit tests.

fn parse_ok(source: &str) -> beans_xml::BeansFile {
    beans_xml::parse(source).beans.expect("beans root")
}

#[test]
fn sb02c_root_level_ns_element_lands_in_beans_file_namespaced() {
    let source = concat!(
        "<beans>",
        r#"<jee:jndi-lookup id="dataSource" jndi-name="java:comp/env/jdbc/DataSource" "#,
        r#"xmlns:jee="http://www.springframework.org/schema/jee"/>"#,
        "</beans>"
    );
    let beans = parse_ok(source);
    assert_eq!(beans.namespaced.len(), 1);
    let element = &beans.namespaced[0];
    assert_eq!(element.local, "jndi-lookup");
    assert_eq!(
        element.id.as_ref().map(|s| s.value.as_str()),
        Some("dataSource")
    );
    // Never fell through to UnknownElement or was silently dropped.
    assert!(beans_xml::parse(source).diagnostics.is_empty());
}

#[test]
fn sb02c_bean_child_decorator_lands_in_bean_decorators_with_allowlisted_ref() {
    let source = concat!(
        "<beans>",
        r#"<bean id="myBean" class="com.example.MyService">"#,
        r#"<aop:scoped-proxy proxy-target-class="true" "#,
        r#"xmlns:aop="http://www.springframework.org/schema/aop"/>"#,
        "</bean>",
        "</beans>"
    );
    let beans = parse_ok(source);
    assert_eq!(beans.beans.len(), 1);
    let decorators = &beans.beans[0].decorators;
    assert_eq!(decorators.len(), 1);
    assert_eq!(decorators[0].local, "scoped-proxy");
    assert_eq!(
        decorators[0].ns,
        "http://www.springframework.org/schema/aop"
    );
}

#[test]
fn sb02c_bean_child_decorator_with_allowlisted_ref_populates_decorators_refs() {
    // Unlike `sb02c_bean_child_decorator_lands_in_bean_decorators_with_allowlisted_ref`
    // above (whose `<aop:scoped-proxy>` carries no `NS_REF_ALLOWLIST` row),
    // this decorator element — `<aop:aspect ref=...>` — IS an allowlist
    // match, so this proves `bean::parse_decorator`'s call site actually
    // populates `Bean::decorators[].refs` in production, not just the
    // root-level `dispatch::parse_namespaced` path every other end-to-end
    // ref-harvest assertion in this file runs through.
    let source = concat!(
        "<beans>",
        r#"<bean id="myBean" class="com.example.MyService">"#,
        r#"<aop:aspect ref="loggingAspect" "#,
        r#"xmlns:aop="http://www.springframework.org/schema/aop"/>"#,
        "</bean>",
        "</beans>"
    );
    let beans = parse_ok(source);
    assert_eq!(beans.beans.len(), 1);
    let decorators = &beans.beans[0].decorators;
    assert_eq!(decorators.len(), 1);
    assert_eq!(decorators[0].local, "aspect");
    let raws: Vec<&str> = decorators[0]
        .refs
        .iter()
        .map(|r| r.value.raw.as_str())
        .collect();
    assert_eq!(raws, vec!["loggingAspect"]);
}

#[test]
fn sb02c_end_to_end_allowlist_ref_and_pointcut_ref_exclusion() {
    let source = concat!(
        "<beans>",
        r#"<aop:config xmlns:aop="http://www.springframework.org/schema/aop">"#,
        r#"<aop:advisor advice-ref="txAdvice" pointcut-ref="allBusinessMethods"/>"#,
        r#"</aop:config>"#,
        "</beans>"
    );
    let beans = parse_ok(source);
    assert_eq!(beans.namespaced.len(), 1);
    let config = &beans.namespaced[0];
    assert_eq!(config.local, "config");
    let raws: Vec<&str> = config.refs.iter().map(|r| r.value.raw.as_str()).collect();
    assert_eq!(raws, vec!["txAdvice"]);
}

#[test]
fn sb02c_util_list_is_id_bearing_and_does_not_harvest_its_beans_ns_ref_children() {
    let source = concat!(
        "<beans>",
        r#"<util:list id="itemList" xmlns:util="http://www.springframework.org/schema/util">"#,
        r#"<ref bean="itemOne"/>"#,
        r#"</util:list>"#,
        "</beans>"
    );
    let beans = parse_ok(source);
    assert_eq!(beans.namespaced.len(), 1);
    let list = &beans.namespaced[0];
    assert_eq!(list.local, "list");
    assert_eq!(list.id.as_ref().map(|s| s.value.as_str()), Some("itemList"));
    assert!(
        list.refs.is_empty(),
        "util:list content <ref> elements are v0.1 blind spot ⑷, must not be harvested"
    );
}
