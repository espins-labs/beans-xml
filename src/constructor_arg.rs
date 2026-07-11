//! Unit **U7** — `<constructor-arg>` (SB-05): wraps [`InjectValue`] (U5a)
//! with `index`/`type`/`name` + `<meta>` into a [`ConstructorArg`].
//! `<constructor-arg>` is intercepted by [`crate::bean::BeanFrame::step`]
//! before `bean::dispatch_bean_child`'s match ever runs (see that method's
//! own doc comment, and `crate::depth_engine`'s module doc comment for the
//! full recursion-engine picture) — [`crate::bean::BeanFrame::begin_constructor_arg`]
//! is the real call site, driving this module's [`finish_constructor_arg`]/
//! [`push_constructor_arg_value_ref_conflict`]/[`resolve_constructor_arg_attrs`]
//! directly rather than through a `parse_constructor_arg` entry point of its
//! own — same treatment U6's `<property>` gets (`property.rs`'s own module
//! doc comment). Per the build plan's U7 row: "wrap InjectValue with
//! name/index/type/meta", built on U5a, parallel with U6 (`Property`,
//! SB-04).
//!
//! Value resolution precedence mirrors `property::resolve_value` exactly
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
//! from `Bean::meta`, P6's stub) — scanned by
//! [`crate::bean::scan_meta_and_candidate`] (shared with U6's own
//! `<property>` scan) rather than by a copy of this loop living here, same
//! reasoning `property.rs`'s own module doc comment gives (a different
//! accumulator than the `Vec<MetaEntry>` a `ConstructorArg` carries).

use crate::dispatch::{find_attr, spanned_attr};
use crate::events::{XmlAttr, XmlElement};
use crate::inject_value::{ref_from_attr, value_lit_from_attr};
use crate::model::{
    BeanCtx, ClassRef, ConstructorArg, DiagCode, Diagnostic, InjectValue, MetaEntry, Spanned,
};

/// `index=`/`type=`/`name=` resolution for one `<constructor-arg>` element
/// — called from [`crate::bean::BeanFrame::begin_constructor_arg`], the
/// live entry point for `<constructor-arg>` (see this module's own doc
/// comment for why there is no `parse_constructor_arg` function here to
/// split this out of anymore) — split out purely for stack-diet framing.
#[inline(never)]
pub(crate) fn resolve_constructor_arg_attrs(
    element: &XmlElement,
) -> (
    Option<u32>,
    Option<Spanned<ClassRef>>,
    Option<Spanned<String>>,
) {
    let index = find_attr(&element.attrs, "index").and_then(|attr| attr.value.value.parse().ok());
    let type_ref = parse_type_ref_attr(&element.attrs);
    let name = find_attr(&element.attrs, "name").map(spanned_attr);
    (index, type_ref, name)
}

/// `ConflictingValueAndRef` diagnostic push, called from
/// [`crate::bean::BeanFrame::begin_constructor_arg`] — split out purely for
/// stack-diet framing, same rationale
/// `property::push_property_value_ref_conflict` documents.
#[inline(never)]
pub(crate) fn push_constructor_arg_value_ref_conflict(
    diagnostics: &mut Vec<Diagnostic>,
    span: crate::model::ByteSpan,
) {
    diagnostics.push(Diagnostic {
        code: DiagCode::ConflictingValueAndRef,
        span: Some(span),
        message: "<constructor-arg> specifies both value= and ref=".to_string(),
    });
}

/// Final `resolve_value` + [`ConstructorArg`] assembly + push, called from
/// [`crate::bean::BeanFrame::begin_constructor_arg`] (synchronously, when
/// there is no value-shaped child to defer) and
/// [`crate::bean::BeanFrame::deliver`] (once a deferred child value has come
/// back off the engine's own stack) — split out purely for stack-diet
/// framing, same rationale `property::finish_property` documents (only ever
/// runs after any recursion reached through this arg's own value-shaped
/// child has fully returned).
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub(crate) fn finish_constructor_arg(
    ctx: &mut BeanCtx,
    span: crate::model::ByteSpan,
    index: Option<u32>,
    type_ref: Option<Spanned<ClassRef>>,
    name: Option<Spanned<String>>,
    value_attr: Option<&XmlAttr>,
    ref_attr: Option<&XmlAttr>,
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
    ctx.constructor_args.push(ConstructorArg {
        span,
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

/// Precedence documented on this module's own doc comment (identical to
/// `property::resolve_value`): `value=` wins, then `ref=`,
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
