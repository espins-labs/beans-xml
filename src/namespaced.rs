//! Unit **P7** — `NamespacedElement` + allowlisted ref harvest (SB-02c).
//!
//! Fills two of the leaf-wave's frozen stubs, per this unit's own build
//! plan row ("P7 NamespacedElement + recursive ref harvest ... [U3] ── P6 ...
//! [U4, decorator←P7]"):
//! - [`dispatch::parse_namespaced`](crate::dispatch::parse_namespaced) — the
//!   root-child catch-all, `BeansFileCtx::namespaced`.
//! - [`bean::parse_decorator`](crate::bean::parse_decorator) — the
//!   bean-child catch-all, `BeanCtx::decorators`.
//!
//! Both call into this module's single [`build_namespaced_element`] rather
//! than duplicating the "preserve ns/local/id/attrs, then recursively
//! harvest allowlisted ref attributes" logic twice — the same "one shared
//! builder, several call sites" shape `inject_value::parse_inject_value_child`
//! already establishes for U6/U7. Per the dispatch contract, this unit
//! touches only those two stub *bodies* — never `dispatch_root_child`'s or
//! `dispatch_bean_child`'s shared match.
//!
//! ## `NS_REF_ALLOWLIST`
//!
//! The build plan's own name for this table ("P7 and the K-2 harness
//! reference the same table (prevents a mismatched ref set)") — a fixed, exhaustive list of
//! (namespace, element local name, ref-bearing attribute) triples. No other
//! attribute, on any element, at any recursion depth, is ever collected
//! into `NamespacedElement::refs` — this is a narrow bean-to-bean edge
//! collection policy, not a general attribute walk (`NamespacedElement`'s
//! own doc comment):
//!
//! - **aop**: `<aspect ref=...>`, `<advisor advice-ref=...>`. Deliberately
//!   **not** `pointcut-ref` on either element — spec's "settled decisions": a
//!   pointcut is not itself a bean, so a `pointcut-ref` target is never a
//!   valid `ref=`-style bean-to-bean edge (harvesting it would manufacture
//!   a confirmed-dangling reference).
//! - **tx**: `<advice transaction-manager=...>`, `<annotation-driven
//!   transaction-manager=...>`.
//! - **task**: `<scheduled ref=...>`, `<scheduled scheduler=...>` — two
//!   separate rules on the same element/namespace, since a `<task:scheduled>`
//!   can carry both attributes at once and both are distinct bean edges.
//! - **jee**: `<jndi-lookup environment-ref=...>`, `<remote-slsb
//!   environment-ref=...>`, `<local-slsb environment-ref=...>` — the one
//!   bean-to-bean edge any of these carry (a `util:properties`/`util:map`
//!   bean supplying JNDI environment entries). Their own `jndi-name=` is an
//!   opaque external JNDI path, not a same-file bean reference, so it is
//!   never in this table.

use crate::dispatch::{find_attr, resolve_qname, spanned_attr, NsScope};
use crate::events::{XmlAttr, XmlElement, XmlNode};
use crate::inject_value::ref_from_attr;
use crate::model::{AttrPair, BeanRef, DiagCode, Diagnostic, NamespacedElement, Spanned};

/// Spring `aop` namespace URI.
const AOP_NS_URI: &str = "http://www.springframework.org/schema/aop";
/// Spring `tx` namespace URI.
const TX_NS_URI: &str = "http://www.springframework.org/schema/tx";
/// Spring `task` namespace URI.
const TASK_NS_URI: &str = "http://www.springframework.org/schema/task";
/// Spring `jee` namespace URI.
const JEE_NS_URI: &str = "http://www.springframework.org/schema/jee";

