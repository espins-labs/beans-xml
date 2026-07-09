//! Unit **U7** — `<constructor-arg>` (SB-05): wraps [`InjectValue`] (U5a)
//! with `index`/`type`/`name` + `<meta>` into a [`ConstructorArg`], and
//! wires the real call into `bean::dispatch_bean_child`'s
//! `"constructor-arg"` arm — the arm that module's own doc comment
//! explicitly reserves for this unit ("U7's `\"constructor-arg\"` arm will
//! need the same [`depth`] once it lands" / "same reservation, for U7"),
//! same treatment as U6's `"property"` arm before it. Per the build plan's
//! U7 row: "wrap InjectValue with name/index/type/meta", built on U5a,
//! parallel with U6 (`Property`, SB-04).
//!
//! Value resolution precedence mirrors `property::parse_property` exactly
//! (see that function's own doc comment for the full rationale) — the two
//! units are deliberately symmetric, since `<constructor-arg>` carries the
//! identical `value=`/`ref=`/value-shaped-child triad `<property>` does,
//! plus three attributes `<property>` doesn't have:
//!
//! - `index="0"` — positional index. Parsed as `u32` when present and
//!   valid; a missing, empty, or non-numeric `index=` all fall back to
//!   `None` (an "indexless" arg, resolved by document order per the spec's
//!   edge-case table — resolving that order into an effective position is a
//!   consumer's job, not this parser's, same "raw data, consumer derives"
//!   split `Bean::names`'s own doc comment describes for the id/name
//!   effective-name rule). No new `DiagCode` is invented for a malformed
//!   `index=` — the spec's edge-case table doesn't call out this shape, and
//!   silently treating it as absent keeps this infallible (rule 4).
//! - `type="..."` — a `ClassRef` naming the argument's declared type.
//!   Empty-value handling mirrors `bean::parse_class_ref`/
//!   `inject_value::parse_type_attr`'s identical policy: invariant #5
//!   (`ClassRef.raw` never empty) is upheld by simply never constructing one
//!   from an empty attribute value.
//! - `name="..."` — the constructor parameter name (Spring's `-parameter-names`
//!   matching, evaluated at runtime — this parser only records the raw
//!   attribute). Optional, unlike `<property name=>`: a `<constructor-arg>`
//!   naming neither `index=` nor `name=` is a legal indexless-positional arg,
//!   so (unlike `Property::name`) this field stays `Option`, never an
//!   empty-string fallback.
//!
//! `<meta key= value=>` children are `ConstructorArg`'s own field (distinct
//! from `Bean::meta`, P6's stub) — read locally here rather than shared with
//! `bean::parse_meta`, same reasoning `property::parse_meta_entry`'s doc
//! comment gives (a different accumulator than the `Vec<MetaEntry>` a
//! `ConstructorArg` carries). The two modules intentionally duplicate this
//! small helper rather than share it, so each unit's leaf stays self
//! contained per the build plan's "leaf fills only its own handler fn" rule.

use crate::dispatch::{find_attr, is_beans_ns, resolve_qname, spanned_attr, NsScope};
use crate::events::{XmlAttr, XmlElement, XmlNode};
use crate::inject_value::{parse_inject_value_child, ref_from_attr, value_lit_from_attr};
use crate::model::{
    BeanCtx, ClassRef, ConstructorArg, DiagCode, Diagnostic, InjectValue, MetaEntry, Spanned,
};

