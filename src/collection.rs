//! Unit **U5b** — collections (SB-07): `<list>`/`<set>`/`<array>`/`<map>`/
//! `<props>` → [`Collection`]. Per the build plan's own U5b row: "an item
//! **reuses U5a InjectValue** (value/ref/inner)" — every item, map key/value,
//! and nested collection resolves through `inject_value::parse_inject_value_child`
//! unchanged; "collection → U5b (self-recursion)" — this module's own entry point
//! ([`parse_collection_value`]) is exactly what that same function's
//! `"list"`/`"set"`/`"array"`/`"map"`/`"props"` match arm now calls, so a
//! `<list>` nested inside another `<list>` recurses back through the same
//! shared match, never a second reimplementation.
//!
//! U5a→U5b is a *serial* continuation of the same match, not a parallel
//! leaf pair (build plan: "U5b (collections) is serial after U5a") — this module wires
//! directly into `inject_value::parse_inject_value_child`'s own match arm
//! (previously reserved with a silent `None`), which is that module's own
//! seam, not one of the frozen root-/bean-child dispatch matches the
//! leaf-conflict-avoidance contract protects.
//!
//! Depth bookkeeping mirrors `inject_value::parse_inner_bean` exactly: the
//! incoming `depth` is checked against [`crate::DEPTH_LIMIT`] *before* any
//! recursion happens (downgrading to an opaque `InjectValue::Null` plus
//! `NestingLimitExceeded` instead), and every further descent — a list/set/
//! array item, a map entry's key/value, a nested collection — passes
//! `depth + 1`, exactly one increment per hop from a container to its own
//! content. `<entry>`/`<key>` wrapper elements are not themselves a hop
//! (same non-incrementing treatment `property::parse_property` gives its
//! own `<property>` wrapper before calling into `parse_inject_value_child`)
//! — only actual value/bean/collection descent counts.
//!
//! `<props>` is the one exception: `<prop key="...">text</prop>` entries
//! are always plain literals (no ref/inner/nested-collection shape the XSD
//! allows there), so [`parse_prop_entry`] needs neither `depth` nor
//! `diagnostics` at all.

use crate::dispatch::{
    element_text_segments, find_attr, find_bool_attr, is_beans_ns, merge_text_segments,
    resolve_qname, spanned_attr, NsScope,
};
use crate::events::{XmlAttr, XmlElement, XmlNode};
use crate::inject_value::{
    build_value_lit_from_segments, parse_inject_value_child, ref_from_attr, value_lit_from_attr,
};
use crate::model::{
    ClassRef, Collection, DiagCode, Diagnostic, InjectValue, MapEntry, PropEntry, Spanned,
};

/// Resolves one already-identified collection element (`<list>`/`<set>`/
/// `<array>`/`<map>`/`<props>`) into an `InjectValue::Collection` — the
/// single entry point `inject_value::parse_inject_value_child`'s own match
/// arm calls for all five element names. `scope` is the caller's
/// pre-overlay scope (same convention every recursive entry point in this
/// crate follows — see this module's own doc comment); this function
/// re-derives its own overlay via `NsScope::from_element` wherever it needs
/// one, rather than the caller doing it.
pub(crate) fn parse_collection_value(
    scope: &NsScope,
    diagnostics: &mut Vec<Diagnostic>,
    depth: u32,
    element: &XmlElement,
) -> InjectValue {
    if depth >= crate::DEPTH_LIMIT {
        diagnostics.push(Diagnostic {
            code: DiagCode::NestingLimitExceeded,
            span: Some(element.span),
            message: format!(
                "collection nesting exceeded {} levels; subtree treated as opaque",
                crate::DEPTH_LIMIT
            ),
        });
        return InjectValue::Null(element.span);
    }

    let own_scope = NsScope::from_element(element, Some(scope));
    let (_, local) = resolve_qname(&element.name, &own_scope);
    let collection = match local.as_str() {
        "list" => {
            let (items, value_type, merge) = parse_list_like(scope, diagnostics, depth, element);
            Collection::List {
                items,
                value_type,
                merge,
            }
        }
        "set" => {
            let (items, value_type, merge) = parse_list_like(scope, diagnostics, depth, element);
            Collection::Set {
                items,
                value_type,
                merge,
            }
        }
        "array" => {
            let (items, value_type, merge) = parse_list_like(scope, diagnostics, depth, element);
            Collection::Array {
                items,
                value_type,
                merge,
            }
        }
        "map" => parse_map(scope, diagnostics, depth, element),
        "props" => parse_props(scope, element),
        // Defensive only: `inject_value::parse_inject_value_child`'s own
        // match calls this function exclusively for the five names above.
        // No panic (rule 4) if that invariant is ever violated — an empty
        // list is the least surprising fallback.
        _ => Collection::List {
            items: Vec::new(),
            value_type: None,
            merge: None,
        },
    };
    InjectValue::Collection(Spanned {
        value: collection,
        span: element.span,
    })
}

