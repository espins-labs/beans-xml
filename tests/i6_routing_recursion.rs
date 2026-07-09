//! Unit **I6** — routing + recursion-completeness integration test, per
//! the internal build plan's "dispatch contract" section and its own
//! I6 row: two back-edges no single unit's own test suite can see.
//!
//! 1. **Routing**: the merged root-/bean-child dispatch (`dispatch.rs`'s
//!    `dispatch_root_child`, `bean.rs`'s `dispatch_bean_child`) is a plain
//!    Rust `match` — mutually exclusive by construction, so "two handlers
//!    claim the same element" can't literally happen. What *can* happen is
//!    a **misrouting** bug: an element landing in the wrong bucket (e.g. a
//!    1st-class element falling through to the `NamespacedElement`
//!    catch-all because its match arm's namespace/local-name test is
//!    subtly wrong) or **silently vanishing** (matched by an arm that
//!    doesn't push it anywhere). This file builds one document exercising
//!    every root-/bean-child dispatch arm **simultaneously** and asserts
//!    the *exact* count in every bucket at once — a bug that mis-routes
//!    one element necessarily shows up as a wrong count somewhere in this
//!    shared assertion set, which no single leaf unit's own narrower test
//!    (one element shape at a time) can catch. Specifically pins the build
//!    plan's own routing clause: `util:properties`/`context:component-scan`/
//!    `context:property-placeholder` must land through their 1st-class
//!    handler stubs (`parse_property_source`/`parse_component_scan`), never
//!    the `NamespacedElement` fallback — true regardless of whether P4/P5
//!    (M1, still stubs) have filled in their own bucket fields yet, since
//!    what this guards is *routing*, not those units' own field population.
//! 2. **Recursion completeness** (the K-2 denominator, spec's edge-set
//!    definition): (a) an inner `<bean>` reached through a `<property>`
//!    carries its *own* `<property ref>` **and** its own collection-nested
//!    `<ref>` (the exact `quartz <list><ref>` shape the spec's K-2 row
//!    names), proving `InjectValue::Inner`'s recursion into the shared
//!    `parse_bean` isn't structure-only; (b) a nested `<beans profile>`
//!    fully populates its own `<bean>` (with `<property ref>`) *and*
//!    `<import>` by re-entering the shared `parse_beans_body`, proving
//!    P10's re-entry isn't structure-only either.
//!
//! Every symbol here is public (`beans_xml::parse`, `model` re-exports) —
//! pure end-to-end, same convention every other `tests/i*.rs`/`tests/p*.rs`
//! file follows.

use beans_xml::{Collection, DiagCode, InjectValue, RefKind};

fn parse_ok(source: &str) -> beans_xml::BeansFile {
    beans_xml::parse(source).beans.expect("beans root")
}

// ---------------------------------------------------------------------
// 1. Routing: one document, every dispatch arm at once.
// ---------------------------------------------------------------------

