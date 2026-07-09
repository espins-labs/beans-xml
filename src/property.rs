//! Unit **U6** — `<property>` (SB-04): wraps [`InjectValue`] (U5a) with
//! `name` + `<meta>` into a [`Property`], and wires the real call into
//! `bean::dispatch_bean_child`'s `"property"` arm — the one arm that
//! module's own doc comment explicitly reserves for this unit ("U6 wires
//! the real call here, not this unit"), unlike the five frozen stubs
//! (`parse_qualifier`/`parse_meta`/`parse_decorator`/`parse_lookup_method`/
//! `parse_replaced_method`) a P-leaf fills without ever touching the match
//! itself. Per the build plan's U6 row: "wrap InjectValue with name/index/
//! type/meta", built on U5a, parallel with U7 (`ConstructorArg`, SB-05).
//!
//! Value resolution precedence, since `<property>` may carry a `value=`
//! shorthand attribute, a `ref=` shorthand attribute, and/or a single
//! value-shaped child element (`<value>`/`<ref>`/`<idref>`/`<null>`/
//! `<bean>`) — none of which is mutually exclusive by construction (only
//! the Spring XSD forbids combining them, and this crate has no schema
//! view, spec's "settled decisions"):
//!
//! 1. `value=` wins if present (deterministic tie-break, not a semantic
//!    claim that it's "more correct" than `ref=`).
//! 2. Otherwise `ref=`, if present and non-empty (an empty `ref=` already
//!    raises `RefWithoutTarget` via [`crate::inject_value::ref_from_attr`]
//!    and falls through to the next step).
//! 3. Otherwise the first recognized value-shaped child element.
//! 4. Otherwise (no `value=`/`ref=`/recognized child at all — a malformed
//!    `<property>` with nothing to inject) `InjectValue::Null` at the
//!    `<property>` element's own span: the closest non-panicking, no-new-
//!    `DiagCode` fallback (spec's edge-case table doesn't call out this
//!    shape; rule 4 — no panics, never `Err` — still applies to it).
//!
//! `value=` **and** `ref=` both present is diagnosed as
//! `ConflictingValueAndRef` regardless of which one the precedence above
//! picks — the diagnostic is additive (both the property and *some*
//! deterministic value are still produced, same "preserve both, diagnose
//! once" policy `dispatch::dispatch_root_child`'s `DuplicateBeanId` arm
//! documents for a duplicate bean id).
//!
//! `<meta key= value=>` children are `Property`'s own field (distinct from
//! `Bean::meta`, P6's stub) — read locally here rather than shared with
//! `bean::parse_meta`, since that stub pushes into `BeanCtx::meta`, a
//! different accumulator than the `Vec<MetaEntry>` a `Property` carries.

use crate::dispatch::{find_attr, is_beans_ns, resolve_qname, spanned_attr, NsScope};
use crate::events::{XmlElement, XmlNode};
use crate::inject_value::{parse_inject_value_child, ref_from_attr, value_lit_from_attr};
use crate::model::{BeanCtx, DiagCode, Diagnostic, InjectValue, MetaEntry, Property, Spanned};