// ---------------------------------------------------------------------
// <list>/<set>/<array> — identical shape (build plan/model: `ListLike`).
// ---------------------------------------------------------------------

/// Shared by `list`/`set`/`array`: every direct child element resolves
/// through `inject_value::parse_inject_value_child` (values, refs, inner
/// beans, *and* — since that function's own match now includes this
/// module's five names — nested collections), each one level deeper than
/// this collection itself (`depth + 1`). An unrecognized child (`None`)
/// is silently skipped, same "this function only ever resolves, never
/// opines" policy `parse_inject_value_child`'s own doc comment states.
fn parse_list_like(
    scope: &NsScope,
    diagnostics: &mut Vec<Diagnostic>,
    depth: u32,
    element: &XmlElement,
) -> (Vec<InjectValue>, Option<Spanned<ClassRef>>, Option<bool>) {
    let own_scope = NsScope::from_element(element, Some(scope));
    let mut items = Vec::new();
    for child in &element.children {
        if let XmlNode::Element(child_element) = child {
            if let Some(value) =
                parse_inject_value_child(&own_scope, diagnostics, depth + 1, child_element)
            {
                items.push(value);
            }
        }
    }
    let value_type = class_ref_from_attr(&element.attrs, "value-type");
    let merge = find_bool_attr(&element.attrs, "merge");
    (items, value_type, merge)
}

// ---------------------------------------------------------------------
// <map>.
// ---------------------------------------------------------------------

/// `<map key-type="..." value-type="..." merge="...">` — `key-type` and
/// (map-level, default-for-every-entry) `value-type` both live on the
/// `Collection::Map` variant itself; a per-`<entry>` `value-type` override
/// lands on that entry's own `MapEntry::value_type` instead (see
/// `parse_map_entry`).
fn parse_map(
    scope: &NsScope,
    diagnostics: &mut Vec<Diagnostic>,
    depth: u32,
    element: &XmlElement,
) -> Collection {
    let own_scope = NsScope::from_element(element, Some(scope));
    let key_type = class_ref_from_attr(&element.attrs, "key-type");
    let value_type = class_ref_from_attr(&element.attrs, "value-type");
    let merge = find_bool_attr(&element.attrs, "merge");

    let mut entries = Vec::new();
    for child in &element.children {
        if let XmlNode::Element(child_element) = child {
            // Per standard XML namespace scoping, a `xmlns`/`xmlns:*`
            // declaration on `<entry>` itself applies to that element's own
            // name — overlay the child's own declarations before resolving
            // it, same as `dispatch_root_child`/`dispatch_bean_child` do for
            // their own children (see those functions' doc comments).
            let child_scope = NsScope::from_element(child_element, Some(&own_scope));
            let (ns, local) = resolve_qname(&child_element.name, &child_scope);
            if local == "entry" && is_beans_ns(&ns) {
                entries.push(parse_map_entry(
                    &own_scope,
                    diagnostics,
                    depth + 1,
                    child_element,
                ));
            }
        }
    }

    Collection::Map {
        entries,
        key_type,
        value_type,
        merge,
    }
}

