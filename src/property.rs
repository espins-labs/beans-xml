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
use crate::inject_value::{parse_inject_value_child_boxed, ref_from_attr, value_lit_from_attr};
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
///
/// **Stack-diet note** (I3 P0 Windows `STATUS_STACK_OVERFLOW` fix): this
/// function's own child loop (below) is the choke point for the
/// bean→property→inner-bean mutual recursion — every one of `DEPTH_LIMIT`
/// (256) nested levels adds a copy of this frame to the call stack, so its
/// *own* locals (as opposed to work that's fully done before or after the
/// loop) are what matters for the recursion's total stack cost. The name/
/// conflict-diagnostic computation before the loop, and the
/// `resolve_value`+`Property` assembly after it, are both factored into
/// `#[inline(never)]` helpers — see `bean::parse_bean`'s own doc comment
/// for the full "-O0 reserves every local for the whole function,
/// regardless of control flow" rationale a plain code-motion (without an
/// actual function-call boundary) wouldn't fix. `child_value` is also
/// `Option<Box<InjectValue>>` rather than `Option<InjectValue>` — 8 bytes
/// instead of ~128 — since `InjectValue` is a ~120-byte enum (its variants'
/// own sizes, not just `Inner`'s already-boxed `Bean`, cap the enum's
/// size); unboxed only once, after the loop, in [`finish_property`].
pub(crate) fn parse_property(
    ctx: &mut BeanCtx,
    diagnostics: &mut Vec<Diagnostic>,
    scope: &NsScope,
    element: &XmlElement,
    depth: u32,
) {
    let name = resolve_property_name(element);
    let value_attr = find_attr(&element.attrs, "value");
    let ref_attr = find_attr(&element.attrs, "ref");
    if value_attr.is_some() && ref_attr.is_some() {
        push_property_value_ref_conflict(diagnostics, element.span, &name.value);
    }

    // Own overlay — this element's own `xmlns`/`xmlns:*` (rare on
    // `<property>` itself, but the same convention every other recursive
    // site in this crate follows) in effect for its children.
    let own_scope = NsScope::from_element(element, Some(scope));

    let mut meta = Vec::new();
    let mut child_value: Option<Box<InjectValue>> = None;
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
        // `child_is_meta` — not an inline `NsScope`/qname-tuple check —
        // purely for stack-diet framing: those locals (~120 bytes, `NsScope`
        // is 72, the qname tuple 48) are only needed for this one
        // classification, never for the recursive descent
        // (`parse_inject_value_child_boxed`) just below — this function's
        // own frame is a link in the bean→property→inner-bean recursion
        // (`parse_property`'s own doc comment), so a real function-call
        // boundary confines them to a frame that's popped before that
        // descent begins rather than reserved for this frame's whole
        // `DEPTH_LIMIT`-deep lifetime.
        if child_is_meta(&own_scope, child_element) {
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
            // `_boxed`, not `parse_inject_value_child(..).map(Box::new)` —
            // see `inject_value::parse_inject_value_child_boxed`'s own doc
            // comment for why boxing one level too late still costs a full
            // unboxed `InjectValue` in *this* frame.
            child_value =
                parse_inject_value_child_boxed(&own_scope, diagnostics, depth, child_element);
        }
    }

    finish_property(
        ctx,
        element.span,
        name,
        value_attr,
        ref_attr,
        child_value,
        meta,
        diagnostics,
    );
}

/// Whether `child_element` resolves (under `scope`) to a `<meta>` element
/// in the beans namespace — split out of [`parse_property`]'s loop purely
/// for stack-diet framing, see that loop's own call-site comment.
#[inline(never)]
fn child_is_meta(scope: &NsScope, child_element: &XmlElement) -> bool {
    let child_scope = NsScope::from_element(child_element, Some(scope));
    // Kept as one `qn: (String, String)` binding rather than destructured
    // `let (ns, local) = ..` — stack-diet micro-optimization: at `-O0`,
    // destructuring a tuple rvalue into two separately-named locals
    // generates an extra move/copy into those locals *on top of* the
    // tuple's own temporary (confirmed empirically via MIR dump —
    // `rustc -Z unpretty=mir` — showing both the tuple local and the two
    // destructured locals coexisting), whereas field projection
    // (`qn.0`/`qn.1`) reads directly out of the one temporary that's
    // already there.
    let qn = resolve_qname(&child_element.name, &child_scope);
    qn.1 == "meta" && is_beans_ns(&qn.0)
}

/// `name=` resolution — split out of [`parse_property`] purely for
/// stack-diet framing (see that function's own doc comment). `name=`
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

/// `ConflictingValueAndRef` diagnostic push — split out of [`parse_property`]
/// purely for stack-diet framing (its `format!` call has its own
/// temporaries that would otherwise sit in `parse_property`'s own frame on
/// every recursive call, per that function's doc comment).
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
/// `ctx.properties` — split out of [`parse_property`] purely for stack-diet
/// framing (see that function's own doc comment): this only ever runs
/// *after* `parse_property`'s own child loop (and therefore after any
/// recursion reached through it) has fully returned, so giving it a
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