/// Parses one `<property>` element — a `<bean>`-body child, dispatched from
/// `bean::dispatch_bean_child`'s `"property"` arm — into a [`Property`],
/// pushed onto `ctx.properties`.
///
/// `scope` is the namespace scope in effect for `element` *before*
/// overlaying whatever `xmlns`/`xmlns:*` declarations `element` itself
/// carries — the same "caller passes its own pre-overlay scope, callee
/// overlays itself if it needs to recurse" convention `bean::parse_bean`
/// and `dispatch::dispatch_root_child`'s handler stubs already follow.
///
/// `depth` is the enclosing `<bean>`'s own nesting depth, forwarded
/// unchanged from `bean::dispatch_bean_child` into
/// `inject_value::parse_inject_value_child` below for this property's own
/// value-shaped child (if any) — the single [`crate::DEPTH_LIMIT`] choke
/// point that bounds bean→property→inner-bean recursion. It must **not** be
/// hardcoded to `0` here: doing so would reset the guard at every
/// `<property>` level and defeat it entirely for any bean reached through a
/// property's inner `<bean>`.
pub(crate) fn parse_property(
    ctx: &mut BeanCtx,
    diagnostics: &mut Vec<Diagnostic>,
    scope: &NsScope,
    element: &XmlElement,
    depth: u32,
) {
    // `name=` absent is not among this unit's tested edge cases (Spring's
    // own XSD makes it mandatory) — falling back to an empty spanned string
    // at the element's own span keeps this infallible (rule 4) without
    // inventing a new `DiagCode` for a shape the spec's edge-case table
    // doesn't call out; invariant #5 (never-empty) only binds
    // `BeanRef.raw`/`ClassRef.raw`, not `Property.name`.
    let name = find_attr(&element.attrs, "name")
        .map(spanned_attr)
        .unwrap_or_else(|| Spanned {
            value: String::new(),
            span: element.span,
        });

    let value_attr = find_attr(&element.attrs, "value");
    let ref_attr = find_attr(&element.attrs, "ref");

    if value_attr.is_some() && ref_attr.is_some() {
        diagnostics.push(Diagnostic {
            code: DiagCode::ConflictingValueAndRef,
            span: Some(element.span),
            message: format!(
                "<property name=\"{}\"> specifies both value= and ref=",
                name.value
            ),
        });
    }

    // Own overlay — this element's own `xmlns`/`xmlns:*` (rare on
    // `<property>` itself, but the same convention every other recursive
    // site in this crate follows) in effect for its children.
    let own_scope = NsScope::from_element(element, Some(scope));

    let mut meta = Vec::new();
    let mut child_value: Option<InjectValue> = None;
    let mut seen_value_child = false;

    for child in &element.children {
        let XmlNode::Element(child_element) = child else {
            // Direct text (whitespace between child elements) has nothing
            // to attach to at this level — same drop `bean::parse_bean`'s
            // own child loop documents.
            continue;
        };
        // Overlay `child_element`'s own `xmlns`/`xmlns:*` declarations
        // before resolving its name — a xmlns declaration on `<meta>`
        // itself applies to that element's own name, same as
        // `dispatch_root_child`/`dispatch_bean_child` do for their own
        // children (see those functions' doc comments, and
        // `collection.rs`'s `parse_map`/`parse_map_entry`/`bean.rs`'s
        // `parse_qualifier`, which document this identical fix for their
        // own nested-element detection).
        let child_scope = NsScope::from_element(child_element, Some(&own_scope));
        let (ns, local) = resolve_qname(&child_element.name, &child_scope);
        if local == "meta" && is_beans_ns(&ns) {
            if let Some(entry) = parse_meta_entry(child_element) {
                meta.push(entry);
            }
            continue;
        }
        // Only the first non-meta child is resolved as this property's
        // value — the XSD only ever allows one; further children are
        // silently ignored (lenient parser, no diagnostic invented for a
        // shape the spec's edge-case table doesn't call out) rather than
        // re-invoking `parse_inject_value_child` and risking duplicate
        // diagnostics (e.g. a stray second `<ref/>` with no target).
        if !seen_value_child {
            seen_value_child = true;
            child_value = parse_inject_value_child(&own_scope, diagnostics, depth, child_element);
        }
    }

    let value = resolve_value(value_attr, ref_attr, child_value, diagnostics, element.span);

    ctx.properties.push(Property {
        span: element.span,
        name,
        value,
        meta,
    });
}

/// Precedence documented on [`parse_property`]'s own doc comment: `value=`
/// wins, then `ref=`, then the first recognized value-shaped child, then
/// (nothing at all resolved) an opaque `Null` at `span` — never a panic,
/// never a missing `Property.value`.
///
/// Ratification: `value=` (or `ref=`) together with a value-shaped child
/// (e.g. `<property value="x"><ref bean="y"/></property>`) is intentionally
/// **not** flagged `ConflictingValueAndRef` — that diagnostic only compares
/// the two shorthand *attributes* against each other (see
/// [`parse_property`]'s own attribute check above `resolve_value`'s call
/// site). Leniency-over-XSD: this crate records whichever value the
/// precedence chain below picks and lets the consumer notice the shape is
/// unusual, rather than this parser inventing an opinion about it.
fn resolve_value(
    value_attr: Option<&crate::events::XmlAttr>,
    ref_attr: Option<&crate::events::XmlAttr>,
    child_value: Option<InjectValue>,
    diagnostics: &mut Vec<Diagnostic>,
    span: crate::model::ByteSpan,
) -> InjectValue {
    if let Some(attr) = value_attr {
        return InjectValue::Value(value_lit_from_attr(attr));
    }
    if let Some(attr) = ref_attr {
        if let Some(bean_ref) = ref_from_attr(attr, diagnostics) {
            return InjectValue::Ref(bean_ref);
        }
        // Empty ref= already raised RefWithoutTarget inside ref_from_attr
        // — fall through to a child value (if any) rather than stopping
        // here.
    }
    if let Some(value) = child_value {
        return value;
    }
    InjectValue::Null(span)
}

/// `<meta key="..." value="...">` → a [`MetaEntry`], or `None` when either
/// attribute is missing — lenient skip, no diagnostic invented for a shape
/// the spec's edge-case table doesn't call out (mirrors this module's own
/// "no new `DiagCode` for untested shapes" policy above).
fn parse_meta_entry(element: &XmlElement) -> Option<MetaEntry> {
    let key = find_attr(&element.attrs, "key")?;
    let value = find_attr(&element.attrs, "value")?;
    Some(MetaEntry {
        key: spanned_attr(key),
        value: spanned_attr(value),
    })
}