/// `<entry key= key-ref= value= value-ref= value-type=>`, plus the `<key>`
/// element form and a value-shaped child element. Precedence (both key and
/// value independently): the literal attribute wins, then the `-ref`
/// attribute, then a resolved child, then (nothing at all present) an
/// opaque `InjectValue::Null` at the entry's own span — same "never a
/// missing value, never a panic" fallback `property::resolve_value`
/// documents for `<property>`. Both `key`+`key-ref` and `value`+`value-ref`
/// present together raise `ConflictingValueAndRef` (additive — some
/// deterministic value/key is still produced), the same diagnostic
/// `property::parse_property` raises for its own `value=`/`ref=` pair.
///
/// `depth` here is already one hop past the owning `<map>` (see
/// `parse_map`'s call site) — passed through unchanged into both the
/// `<key>` child and the entry's own value child, since `<entry>`/`<key>`
/// are wrapper elements, not themselves a nesting hop (this module's own
/// doc comment).
fn parse_map_entry(
    scope: &NsScope,
    diagnostics: &mut Vec<Diagnostic>,
    depth: u32,
    element: &XmlElement,
) -> MapEntry {
    let own_scope = NsScope::from_element(element, Some(scope));

    let key_attr = find_attr(&element.attrs, "key");
    let key_ref_attr = find_attr(&element.attrs, "key-ref");
    if key_attr.is_some() && key_ref_attr.is_some() {
        diagnostics.push(Diagnostic {
            code: DiagCode::ConflictingValueAndRef,
            span: Some(element.span),
            message: "<entry> specifies both key= and key-ref=".to_string(),
        });
    }
    let value_attr = find_attr(&element.attrs, "value");
    let value_ref_attr = find_attr(&element.attrs, "value-ref");
    if value_attr.is_some() && value_ref_attr.is_some() {
        diagnostics.push(Diagnostic {
            code: DiagCode::ConflictingValueAndRef,
            span: Some(element.span),
            message: "<entry> specifies both value= and value-ref=".to_string(),
        });
    }
    let value_type = class_ref_from_attr(&element.attrs, "value-type");

    let mut key_child: Option<InjectValue> = None;
    let mut value_child: Option<InjectValue> = None;
    let mut seen_value_child = false;

    for child in &element.children {
        let XmlNode::Element(child_element) = child else {
            continue;
        };
        // Overlay `child_element`'s own `xmlns`/`xmlns:*` before resolving
        // its name — same namespace-scoping fix `parse_map`'s own
        // `<entry>` detection applies (see that call site's comment).
        let child_scope = NsScope::from_element(child_element, Some(&own_scope));
        let (ns, local) = resolve_qname(&child_element.name, &child_scope);
        if local == "key" && is_beans_ns(&ns) {
            if key_child.is_none() {
                key_child =
                    resolve_first_child_value(&own_scope, diagnostics, depth, child_element);
            }
            continue;
        }
        // The first non-`<key>` child element is this entry's value —
        // mirrors `property::parse_property`'s "only the first ... child is
        // resolved" leniency (the XSD only ever allows one).
        if !seen_value_child {
            seen_value_child = true;
            value_child = parse_inject_value_child(&own_scope, diagnostics, depth, child_element);
        }
    }

    let key = key_attr
        .map(|attr| InjectValue::Value(value_lit_from_attr(attr)))
        .or_else(|| {
            key_ref_attr.and_then(|attr| ref_from_attr(attr, diagnostics).map(InjectValue::Ref))
        })
        .or(key_child)
        .unwrap_or(InjectValue::Null(element.span));

    let value = value_attr
        .map(|attr| InjectValue::Value(value_lit_from_attr(attr)))
        .or_else(|| {
            value_ref_attr.and_then(|attr| ref_from_attr(attr, diagnostics).map(InjectValue::Ref))
        })
        .or(value_child)
        .unwrap_or(InjectValue::Null(element.span));

    MapEntry {
        span: element.span,
        key,
        value,
        value_type,
    }
}

/// `<key>...</key>` wraps exactly one value-shaped (or collection-shaped)
/// child per the XSD; this crate has no schema view, so — same leniency
/// `ref_from_element`'s own doc comment documents for `<ref>`'s
/// bean=/local=/parent= triad — only the first child element found is ever
/// resolved, whatever it is.
fn resolve_first_child_value(
    scope: &NsScope,
    diagnostics: &mut Vec<Diagnostic>,
    depth: u32,
    element: &XmlElement,
) -> Option<InjectValue> {
    let own_scope = NsScope::from_element(element, Some(scope));
    for child in &element.children {
        if let XmlNode::Element(child_element) = child {
            return parse_inject_value_child(&own_scope, diagnostics, depth, child_element);
        }
    }
    None
}

// ---------------------------------------------------------------------
// <props>.
// ---------------------------------------------------------------------

/// `<props merge="...">` — every `<prop key="...">` child becomes a
/// [`PropEntry`]; anything else is silently skipped (same "no opinion on
/// an unrecognized shape" policy this whole module follows). No `depth`/
/// `diagnostics` needed: a `<prop>` value is always a plain literal (the
/// XSD gives it no ref/inner/nested-collection form), so there is nothing
/// here that can recurse or need a `RefWithoutTarget`-shaped diagnostic.
fn parse_props(scope: &NsScope, element: &XmlElement) -> Collection {
    let own_scope = NsScope::from_element(element, Some(scope));
    let merge = find_bool_attr(&element.attrs, "merge");
    let mut entries = Vec::new();
    for child in &element.children {
        if let XmlNode::Element(child_element) = child {
            // Overlay `child_element`'s own declarations before resolving
            // its name — same namespace-scoping fix `parse_map`'s own
            // `<entry>` detection applies (see that call site's comment).
            let child_scope = NsScope::from_element(child_element, Some(&own_scope));
            let (ns, local) = resolve_qname(&child_element.name, &child_scope);
            if local == "prop" && is_beans_ns(&ns) {
                entries.push(parse_prop_entry(child_element));
            }
        }
    }
    Collection::Props { entries, merge }
}

