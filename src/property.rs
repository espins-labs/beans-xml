//! Unit **U6** — `<property>` (SB-04): wraps [`InjectValue`] (U5a) with
//! `name` + `<meta>` into a [`Property`]. `<property>` is intercepted by
//! [`crate::bean::BeanFrame::step`] before `bean::dispatch_bean_child`'s
//! match ever runs (see that method's own doc comment, and
//! `crate::depth_engine`'s module doc comment for the full recursion-engine
//! picture) — [`crate::bean::BeanFrame::begin_property`] is the real call
//! site, driving this module's [`finish_property`]/[`push_property_value_ref_conflict`]/
//! [`resolve_property_name`] directly rather than through a `parse_property`
//! entry point of its own. Per the build plan's U6 row: "wrap InjectValue
//! with name/index/type/meta", built on U5a, parallel with U7
//! (`ConstructorArg`, SB-05).
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
//! `Bean::meta`, P6's stub) — scanned by [`crate::bean::scan_meta_and_candidate`]
//! (shared with U7's own `<constructor-arg>` scan) rather than by a copy of
//! this loop living here, since that stub pushes into `BeanCtx::meta`, a
//! different accumulator than the `Vec<MetaEntry>` a `Property` carries.

use crate::dispatch::{find_attr, spanned_attr};
use crate::events::XmlElement;
use crate::inject_value::{ref_from_attr, value_lit_from_attr};
use crate::model::{BeanCtx, DiagCode, Diagnostic, InjectValue, MetaEntry, Property, Spanned};

/// `name=` resolution for one `<property>` element — called from
/// [`crate::bean::BeanFrame::begin_property`], the live entry point for
/// `<property>` (see this module's own doc comment for why there is no
/// `parse_property` function here to split this out of anymore). `name=`
/// absent is not among this unit's tested edge cases (Spring's own XSD
/// makes it mandatory) — falling back to an empty spanned string at the
/// element's own span keeps this infallible (rule 4) without inventing a
/// new `DiagCode` for a shape the spec's edge-case table doesn't call out;
/// invariant #5 (never-empty) only binds `BeanRef.raw`/`ClassRef.raw`, not
/// `Property.name`.
#[inline(never)]
pub(crate) fn resolve_property_name(element: &XmlElement) -> Spanned<String> {
    find_attr(&element.attrs, "name")
        .map(spanned_attr)
        .unwrap_or_else(|| Spanned {
            value: String::new(),
            span: element.span,
        })
}

/// `ConflictingValueAndRef` diagnostic push, called from
/// [`crate::bean::BeanFrame::begin_property`] — split out purely for
/// stack-diet framing (its `format!` call has its own temporaries that
/// would otherwise sit in the caller's own frame on every recursive call).
#[inline(never)]
pub(crate) fn push_property_value_ref_conflict(
    diagnostics: &mut Vec<Diagnostic>,
    span: crate::model::ByteSpan,
    name: &str,
) {
    diagnostics.push(Diagnostic {
        code: DiagCode::ConflictingValueAndRef,
        span: Some(span),
        message: format!("<property name=\"{name}\"> specifies both value= and ref="),
    });
}

/// Final `resolve_value` + [`Property`] assembly + push onto
/// `ctx.properties`, called from [`crate::bean::BeanFrame::begin_property`]
/// (synchronously, when there is no value-shaped child to defer) and
/// [`crate::bean::BeanFrame::deliver`] (once a deferred child value has come
/// back off the engine's own stack) — split out purely for stack-diet
/// framing: this only ever runs *after* any recursion reached through this
/// property's own value-shaped child has fully returned, so giving it a
/// separate frame means its locals (notably the assembled `Property`
/// struct literal itself, ~184 bytes) are never reserved during the
/// recursive descent — only transiently, once, while unwinding back
/// through this one level.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub(crate) fn finish_property(
    ctx: &mut BeanCtx,
    span: crate::model::ByteSpan,
    name: Spanned<String>,
    value_attr: Option<&crate::events::XmlAttr>,
    ref_attr: Option<&crate::events::XmlAttr>,
    child_value: Option<Box<InjectValue>>,
    meta: Vec<MetaEntry>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let value = resolve_value(
        value_attr,
        ref_attr,
        child_value.map(|b| *b),
        diagnostics,
        span,
    );
    ctx.properties.push(Property {
        span,
        name,
        value,
        meta,
    });
}

/// Precedence documented on this module's own doc comment: `value=` wins,
/// then `ref=`, then the first recognized value-shaped child, then (nothing
/// at all resolved) an opaque `Null` at `span` — never a panic, never a
/// missing `Property.value`.
///
/// Ratification: `value=` (or `ref=`) together with a value-shaped child
/// (e.g. `<property value="x"><ref bean="y"/></property>`) is intentionally
/// **not** flagged `ConflictingValueAndRef` — that diagnostic only compares
/// the two shorthand *attributes* against each other (see
/// [`crate::bean::BeanFrame::begin_property`]'s own attribute check, made
/// before `resolve_value` is ever called). Leniency-over-XSD: this crate
/// records whichever value the precedence chain below picks and lets the
/// consumer notice the shape is unusual, rather than this parser inventing
/// an opinion about it.
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