#[test]
fn i6_every_element_lands_in_exactly_one_bucket_context_and_util_properties_bypass_namespaced_fallback(
) {
    let source = concat!(
        "<beans xmlns:context=\"http://www.springframework.org/schema/context\" ",
        "xmlns:util=\"http://www.springframework.org/schema/util\" ",
        "xmlns:jee=\"http://www.springframework.org/schema/jee\" ",
        "xmlns:aop=\"http://www.springframework.org/schema/aop\">",
        "<description>root description</description>",
        r#"<import resource="classpath:other.xml"/>"#,
        r#"<alias name="realBean" alias="aliasName"/>"#,
        r#"<context:component-scan base-package="com.example.scan"/>"#,
        r#"<context:property-placeholder location="classpath:app.properties"/>"#,
        r#"<util:properties id="appProps" location="classpath:app2.properties"/>"#,
        r#"<jee:jndi-lookup id="jndiDataSource" jndi-name="java:comp/env/jdbc/DS"/>"#,
        r#"<bean id="realBean" class="com.example.RealBean">"#,
        "<description>bean description</description>",
        r#"<property name="collaborator" ref="collabBean"/>"#,
        r#"<constructor-arg index="0" value="42"/>"#,
        r#"<qualifier value="main"/>"#,
        r#"<meta key="k" value="v"/>"#,
        r#"<lookup-method name="createX" bean="collabBean"/>"#,
        r#"<replaced-method name="doIt" replacer="collabBean"/>"#,
        r#"<aop:scoped-proxy proxy-target-class="true"/>"#,
        "</bean>",
        r#"<bean id="collabBean" class="com.example.CollabBean"/>"#,
        r#"<beans profile="dev"><bean id="devBean" class="com.example.DevBean"/></beans>"#,
        "</beans>",
    );

    let result = beans_xml::parse(source);
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|d| d.code == DiagCode::UnknownElement),
        "every element in this document is recognized — no UnknownElement expected: {:?}",
        result.diagnostics
    );

    let beans = result.beans.expect("beans root");

    // Root-level buckets, each claimed by exactly its own handler.
    assert_eq!(
        beans.description.as_ref().map(|d| d.value.as_str()),
        Some("root description")
    );
    assert_eq!(beans.imports.len(), 1);
    assert_eq!(beans.aliases.len(), 1);
    assert_eq!(
        beans.beans.len(),
        2,
        "realBean + collabBean, top-level only"
    );
    assert_eq!(beans.nested_profiles.len(), 1);

    // The routing assertion this unit exists for: `context:component-scan`/
    // `context:property-placeholder`/`util:properties` must NOT fall
    // through to the `NamespacedElement` catch-all — only the genuinely
    // out-of-1st-class-scope `jee:jndi-lookup` should be there.
    assert_eq!(
        beans.namespaced.len(),
        1,
        "context:*/util:properties must route to their 1st-class handler, not NamespacedElement: {:?}",
        beans.namespaced
    );
    assert_eq!(beans.namespaced[0].local, "jndi-lookup");
    assert_eq!(
        beans.namespaced[0].ns,
        "http://www.springframework.org/schema/jee"
    );

    // Bean-level buckets on the one bean exercising all of them.
    let real_bean = beans
        .beans
        .iter()
        .find(|b| b.id.as_ref().map(|s| s.value.as_str()) == Some("realBean"))
        .expect("realBean present");
    assert_eq!(
        real_bean.description.as_ref().map(|d| d.value.as_str()),
        Some("bean description")
    );
    assert_eq!(real_bean.properties.len(), 1);
    assert_eq!(real_bean.constructor_args.len(), 1);
    assert_eq!(real_bean.qualifiers.len(), 1);
    assert_eq!(real_bean.meta.len(), 1);
    assert_eq!(real_bean.lookup_methods.len(), 1);
    assert_eq!(real_bean.replaced_methods.len(), 1);
    assert_eq!(
        real_bean.decorators.len(),
        1,
        "aop:scoped-proxy must land in Bean::decorators, not anywhere else"
    );
    assert_eq!(real_bean.decorators[0].local, "scoped-proxy");

    // Nested profile: its own bean is not double-counted into the parent's
    // `beans.beans`.
    let nested = &beans.nested_profiles[0];
    assert_eq!(
        nested.profile.as_ref().map(|p| p.value.as_str()),
        Some("dev")
    );
    assert_eq!(nested.beans.len(), 1);
    assert_eq!(
        nested.beans[0].id.as_ref().map(|s| s.value.as_str()),
        Some("devBean")
    );
    assert!(
        !beans
            .beans
            .iter()
            .any(|b| b.id.as_ref().map(|s| s.value.as_str()) == Some("devBean")),
        "a nested profile's own bean must not also appear in the parent's top-level beans"
    );
}

#[test]
fn i6_unrecognized_beans_ns_root_child_is_unknown_element_not_silently_dropped_or_namespaced() {
    // A typo/future element still inside the `beans` namespace itself must
    // be diagnosed as `UnknownElement` — never silently dropped, and never
    // mistaken for the `NamespacedElement` catch-all (which is reserved for
    // *other* namespaces, per `dispatch_root_child`'s own doc comment on
    // arm ordering).
    let source = "<beans><no-such-beans-element/></beans>";
    let result = beans_xml::parse(source);
    let beans = result.beans.expect("beans root");
    assert!(beans.namespaced.is_empty());
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == DiagCode::UnknownElement));
}

#[test]
fn i6_unrecognized_beans_ns_bean_child_is_unknown_element_not_silently_dropped_or_decorator() {
    // Same shape, one level down: a typo'd `<bean>` child in the beans
    // namespace must not be mistaken for a decorator either.
    let source =
        r#"<beans><bean id="a" class="com.example.A"><no-such-bean-child/></bean></beans>"#;
    let result = beans_xml::parse(source);
    let beans = result.beans.expect("beans root");
    assert!(beans.beans[0].decorators.is_empty());
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == DiagCode::UnknownElement));
}