/// `<prop key="...">literal text</prop>` → a [`PropEntry`]. `key=` absent
/// falls back to an empty spanned string at the element's own span — same
/// infallible fallback `property::parse_property` documents for a missing
/// `<property name=>` (rule 4: no panics, no invented `DiagCode` for a
/// shape the spec's edge-case table doesn't call out).
fn parse_prop_entry(element: &XmlElement) -> PropEntry {
    let key = find_attr(&element.attrs, "key")
        .map(spanned_attr)
        .unwrap_or_else(|| Spanned {
            value: String::new(),
            span: element.span,
        });
    let segments = element_text_segments(element);
    let text = merge_text_segments(&segments, element.span);
    let value = build_value_lit_from_segments(&segments, text, element.span, None);
    PropEntry {
        span: element.span,
        key,
        value,
    }
}

// ---------------------------------------------------------------------
// value-type= / key-type= — the one `ClassRef`-bearing attribute shape
// this unit produces, three times over (list/set/array's `value-type`,
// map's `key-type`/`value-type`, entry's `value-type`).
// ---------------------------------------------------------------------

/// Mirrors `bean::parse_class_ref`/`inject_value::parse_type_attr`'s
/// identical policy: invariant #5 (`ClassRef.raw` never empty) is upheld
/// by simply never constructing one from an absent or present-but-empty
/// attribute value.
fn class_ref_from_attr(attrs: &[XmlAttr], name: &str) -> Option<Spanned<ClassRef>> {
    let attr = find_attr(attrs, name)?;
    if attr.value.value.is_empty() {
        return None;
    }
    Some(Spanned {
        value: ClassRef {
            raw: attr.value.value.clone(),
        },
        span: attr.value.span,
    })
}