/// `true` for the declared `aop` URI or (no `xmlns:aop` in scope) the raw
/// `aop` prefix text itself — same "resolved URI, or raw prefix" fallback
/// `dispatch::is_context_ns`/`bean::is_p_ns` already apply for their own
/// namespaces, reused here so a hand-written fixture that skips the (very
/// common, but not XSD-mandatory) `xmlns:aop` declaration still matches.
fn is_aop_ns(ns: &str) -> bool {
    ns == AOP_NS_URI || ns == "aop"
}

/// See [`is_aop_ns`] — same fallback, `tx` namespace.
fn is_tx_ns(ns: &str) -> bool {
    ns == TX_NS_URI || ns == "tx"
}

/// See [`is_aop_ns`] — same fallback, `task` namespace.
fn is_task_ns(ns: &str) -> bool {
    ns == TASK_NS_URI || ns == "task"
}

/// See [`is_aop_ns`] — same fallback, `jee` namespace.
fn is_jee_ns(ns: &str) -> bool {
    ns == JEE_NS_URI || ns == "jee"
}

/// One `NS_REF_ALLOWLIST` row: an element (identified by namespace-matcher
/// function + local name) and the one attribute on it that names another
/// bean.
struct NsRefRule {
    ns_matches: fn(&str) -> bool,
    local: &'static str,
    attr: &'static str,
}

/// The frozen allowlist table itself — see this module's own doc comment
/// for the full per-namespace rationale.
const NS_REF_ALLOWLIST: &[NsRefRule] = &[
    NsRefRule {
        ns_matches: is_aop_ns,
        local: "aspect",
        attr: "ref",
    },
    NsRefRule {
        ns_matches: is_aop_ns,
        local: "advisor",
        attr: "advice-ref",
    },
    NsRefRule {
        ns_matches: is_tx_ns,
        local: "advice",
        attr: "transaction-manager",
    },
    NsRefRule {
        ns_matches: is_tx_ns,
        local: "annotation-driven",
        attr: "transaction-manager",
    },
    NsRefRule {
        ns_matches: is_task_ns,
        local: "scheduled",
        attr: "ref",
    },
    NsRefRule {
        ns_matches: is_task_ns,
        local: "scheduled",
        attr: "scheduler",
    },
    NsRefRule {
        ns_matches: is_jee_ns,
        local: "jndi-lookup",
        attr: "environment-ref",
    },
    NsRefRule {
        ns_matches: is_jee_ns,
        local: "remote-slsb",
        attr: "environment-ref",
    },
    NsRefRule {
        ns_matches: is_jee_ns,
        local: "local-slsb",
        attr: "environment-ref",
    },
];

// ---------------------------------------------------------------------
// The shared builder.
// ---------------------------------------------------------------------

/// Builds a [`NamespacedElement`] from `element` — either a `<beans>`-body
/// child not claimed by any first-class handler
/// ([`crate::dispatch::parse_namespaced`]'s call site), or a `<bean>`-body
/// child in some non-`beans` namespace ([`crate::bean::parse_decorator`]'s).
///
/// `scope` is the PARENT scope — the one in effect for `element`'s parent,
/// *before* overlaying whatever `xmlns`/`xmlns:*` declarations `element`
/// itself carries — same convention every other handler stub in this crate
/// documents; this function overlays its own once, for `ns`/`local`
/// resolution, `id`/`attrs` reading, and the recursive ref harvest below.
///
/// `id` is read generically — whatever `id="..."` attribute `element`
/// itself carries, regardless of its specific namespace/local name (not
/// gated to a fixed set like `jee:jndi-lookup`/`util:list`): those are only
/// the *reason* an id matters (an NS element with an id registers a bean
/// id the same way `<bean id=...>` does), not an exhaustive allowlist of
/// which elements are permitted to carry one.
///
/// `attrs` preserves every attribute `element` itself carries, in document
/// order, **except** `xmlns`/`xmlns:*` declarations — those are namespace
/// bookkeeping (already folded into `scope`/the overlay above), not a
/// semantic attribute a consumer would want raw-preserved here, same
/// exclusion `bean::normalize_pc_attr` applies before reading a `p:`/`c:`
/// attribute.
///
/// `refs` is the allowlisted-ref harvest — see [`harvest_refs`].
pub(crate) fn build_namespaced_element(
    scope: &NsScope,
    element: &XmlElement,
    diagnostics: &mut Vec<Diagnostic>,
) -> NamespacedElement {
    let own_scope = NsScope::from_element(element, Some(scope));
    let (ns, local) = resolve_qname(&element.name, &own_scope);

    let id = find_attr(&element.attrs, "id").map(spanned_attr);
    let attrs = element
        .attrs
        .iter()
        .filter(|attr| !is_xmlns_decl(attr))
        .map(attr_pair)
        .collect();

    let mut refs = Vec::new();
    harvest_refs(scope, element, &mut refs, diagnostics, 0);

    NamespacedElement {
        ns,
        local,
        span: element.span,
        id,
        attrs,
        refs,
    }
}