// ---------------------------------------------------------------------
// 2a. Recursion completeness — inner <bean> with its own <property ref>
// and collection-nested <ref> (the quartz "<list><ref>" K-2 shape).
// ---------------------------------------------------------------------

#[test]
fn i6_inner_bean_reached_through_property_carries_its_own_property_ref_and_collection_ref() {
    let source = concat!(
        "<beans>",
        r#"<bean id="outer" class="com.example.Outer">"#,
        r#"<property name="inner">"#,
        r#"<bean id="innerBean" class="com.example.Inner">"#,
        r#"<property name="collaborator" ref="collabBean"/>"#,
        r#"<property name="triggers"><list><ref bean="listRefBean"/></list></property>"#,
        "</bean>",
        "</property>",
        "</bean>",
        r#"<bean id="collabBean" class="com.example.Collab"/>"#,
        r#"<bean id="listRefBean" class="com.example.ListRef"/>"#,
        "</beans>",
    );
    let beans = parse_ok(source);
    let outer = beans
        .beans
        .iter()
        .find(|b| b.id.as_ref().map(|s| s.value.as_str()) == Some("outer"))
        .expect("outer bean present");
    assert_eq!(outer.properties.len(), 1);

    let InjectValue::Inner(inner_bean) = &outer.properties[0].value else {
        panic!(
            "expected InjectValue::Inner, got {:?}",
            outer.properties[0].value
        );
    };
    assert_eq!(
        inner_bean.id.as_ref().map(|s| s.value.as_str()),
        Some("innerBean")
    );
    // The K-2 denominator's whole point: the inner bean's own properties
    // (not just its id/class) must be present — a structure-only recursion
    // that stopped at `InjectValue::Inner(Box::new(Bean::default()))`-shaped
    // content would still satisfy a shallower "did it recurse at all" check
    // but silently drop every edge inside.
    assert_eq!(inner_bean.properties.len(), 2);

    match &inner_bean.properties[0].value {
        InjectValue::Ref(bean_ref) => {
            assert_eq!(bean_ref.value.raw, "collabBean");
            assert_eq!(bean_ref.value.kind, RefKind::Bean);
        }
        other => panic!("expected InjectValue::Ref, got {other:?}"),
    }

    match &inner_bean.properties[1].value {
        InjectValue::Collection(collection) => match &collection.value {
            Collection::List { items, .. } => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    InjectValue::Ref(bean_ref) => {
                        assert_eq!(bean_ref.value.raw, "listRefBean");
                    }
                    other => panic!("expected InjectValue::Ref inside the list, got {other:?}"),
                }
            }
            other => panic!("expected Collection::List, got {other:?}"),
        },
        other => panic!("expected InjectValue::Collection, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// 2b. Recursion completeness — nested <beans profile> fills its own
// <bean> (with <property ref>) and <import> via the shared re-entrant
// parse_beans_body, not a structure-only stub.
// ---------------------------------------------------------------------

#[test]
fn i6_nested_beans_profile_fills_its_own_bean_property_ref_and_import() {
    let source = concat!(
        "<beans>",
        r#"<beans profile="qa">"#,
        r#"<import resource="classpath:qa-extra.xml"/>"#,
        r#"<bean id="qaBean" class="com.example.QaBean">"#,
        r#"<property name="dependency" ref="depBean"/>"#,
        "</bean>",
        "</beans>",
        "</beans>",
    );
    let beans = parse_ok(source);
    assert_eq!(beans.nested_profiles.len(), 1);
    let nested = &beans.nested_profiles[0];
    assert_eq!(
        nested.profile.as_ref().map(|p| p.value.as_str()),
        Some("qa")
    );

    assert_eq!(nested.imports.len(), 1);
    assert_eq!(
        nested.imports[0].value.resource.value,
        "classpath:qa-extra.xml"
    );

    assert_eq!(nested.beans.len(), 1);
    let qa_bean = &nested.beans[0];
    assert_eq!(
        qa_bean.id.as_ref().map(|s| s.value.as_str()),
        Some("qaBean")
    );
    assert_eq!(qa_bean.properties.len(), 1);
    match &qa_bean.properties[0].value {
        InjectValue::Ref(bean_ref) => assert_eq!(bean_ref.value.raw, "depBean"),
        other => panic!("expected InjectValue::Ref, got {other:?}"),
    }

    // Not accidentally hoisted to the parent's own top-level beans/imports.
    assert!(beans.beans.is_empty());
    assert!(beans.imports.is_empty());
}