// ---------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------
//
// In-module (rather than only `tests/u5b_collection.rs`) for the same
// reason `inject_value`'s own suite lives here: every type/function in
// this module is `pub(crate)`/private — a seam not visible from an
// external integration-test binary. `tests/u5b_collection.rs` carries the
// same pointer-plus-smoke-test shape those files already established.

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

    fn parse(source: &str) -> InjectValue {
        let element = parse_fragment(source);
        let mut diagnostics = no_diag();
        let result = parse_collection_value(&NsScope::default(), &mut diagnostics, 0, &element);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics for {source}: {diagnostics:?}"
        );
        result
    }

    // -------------------------------------------------------------
    // Snapshot per collection kind.
    // -------------------------------------------------------------

    #[test]
    fn sb07_list_snapshot() {
        let result = parse(concat!(
            "<list value-type=\"java.lang.String\">",
            "<value>a</value><value>b</value>",
            "</list>"
        ));
        insta::assert_json_snapshot!(result);
    }

    #[test]
    fn sb07_set_snapshot() {
        let result = parse("<set><value>a</value><ref bean=\"b\"/></set>");
        insta::assert_json_snapshot!(result);
    }

    #[test]
    fn sb07_array_snapshot() {
        let result = parse("<array><value>1</value><value>2</value></array>");
        insta::assert_json_snapshot!(result);
    }

    #[test]
    fn sb07_map_snapshot() {
        let result = parse(concat!(
            "<map key-type=\"java.lang.String\" value-type=\"java.lang.Integer\">",
            "<entry key=\"a\" value=\"1\"/>",
            "<entry key=\"b\" value=\"2\"/>",
            "</map>"
        ));
        insta::assert_json_snapshot!(result);
    }

    #[test]
    fn sb07_props_snapshot() {
        let result = parse(concat!(
            "<props>",
            "<prop key=\"driver\">com.example.Driver</prop>",
            "<prop key=\"url\">jdbc:example</prop>",
            "</props>"
        ));
        insta::assert_json_snapshot!(result);
    }

    // -------------------------------------------------------------
    // merge="true".
    // -------------------------------------------------------------

    #[test]
    fn sb07_list_merge_true_is_recorded() {
        let result = parse("<list merge=\"true\"><value>a</value></list>");
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::List { merge, .. } => assert_eq!(merge, Some(true)),
                other => panic!("expected List, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    #[test]
    fn sb07_map_merge_and_props_merge_are_recorded() {
        let map = parse("<map merge=\"true\"><entry key=\"a\" value=\"1\"/></map>");
        match map {
            InjectValue::Collection(c) => match c.value {
                Collection::Map { merge, .. } => assert_eq!(merge, Some(true)),
                other => panic!("expected Map, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }

        let props = parse("<props merge=\"false\"><prop key=\"a\">1</prop></props>");
        match props {
            InjectValue::Collection(c) => match c.value {
                Collection::Props { merge, .. } => assert_eq!(merge, Some(false)),
                other => panic!("expected Props, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    #[test]
    fn sb07_merge_absent_is_none() {
        let result = parse("<list><value>a</value></list>");
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::List { merge, .. } => assert_eq!(merge, None),
                other => panic!("expected List, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    // -------------------------------------------------------------
    // <entry key-ref value-ref>.
    // -------------------------------------------------------------

    #[test]
    fn sb07_entry_key_ref_and_value_ref_snapshot() {
        let result = parse(concat!(
            "<map>",
            "<entry key-ref=\"keyBean\" value-ref=\"valueBean\"/>",
            "</map>"
        ));
        insta::assert_json_snapshot!(result);
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::Map { entries, .. } => {
                    assert_eq!(entries.len(), 1);
                    match &entries[0].key {
                        InjectValue::Ref(r) => assert_eq!(r.value.raw, "keyBean"),
                        other => panic!("expected Ref, got {other:?}"),
                    }
                    match &entries[0].value {
                        InjectValue::Ref(r) => assert_eq!(r.value.raw, "valueBean"),
                        other => panic!("expected Ref, got {other:?}"),
                    }
                }
                other => panic!("expected Map, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    // -------------------------------------------------------------
    // <key> element form.
    // -------------------------------------------------------------

    #[test]
    fn sb07_entry_key_element_form_snapshot() {
        let result = parse(concat!(
            "<map>",
            "<entry value=\"1\"><key><value>k</value></key></entry>",
            "</map>"
        ));
        insta::assert_json_snapshot!(result);
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::Map { entries, .. } => {
                    assert_eq!(entries.len(), 1);
                    match &entries[0].key {
                        InjectValue::Value(v) => assert_eq!(v.text.value, "k"),
                        other => panic!("expected Value, got {other:?}"),
                    }
                }
                other => panic!("expected Map, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    #[test]
    fn sb07_entry_key_element_wrapping_ref_snapshot() {
        let result = parse(concat!(
            "<map>",
            "<entry value=\"1\"><key><ref bean=\"keyBean\"/></key></entry>",
            "</map>"
        ));
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::Map { entries, .. } => match &entries[0].key {
                    InjectValue::Ref(r) => assert_eq!(r.value.raw, "keyBean"),
                    other => panic!("expected Ref, got {other:?}"),
                },
                other => panic!("expected Map, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    // -------------------------------------------------------------
    // Namespace filtering — negative branch. `is_beans_ns` guards
    // `<entry>`/`<key>`/`<prop>` detection (this module's own doc comment,
    // cold-review test-gap finding): a foreign-namespaced child must be
    // skipped, whether via an inherited prefix or via a redeclaration the
    // child element carries on itself (the latter overlay is the fix for
    // the companion spec-deviation finding — see `parse_map`'s inline
    // comment at its `<entry>` detection call site).
    // -------------------------------------------------------------

    #[test]
    fn sb07_map_prefixed_foreign_entry_is_skipped() {
        let result = parse(concat!(
            "<map xmlns:foo=\"urn:not-spring\">",
            "<foo:entry key=\"a\" value=\"1\"/>",
            "</map>"
        ));
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::Map { entries, .. } => assert!(entries.is_empty()),
                other => panic!("expected Map, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    #[test]
    fn sb07_map_entry_with_own_xmlns_redeclaration_is_skipped() {
        // The <entry> element redeclares its own default namespace to
        // something other than the beans namespace. Per standard XML
        // namespace scoping, that declaration applies to the element it's
        // on, so this must not be recognized as a beans <entry>.
        let result = parse("<map><entry xmlns=\"urn:not-spring\" key=\"a\" value=\"1\"/></map>");
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::Map { entries, .. } => assert!(entries.is_empty()),
                other => panic!("expected Map, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    #[test]
    fn sb07_props_prefixed_foreign_prop_is_skipped() {
        let result = parse(concat!(
            "<props xmlns:foo=\"urn:not-spring\">",
            "<foo:prop key=\"a\">1</foo:prop>",
            "</props>"
        ));
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::Props { entries, .. } => assert!(entries.is_empty()),
                other => panic!("expected Props, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    #[test]
    fn sb07_props_prop_with_own_xmlns_redeclaration_is_skipped() {
        let result = parse("<props><prop xmlns=\"urn:not-spring\" key=\"a\">1</prop></props>");
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::Props { entries, .. } => assert!(entries.is_empty()),
                other => panic!("expected Props, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    #[test]
    fn sb07_entry_key_prefixed_foreign_namespace_is_not_treated_as_key() {
        let result = parse(concat!(
            "<map xmlns:foo=\"urn:not-spring\">",
            "<entry value=\"1\"><foo:key><value>k</value></foo:key></entry>",
            "</map>"
        ));
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::Map { entries, .. } => {
                    assert!(matches!(entries[0].key, InjectValue::Null(_)));
                }
                other => panic!("expected Map, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    #[test]
    fn sb07_entry_key_with_own_xmlns_redeclaration_is_not_treated_as_key() {
        // The <key> element redeclares its own default namespace; per
        // standard XML namespace scoping that applies to <key> itself, so
        // it must not be recognized as a beans <key> even though the
        // enclosing <entry>/<map> are unqualified.
        let result = parse(concat!(
            "<map>",
            "<entry value=\"1\"><key xmlns=\"urn:not-spring\"><value>k</value></key></entry>",
            "</map>"
        ));
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::Map { entries, .. } => {
                    assert!(matches!(entries[0].key, InjectValue::Null(_)));
                }
                other => panic!("expected Map, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    // -------------------------------------------------------------
    // map-level key-type vs entry-level value-type.
    // -------------------------------------------------------------

    #[test]
    fn sb07_map_key_type_lands_on_collection_map_entry_value_type_lands_on_map_entry() {
        let result = parse(concat!(
            "<map key-type=\"java.lang.String\">",
            "<entry key=\"a\" value=\"1\" value-type=\"java.lang.Integer\"/>",
            "</map>"
        ));
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::Map {
                    key_type,
                    value_type,
                    entries,
                    ..
                } => {
                    assert_eq!(
                        key_type.as_ref().map(|t| t.value.raw.as_str()),
                        Some("java.lang.String")
                    );
                    // No map-level value-type attribute here — the map's
                    // own field stays None; the entry's own override is
                    // what carries "java.lang.Integer".
                    assert_eq!(value_type, None);
                    assert_eq!(
                        entries[0].value_type.as_ref().map(|t| t.value.raw.as_str()),
                        Some("java.lang.Integer")
                    );
                }
                other => panic!("expected Map, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    #[test]
    fn sb07_map_level_value_type_is_distinct_from_entry_level_value_type() {
        let result = parse(concat!(
            "<map value-type=\"java.lang.String\">",
            "<entry key=\"a\" value=\"1\"/>",
            "</map>"
        ));
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::Map {
                    value_type,
                    entries,
                    ..
                } => {
                    assert_eq!(
                        value_type.as_ref().map(|t| t.value.raw.as_str()),
                        Some("java.lang.String")
                    );
                    assert_eq!(entries[0].value_type, None);
                }
                other => panic!("expected Map, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    // -------------------------------------------------------------
    // Nested collection with an inner ref (quartz-style
    // <list><ref bean=.../></list> inside a <map> entry value).
    // -------------------------------------------------------------

    #[test]
    fn sb07_nested_collection_with_inner_ref_snapshot() {
        let result = parse(concat!(
            "<map>",
            "<entry key=\"triggers\">",
            "<list><ref bean=\"triggerBean\"/><value>literal</value></list>",
            "</entry>",
            "</map>"
        ));
        insta::assert_json_snapshot!(result);
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::Map { entries, .. } => match &entries[0].value {
                    InjectValue::Collection(inner) => match &inner.value {
                        Collection::List { items, .. } => {
                            assert_eq!(items.len(), 2);
                            match &items[0] {
                                InjectValue::Ref(r) => assert_eq!(r.value.raw, "triggerBean"),
                                other => panic!("expected Ref, got {other:?}"),
                            }
                        }
                        other => panic!("expected List, got {other:?}"),
                    },
                    other => panic!("expected nested Collection, got {other:?}"),
                },
                other => panic!("expected Map, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    #[test]
    fn sb07_nested_list_of_lists_structural() {
        let result = parse("<list><list><value>deep</value></list></list>");
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::List { items, .. } => {
                    assert_eq!(items.len(), 1);
                    match &items[0] {
                        InjectValue::Collection(inner) => match &inner.value {
                            Collection::List { items, .. } => {
                                assert_eq!(items.len(), 1);
                                match &items[0] {
                                    InjectValue::Value(v) => assert_eq!(v.text.value, "deep"),
                                    other => panic!("expected Value, got {other:?}"),
                                }
                            }
                            other => panic!("expected inner List, got {other:?}"),
                        },
                        other => panic!("expected Collection, got {other:?}"),
                    }
                }
                other => panic!("expected List, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    // -------------------------------------------------------------
    // Conflicting key/key-ref and value/value-ref on the same <entry>.
    // -------------------------------------------------------------

    #[test]
    fn sb07_entry_key_and_key_ref_both_present_is_conflicting_value_and_ref() {
        let element = parse_fragment(concat!(
            "<map>",
            "<entry key=\"a\" key-ref=\"aBean\" value=\"1\"/>",
            "</map>"
        ));
        let mut diagnostics = no_diag();
        let _ = parse_collection_value(&NsScope::default(), &mut diagnostics, 0, &element);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == DiagCode::ConflictingValueAndRef));
    }

    #[test]
    fn sb07_entry_value_and_value_ref_both_present_is_conflicting_value_and_ref() {
        let element = parse_fragment(concat!(
            "<map>",
            "<entry key=\"a\" value=\"1\" value-ref=\"aBean\"/>",
            "</map>"
        ));
        let mut diagnostics = no_diag();
        let _ = parse_collection_value(&NsScope::default(), &mut diagnostics, 0, &element);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == DiagCode::ConflictingValueAndRef));
    }

    // -------------------------------------------------------------
    // Entry / prop with nothing resolved falls back to Null, never panics.
    // -------------------------------------------------------------

    #[test]
    fn sb07_entry_with_no_key_at_all_falls_back_to_null() {
        let result = parse("<map><entry value=\"1\"/></map>");
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::Map { entries, .. } => {
                    assert!(matches!(entries[0].key, InjectValue::Null(_)));
                }
                other => panic!("expected Map, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    #[test]
    fn sb07_prop_without_key_attr_falls_back_to_empty_key() {
        let result = parse("<props><prop>orphan</prop></props>");
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::Props { entries, .. } => {
                    assert_eq!(entries[0].key.value, "");
                    assert_eq!(entries[0].value.text.value, "orphan");
                }
                other => panic!("expected Props, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    // -------------------------------------------------------------
    // span: every collection/entry/prop carries its own span.
    // -------------------------------------------------------------

    #[test]
    fn sb07_list_span_covers_the_whole_element() {
        let source = "<list><value>a</value></list>";
        let element = parse_fragment(source);
        let mut diagnostics = no_diag();
        let result = parse_collection_value(&NsScope::default(), &mut diagnostics, 0, &element);
        match result {
            InjectValue::Collection(c) => {
                assert_eq!(&source[c.span.start as usize..c.span.end as usize], source);
            }
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    #[test]
    fn sb07_entry_and_prop_span_cover_their_own_elements() {
        let source = "<map><entry key=\"a\" value=\"1\"/></map>";
        let element = parse_fragment(source);
        let mut diagnostics = no_diag();
        let result = parse_collection_value(&NsScope::default(), &mut diagnostics, 0, &element);
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::Map { entries, .. } => {
                    let entry = &entries[0];
                    assert_eq!(
                        &source[entry.span.start as usize..entry.span.end as usize],
                        "<entry key=\"a\" value=\"1\"/>"
                    );
                }
                other => panic!("expected Map, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }

        let props_source = "<props><prop key=\"a\">1</prop></props>";
        let props_element = parse_fragment(props_source);
        let mut diagnostics2 = no_diag();
        let props_result =
            parse_collection_value(&NsScope::default(), &mut diagnostics2, 0, &props_element);
        match props_result {
            InjectValue::Collection(c) => match c.value {
                Collection::Props { entries, .. } => {
                    let prop = &entries[0];
                    assert_eq!(
                        &props_source[prop.span.start as usize..prop.span.end as usize],
                        "<prop key=\"a\">1</prop>"
                    );
                }
                other => panic!("expected Props, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    // -------------------------------------------------------------
    // value-type on list/set/array.
    // -------------------------------------------------------------

    #[test]
    fn sb07_list_value_type_is_recorded() {
        let result = parse("<list value-type=\"java.lang.String\"><value>a</value></list>");
        match result {
            InjectValue::Collection(c) => match c.value {
                Collection::List { value_type, .. } => {
                    assert_eq!(
                        value_type.as_ref().map(|t| t.value.raw.as_str()),
                        Some("java.lang.String")
                    );
                }
                other => panic!("expected List, got {other:?}"),
            },
            other => panic!("expected Collection, got {other:?}"),
        }
    }

    // -------------------------------------------------------------
    // DEPTH_LIMIT — pinned boundary case, mirroring
    // `inject_value::tests::sb06_depth_limit_downgrades_inner_bean_to_null_plus_diagnostic`.
    // -------------------------------------------------------------

    #[test]
    fn sb07_depth_limit_downgrades_collection_to_null_plus_diagnostic() {
        let element = parse_fragment("<list><value>a</value></list>");
        let mut diagnostics = no_diag();
        let result = parse_collection_value(
            &NsScope::default(),
            &mut diagnostics,
            crate::DEPTH_LIMIT,
            &element,
        );
        assert_eq!(result, InjectValue::Null(element.span));
        assert!(diagnostics
            .iter()
            .any(|d| d.code == DiagCode::NestingLimitExceeded));
    }

    #[test]
    fn sb07_depth_one_below_limit_still_recurses_into_a_real_collection() {
        let element = parse_fragment("<list><value>a</value></list>");
        let mut diagnostics = no_diag();
        let result = parse_collection_value(
            &NsScope::default(),
            &mut diagnostics,
            crate::DEPTH_LIMIT - 1,
            &element,
        );
        assert!(!diagnostics
            .iter()
            .any(|d| d.code == DiagCode::NestingLimitExceeded));
        assert!(matches!(result, InjectValue::Collection(_)));
    }

    // -------------------------------------------------------------
    // proptest: DEPTH_LIMIT boundary (reuses the exact same DEPTH_LIMIT
    // thread `inject_value`'s own
    // `sb06_proptest_arbitrary_depth_never_panics_and_downgrades_exactly_at_the_limit`
    // pins for inner beans — this is that same guard, exercised at this
    // module's own call boundary).
    // -------------------------------------------------------------

    proptest::proptest! {
        #[test]
        fn sb07_proptest_arbitrary_depth_never_panics_and_downgrades_exactly_at_the_limit(
            depth in 0u32..2000,
        ) {
            let element = parse_fragment("<list><value>a</value></list>");
            let mut diagnostics = Vec::new();
            let result = parse_collection_value(&NsScope::default(), &mut diagnostics, depth, &element);
            let downgraded = diagnostics.iter().any(|d| d.code == DiagCode::NestingLimitExceeded);
            if depth >= crate::DEPTH_LIMIT {
                proptest::prop_assert!(downgraded, "expected NestingLimitExceeded at depth {}", depth);
                proptest::prop_assert!(matches!(result, InjectValue::Null(_)));
            } else {
                proptest::prop_assert!(!downgraded, "unexpected NestingLimitExceeded at depth {}", depth);
                proptest::prop_assert!(matches!(result, InjectValue::Collection(_)));
            }
        }

        // Structural nested-depth proptest: a chain of N nested <list>
        // elements, N in 0..6 (build plan's own SB-15/U5a generator range
        // for nested depth), must always parse to exactly N levels of
        // `Collection::List` without panicking, bottoming out at a single
        // `<value>` leaf.
        #[test]
        fn sb07_proptest_nested_list_chain_depth_0_to_6_panic_free(
            n in 0u32..6,
        ) {
            let mut source = String::new();
            for _ in 0..n {
                source.push_str("<list>");
            }
            source.push_str("<value>leaf</value>");
            for _ in 0..n {
                source.push_str("</list>");
            }
            let wrapped = format!("<list>{source}</list>");
            let element = parse_fragment(&wrapped);
            let mut diagnostics = Vec::new();
            let result = parse_collection_value(&NsScope::default(), &mut diagnostics, 0, &element);
            proptest::prop_assert!(
                !diagnostics.iter().any(|d| d.code == DiagCode::NestingLimitExceeded)
            );

            // Walk down exactly n+1 levels (the outer wrapping <list> plus
            // the n generated ones) and confirm the leaf <value> survives.
            let mut current = result;
            for _ in 0..=n {
                match current {
                    InjectValue::Collection(c) => match c.value {
                        Collection::List { mut items, .. } => {
                            proptest::prop_assert_eq!(items.len(), 1);
                            current = items.remove(0);
                        }
                        other => return Err(proptest::test_runner::TestCaseError::fail(format!(
                            "expected List, got {other:?}"
                        ))),
                    },
                    other => return Err(proptest::test_runner::TestCaseError::fail(format!(
                        "expected Collection, got {other:?}"
                    ))),
                }
            }
            match current {
                InjectValue::Value(v) => proptest::prop_assert_eq!(v.text.value, "leaf"),
                other => return Err(proptest::test_runner::TestCaseError::fail(format!(
                    "expected leaf Value, got {other:?}"
                ))),
            }
        }
    }
}