/// `true` for `xmlns` or any `xmlns:*` attribute — see
/// [`build_namespaced_element`]'s own doc comment for why these are
/// excluded from `attrs`.
fn is_xmlns_decl(attr: &XmlAttr) -> bool {
    attr.name == "xmlns" || attr.name.starts_with("xmlns:")
}

/// `AttrPair` from one raw `XmlAttr`, preserving both the attribute name's
/// own span and its value's span (no single combined "whole attribute"
/// span here — unlike `bean::normalize_pc_attr`'s p:/c: entries, an
/// `AttrPair` has no wrapping `Property`/`ConstructorArg`-shaped consumer
/// that needs one).
fn attr_pair(attr: &XmlAttr) -> AttrPair {
    AttrPair {
        key: Spanned {
            value: attr.name.clone(),
            span: attr.name_span,
        },
        value: spanned_attr(attr),
    }
}

/// Recursively walks `element` and every descendant element — regardless of
/// namespace, including plain `beans`-namespace descendants (a `<ref>`
/// nested inside a preserved `util:list`, say) — collecting a
/// `Spanned<BeanRef>` for each `(namespace, local, attribute)` combination
/// matching a [`NS_REF_ALLOWLIST`] row. A `beans`-namespace descendant like
/// `<ref bean=...>` never actually matches any row (no row's namespace
/// matcher accepts the beans namespace, and `"ref"`/`"bean"` aren't any
/// row's `local`/`attr`), so this walks through it harmlessly rather than
/// needing a special-cased skip — this is also what keeps spec's known
/// blind spot ⑷ ("a `<ref>` element inside util contents ... not collected
/// in v0.1") true by construction, not by an extra check.
///
/// `scope` is `element`'s own PARENT scope, mirroring
/// [`build_namespaced_element`]'s convention — this function computes its
/// own overlay before reading `element`'s attributes or recursing.
///
/// `depth` bounds this recursion the same way U5a's inner-bean recursion
/// does (`inject_value::parse_inner_bean`): an adversarial input with
/// thousands of nested elements under one `NamespacedElement` must not
/// blow the call stack (invariant #1). The two guards trip at a
/// deliberately *different* boundary, though, not an identical one: this
/// function's own allowlisted-ref check always runs first, *then* the
/// `depth >= DEPTH_LIMIT` test decides whether to recurse into children —
/// so an element at exactly `depth == DEPTH_LIMIT` still gets its own ref
/// harvested (deepest-so-far element processed, not "one past" it) before
/// the guard stops further descent, meaning this function processes real
/// elements at `depth` `0..=DEPTH_LIMIT` inclusive. `parse_inner_bean`
/// checks its guard *before* doing anything else at all, so it only ever
/// processes `depth` `0..=DEPTH_LIMIT - 1`. This is a deliberate choice,
/// not an oversight: harvesting one further level's own ref is cheap and
/// still bounded (it never recurses past it), unlike calling `parse_bean`
/// one level too deep would be.
fn harvest_refs(
    scope: &NsScope,
    element: &XmlElement,
    out: &mut Vec<Spanned<BeanRef>>,
    diagnostics: &mut Vec<Diagnostic>,
    depth: u32,
) {
    let own_scope = NsScope::from_element(element, Some(scope));
    let (ns, local) = resolve_qname(&element.name, &own_scope);

    for rule in NS_REF_ALLOWLIST {
        if (rule.ns_matches)(&ns) && rule.local == local {
            if let Some(attr) = find_attr(&element.attrs, rule.attr) {
                if let Some(bean_ref) = ref_from_attr(attr, diagnostics) {
                    out.push(bean_ref);
                }
            }
        }
    }

    if depth >= crate::DEPTH_LIMIT {
        diagnostics.push(Diagnostic {
            code: DiagCode::NestingLimitExceeded,
            span: Some(element.span),
            message: format!(
                "namespaced element nesting exceeded {} levels while harvesting refs; \
                 remaining subtree treated as opaque",
                crate::DEPTH_LIMIT
            ),
        });
        return;
    }

    for child in &element.children {
        if let XmlNode::Element(child_element) = child {
            harvest_refs(&own_scope, child_element, out, diagnostics, depth + 1);
        }
    }
}