/// Parses one `<constructor-arg>` element — a `<bean>`-body child,
/// dispatched from `bean::dispatch_bean_child`'s `"constructor-arg"` arm —
/// into a [`ConstructorArg`], pushed onto `ctx.constructor_args`.
///
/// `scope`/`depth` follow the exact same conventions
/// `property::parse_property` documents on its own signature: `scope` is
/// the pre-overlay namespace scope in effect for `element`, and `depth` is
/// the enclosing `<bean>`'s own nesting depth, forwarded unchanged into
/// `inject_value::parse_inject_value_child` for this arg's own value-shaped
/// child (if any) — the single [`crate::DEPTH_LIMIT`] choke point bounding
/// bean→constructor-arg→inner-bean recursion. As with `parse_property`, this
/// must **not** be hardcoded to `0`: doing so would reset the guard at every
/// `<constructor-arg>` level and defeat it entirely for recursion reached
/// through a constructor-arg's inner `<bean>`.
pub(crate) fn parse_constructor_arg(
    ctx: &mut BeanCtx,
    diagnostics: &mut Vec<Diagnostic>,
    scope: &NsScope,
    element: &XmlElement,
    depth: u32,
) {
    let index = find_attr(&element.attrs, "index").and_then(|attr| attr.value.value.parse().ok());
    let type_ref = parse_type_ref_attr(&element.attrs);
    let name = find_attr(&element.attrs, "name").map(spanned_attr);

    let value_attr = find_attr(&element.attrs, "value");
    let ref_attr = find_attr(&element.attrs, "ref");

    if value_attr.is_some() && ref_attr.is_some() {
        diagnostics.push(Diagnostic {
            code: DiagCode::ConflictingValueAndRef,
            span: Some(element.span),
            message: "<constructor-arg> specifies both value= and ref=".to_string(),
        });
    }

    // Own overlay — this element's own `xmlns`/`xmlns:*` (rare on
    // `<constructor-arg>` itself, but the same convention every other
    // recursive site in this crate follows) in effect for its children.
    let own_scope = NsScope::from_element(element, Some(scope));

    let mut meta = Vec::new();
    let mut child_value: Option<InjectValue> = None;
    let mut seen_value_child = false;

    for child in &element.children {
        let XmlNode::Element(child_element) = child else {
            // Direct text (whitespace between child elements) has nothing
            // to attach to at this level — same drop
            // `property::parse_property`'s own child loop documents.
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
        // Only the first non-meta child is resolved as this arg's value —
        // same "XSD only ever allows one, further children silently
        // ignored" policy `property::parse_property`'s own loop documents.
        if !seen_value_child {
            seen_value_child = true;
            child_value = parse_inject_value_child(&own_scope, diagnostics, depth, child_element);
        }
    }

    let value = resolve_value(value_attr, ref_attr, child_value, diagnostics, element.span);

    ctx.constructor_args.push(ConstructorArg {
        span: element.span,
        index,
        type_ref,
        name,
        value,
        meta,
    });
}

/// `type="..."` → a `ClassRef`, or `None` when the attribute is absent
/// **or present-but-empty** — same empty-value policy
/// `bean::parse_class_ref`/`inject_value::parse_type_attr` both already
/// apply for their own `ClassRef` sites (invariant #5: `ClassRef.raw` never
/// empty).
fn parse_type_ref_attr(attrs: &[XmlAttr]) -> Option<Spanned<ClassRef>> {
    let attr = find_attr(attrs, "type")?;
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

/// Precedence documented on [`parse_constructor_arg`]'s own doc comment
/// (identical to `property::resolve_value`): `value=` wins, then `ref=`,
/// then the first recognized value-shaped child, then (nothing at all
/// resolved) an opaque `Null` at `span` — never a panic, never a missing
/// `ConstructorArg.value`.
///
/// Ratification (mirrors `property::resolve_value`'s identical note):
/// `value=` (or `ref=`) together with a value-shaped child is intentionally
/// **not** flagged `ConflictingValueAndRef` — that diagnostic only compares
/// the two shorthand attributes against each other, not against a child
/// element. Leniency-over-XSD: this crate records whichever value the
/// precedence chain below picks rather than inventing an opinion about the
/// unusual shape.
fn resolve_value(
    value_attr: Option<&XmlAttr>,
    ref_attr: Option<&XmlAttr>,
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
/// the spec's edge-case table doesn't call out (same policy
/// `property::parse_meta_entry` documents).
fn parse_meta_entry(element: &XmlElement) -> Option<MetaEntry> {
    let key = find_attr(&element.attrs, "key")?;
    let value = find_attr(&element.attrs, "value")?;
    Some(MetaEntry {
        key: spanned_attr(key),
        value: spanned_attr(value),
    })
}