// ---------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------
//
// In-module (rather than only `tests/p7_namespaced.rs`) because
// `build_namespaced_element`/`harvest_refs` are `pub(crate)` — a seam not
// visible from an external integration-test binary, the same situation
// `inject_value.rs`'s/`collection.rs`'s own `#[cfg(test)] mod tests` doc
// comments document. `tests/p7_namespaced.rs` carries a pointer-plus-smoke
// test through the public `beans_xml::parse` API, proving both call sites
// (`dispatch::parse_namespaced`, `bean::parse_decorator`) actually reach
// this module's builder in production, not just in these unit tests.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::build_tree;

    fn parse_fragment(source: &str) -> XmlElement {
        build_tree(source).root.expect("root element found")
    }

    fn no_diag() -> Vec<Diagnostic> {
        Vec::new()
    }

    // -------------------------------------------------------------
    // jee:jndi-lookup — id-bearing NS element (registers a bean id).
    // -------------------------------------------------------------

    #[test]
    fn sb02c_jee_jndi_lookup_is_id_bearing_snapshot() {
        let element = parse_fragment(concat!(
            r#"<jee:jndi-lookup id="dataSource" "#,
            r#"jndi-name="java:comp/env/jdbc/DataSource" "#,
            r#"xmlns:jee="http://www.springframework.org/schema/jee"/>"#,
        ));
        let mut diagnostics = no_diag();
        let result = build_namespaced_element(&NsScope::default(), &element, &mut diagnostics);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        assert_eq!(
            result.id.as_ref().map(|s| s.value.as_str()),
            Some("dataSource")
        );
        assert_eq!(result.local, "jndi-lookup");
        assert!(result.refs.is_empty(), "no environment-ref present here");
        insta::assert_json_snapshot!(result);
    }

    // -------------------------------------------------------------
    // Allowlist harvest — one case per NS_REF_ALLOWLIST row.
    // -------------------------------------------------------------

    #[test]
    fn sb02c_aop_aspect_ref_is_harvested() {
        let element = parse_fragment(concat!(
            r#"<aop:aspect ref="loggingAspect" "#,
            r#"xmlns:aop="http://www.springframework.org/schema/aop"/>"#,
        ));
        let mut diagnostics = no_diag();
        let result = build_namespaced_element(&NsScope::default(), &element, &mut diagnostics);
        assert_eq!(result.refs.len(), 1);
        assert_eq!(result.refs[0].value.raw, "loggingAspect");
    }

    #[test]
    fn sb02c_aop_advisor_advice_ref_is_harvested_but_pointcut_ref_is_not() {
        let element = parse_fragment(concat!(
            r#"<aop:advisor advice-ref="txAdvice" pointcut-ref="allBusinessMethods" "#,
            r#"xmlns:aop="http://www.springframework.org/schema/aop"/>"#,
        ));
        let mut diagnostics = no_diag();
        let result = build_namespaced_element(&NsScope::default(), &element, &mut diagnostics);
        assert_eq!(result.refs.len(), 1, "only advice-ref, never pointcut-ref");
        assert_eq!(result.refs[0].value.raw, "txAdvice");
        assert!(
            !result
                .refs
                .iter()
                .any(|r| r.value.raw == "allBusinessMethods"),
            "pointcut-ref must never be harvested — a pointcut is not a bean"
        );
    }

    #[test]
    fn sb02c_tx_advice_transaction_manager_is_harvested() {
        let element = parse_fragment(concat!(
            r#"<tx:advice transaction-manager="txManager" "#,
            r#"xmlns:tx="http://www.springframework.org/schema/tx"/>"#,
        ));
        let mut diagnostics = no_diag();
        let result = build_namespaced_element(&NsScope::default(), &element, &mut diagnostics);
        assert_eq!(result.refs.len(), 1);
        assert_eq!(result.refs[0].value.raw, "txManager");
    }

    #[test]
    fn sb02c_tx_annotation_driven_transaction_manager_is_harvested() {
        let element = parse_fragment(concat!(
            r#"<tx:annotation-driven transaction-manager="txManager" "#,
            r#"xmlns:tx="http://www.springframework.org/schema/tx"/>"#,
        ));
        let mut diagnostics = no_diag();
        let result = build_namespaced_element(&NsScope::default(), &element, &mut diagnostics);
        assert_eq!(result.refs.len(), 1);
        assert_eq!(result.refs[0].value.raw, "txManager");
    }

    #[test]
    fn sb02c_task_scheduled_ref_and_scheduler_are_both_harvested() {
        let element = parse_fragment(concat!(
            r#"<task:scheduled ref="myTask" scheduler="myScheduler" method="run" "#,
            r#"cron="0 0 * * * *" "#,
            r#"xmlns:task="http://www.springframework.org/schema/task"/>"#,
        ));
        let mut diagnostics = no_diag();
        let result = build_namespaced_element(&NsScope::default(), &element, &mut diagnostics);
        let raws: Vec<&str> = result.refs.iter().map(|r| r.value.raw.as_str()).collect();
        assert_eq!(raws.len(), 2);
        assert!(raws.contains(&"myTask"));
        assert!(raws.contains(&"myScheduler"));
    }

    #[test]
    fn sb02c_jee_jndi_lookup_environment_ref_is_harvested() {
        let element = parse_fragment(concat!(
            r#"<jee:jndi-lookup id="dataSource" jndi-name="java:comp/env/jdbc/DataSource" "#,
            r#"environment-ref="jndiEnv" "#,
            r#"xmlns:jee="http://www.springframework.org/schema/jee"/>"#,
        ));
        let mut diagnostics = no_diag();
        let result = build_namespaced_element(&NsScope::default(), &element, &mut diagnostics);
        assert_eq!(result.refs.len(), 1);
        assert_eq!(result.refs[0].value.raw, "jndiEnv");
    }

    // -------------------------------------------------------------
    // Descendant recursion + beans-ns child <ref> exclusion.
    // -------------------------------------------------------------

    #[test]
    fn sb02c_descendant_ns_elements_are_recursed_into_snapshot() {
        // The top-level catch-all element is `aop:config` — not itself in
        // NS_REF_ALLOWLIST — wrapping an `aop:aspect` descendant that is.
        let element = parse_fragment(concat!(
            r#"<aop:config xmlns:aop="http://www.springframework.org/schema/aop">"#,
            r#"<aop:aspect ref="loggingAspect">"#,
            r#"<aop:pointcut id="allMethods" expression="execution(* com.example..*(..))"/>"#,
            r#"</aop:aspect>"#,
            r#"</aop:config>"#,
        ));
        let mut diagnostics = no_diag();
        let result = build_namespaced_element(&NsScope::default(), &element, &mut diagnostics);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        assert_eq!(result.local, "config");
        assert_eq!(result.refs.len(), 1, "harvested from the descendant aspect");
        assert_eq!(result.refs[0].value.raw, "loggingAspect");
        insta::assert_json_snapshot!(result);
    }

    #[test]
    fn sb02c_beans_ns_ref_child_is_not_harvested() {
        // A `<ref>` (plain beans namespace) nested inside a preserved
        // `util:list` — spec's own named blind spot ⑷ ("a `<ref>` element
        // inside util contents ... not collected in v0.1"). No NS_REF_ALLOWLIST row's namespace matcher
        // accepts the beans namespace, so this must fall straight through.
        let element = parse_fragment(concat!(
            r#"<util:list xmlns:util="http://www.springframework.org/schema/util">"#,
            r#"<ref bean="itemOne"/>"#,
            r#"<value>literalItem</value>"#,
            r#"</util:list>"#,
        ));
        let mut diagnostics = no_diag();
        let result = build_namespaced_element(&NsScope::default(), &element, &mut diagnostics);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        assert_eq!(result.local, "list");
        assert!(
            result.refs.is_empty(),
            "a beans-ns <ref> child must never be harvested: {:?}",
            result.refs
        );
    }

    #[test]
    fn sb02c_attrs_preserves_own_attributes_excluding_xmlns() {
        let element = parse_fragment(concat!(
            r#"<util:constant id="maxRetries" "#,
            r#"static-field="com.example.Constants.MAX_RETRIES" "#,
            r#"xmlns:util="http://www.springframework.org/schema/util"/>"#,
        ));
        let mut diagnostics = no_diag();
        let result = build_namespaced_element(&NsScope::default(), &element, &mut diagnostics);
        let keys: Vec<&str> = result.attrs.iter().map(|a| a.key.value.as_str()).collect();
        assert!(keys.contains(&"id"));
        assert!(keys.contains(&"static-field"));
        assert!(
            !keys.iter().any(|k| k.starts_with("xmlns")),
            "xmlns declarations must be excluded from attrs: {keys:?}"
        );
    }

    // -------------------------------------------------------------
    // DEPTH_LIMIT downgrade (invariant #1 — no unbounded call-stack
    // recursion for an adversarially deep NS subtree).
    // -------------------------------------------------------------

    #[test]
    fn sb02c_depth_limit_downgrades_ref_harvest_with_diagnostic() {
        let element = parse_fragment(
            r#"<aop:aspect ref="a" xmlns:aop="http://www.springframework.org/schema/aop"><aop:pointcut id="p" expression="e"/></aop:aspect>"#,
        );
        let mut diagnostics = no_diag();
        let mut refs = Vec::new();
        harvest_refs(
            &NsScope::default(),
            &element,
            &mut refs,
            &mut diagnostics,
            crate::DEPTH_LIMIT,
        );
        // The element itself is still checked before the guard trips
        // (matches `parse_inner_bean`'s "guard before descending further"
        // shape) — its own ref is harvested, but no deeper recursion runs.
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].value.raw, "a");
        assert!(diagnostics
            .iter()
            .any(|d| d.code == DiagCode::NestingLimitExceeded));
    }

    proptest::proptest! {
        #[test]
        fn sb02c_proptest_arbitrary_depth_never_panics(depth in 0u32..2000) {
            let element = parse_fragment(r#"<aop:aspect ref="a" xmlns:aop="http://www.springframework.org/schema/aop"/>"#);
            let mut diagnostics = Vec::new();
            let mut refs = Vec::new();
            harvest_refs(&NsScope::default(), &element, &mut refs, &mut diagnostics, depth);
            proptest::prop_assert_eq!(refs.len(), 1);
        }
    }

    // -------------------------------------------------------------
    // DEPTH_LIMIT exercised via a genuinely deep tree built from source
    // (not just the `depth` PARAMETER on a flat/2-level element above) —
    // a real chain of nested `aop:config` wrapper elements, so the actual
    // per-level `depth + 1` recursive-descent path (`own_scope` rebuilt at
    // every level, one `harvest_refs` call frame per element) runs for
    // real, the same way `collection.rs`'s own
    // `sb07_proptest_nested_list_chain_depth_0_to_6_panic_free` builds a
    // real nested-element chain rather than only varying a depth
    // parameter.
    // -------------------------------------------------------------

    /// Builds `n` levels of nested `<aop:config>` wrapper elements (none of
    /// which match any `NS_REF_ALLOWLIST` row) around one innermost
    /// `<aop:aspect ref="deep"/>` — a genuine element chain whose actual
    /// tree depth is `n + 1`, not a flat element visited `n` times via the
    /// `depth` parameter.
    fn nested_aop_config_chain(n: u32) -> String {
        let mut source = String::new();
        source.push_str(r#"<aop:config xmlns:aop="http://www.springframework.org/schema/aop">"#);
        for _ in 0..n {
            source.push_str("<aop:config>");
        }
        source.push_str(r#"<aop:aspect ref="deep"/>"#);
        for _ in 0..n {
            source.push_str("</aop:config>");
        }
        source.push_str("</aop:config>");
        source
    }

    #[test]
    fn sb02c_real_deep_tree_below_limit_harvests_the_leaf_ref() {
        // Total real element depth (outer <aop:config> + wrappers +
        // innermost <aop:aspect>) stays comfortably under DEPTH_LIMIT, so
        // the innermost aspect's own `ref` must survive the actual
        // recursive descent through every real wrapper level.
        let n = crate::DEPTH_LIMIT - 10;
        let source = nested_aop_config_chain(n);
        let element = parse_fragment(&source);
        let mut diagnostics = no_diag();
        let result = build_namespaced_element(&NsScope::default(), &element, &mut diagnostics);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics on a real chain under the limit: {diagnostics:?}"
        );
        assert_eq!(result.refs.len(), 1);
        assert_eq!(result.refs[0].value.raw, "deep");
    }

    #[test]
    fn sb02c_real_deep_tree_beyond_limit_downgrades_before_reaching_the_leaf() {
        // Total real element depth now exceeds DEPTH_LIMIT, via genuine
        // nested elements rather than the `depth` parameter — the guard
        // must trip somewhere along the real descent, before the
        // recursion ever reaches the innermost `<aop:aspect ref="deep"/>`,
        // so that leaf ref must never be harvested and a
        // `NestingLimitExceeded` diagnostic must be present.
        //
        // `DEPTH_LIMIT + 10` stays well under `events::MAX_TREE_DEPTH`
        // (the P0 fix's own, deliberately much larger, cap on the raw
        // tree's own structural depth — see that constant's doc comment),
        // so `build_tree` builds this chain in full and this test still
        // exercises `harvest_refs`'s own guard specifically, not
        // `build_tree`'s.
        let n = crate::DEPTH_LIMIT + 10;
        let source = nested_aop_config_chain(n);
        let element = parse_fragment(&source);
        let mut diagnostics = no_diag();
        let result = build_namespaced_element(&NsScope::default(), &element, &mut diagnostics);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code == DiagCode::NestingLimitExceeded),
            "expected a NestingLimitExceeded diagnostic on a real over-limit chain: {diagnostics:?}"
        );
        assert!(
            result.refs.is_empty(),
            "the innermost aspect's ref lies beyond DEPTH_LIMIT and must never be reached: {:?}",
            result.refs
        );
    }
}
