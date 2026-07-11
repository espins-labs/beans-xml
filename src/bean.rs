//! Unit **U4** — `<bean>` core attributes (SB-02), plus the **frozen
//! bean-child dispatch skeleton** the build plan's "dispatch contract" section
//! requires land before the parallel leaf wave (P2/P6/P8) fans out.
//!
//! Two things this module owns, per the internal build plan's
//! U4 row:
//!
//! 1. **SB-02 itself, fully implemented here** (not stubbed): every core
//!    `<bean>` attribute (`id`/`name`/`class`/`parent`/`scope`/`abstract`/
//!    `lazy-init`/`autowire`/`autowire-candidate`/`primary`/`depends-on`/
//!    `factory-bean`/`factory-method`/`init-method`/`destroy-method`), the
//!    effective-name rule's supporting fields (`id`/`names`, see
//!    [`crate::model::Bean::names`]'s own doc comment for the rule itself
//!    — it is documentation, not a stored field; deriving it is a
//!    consumer's job), the legacy DTD `singleton="true"/"false"`
//!    normalization into `scope`, `DuplicateBeanId` (applied by the
//!    caller, `dispatch::dispatch_root_child`'s `"bean"` arm — see that
//!    arm's own doc comment for why it, not this function, owns that
//!    check), and `BeanWithoutClassOrParent` (with the `abstract` template
//!    exemption).
//! 2. **The bean-child dispatch match** in [`dispatch_bean_child`]: element
//!    name + resolved namespace → one of five per-element handler-fn stubs
//!    (`parse_qualifier`/`parse_meta`/`parse_decorator`/
//!    `parse_lookup_method`/`parse_replaced_method`), every one of them an
//!    intentional no-op — filling a stub's body is leaf-unit work (P6/P6/
//!    P7-gated-P6/P8/P8 respectively), and per the build plan's stated
//!    contract, a leaf touches **only its own handler function**, never
//!    this match. Plus **the prefixed-attribute hook** in
//!    [`normalize_pc_attr`] (**P2, SB-08, fully implemented here**):
//!    normalizes `p:`/`c:`-namespace attributes into `Property`/
//!    `ConstructorArg` entries, joining the same `Vec<Property>`/
//!    `Vec<ConstructorArg>` the `"property"`/`"constructor-arg"` element
//!    arms above populate.
//!
//! `<property>`/`<constructor-arg>` are deliberately **not** among the five
//! stubs above, and are not arms of [`dispatch_bean_child`]'s match at all:
//! wrapping `InjectValue` into a `Property`/`ConstructorArg` is U6/U7's
//! contract, which build on **U5a**, but both element names are intercepted
//! by [`BeanFrame::step`] *before* this match ever runs (see that method's
//! own doc comment, and `crate::depth_engine`'s module doc comment for the
//! full recursion-engine picture) — U6's `property::finish_property`/
//! U7's `constructor_arg::finish_constructor_arg` are the real call sites,
//! reached through [`BeanFrame::begin_property`]/[`BeanFrame::begin_constructor_arg`].
//!
//! `pub(crate)` — like `dispatch`, not part of the published API surface.

use crate::constructor_arg::{
    finish_constructor_arg, push_constructor_arg_value_ref_conflict, resolve_constructor_arg_attrs,
};
use crate::dispatch::{
    element_text, find_attr, find_bool_attr, is_beans_ns, resolve_qname, spanned_attr, NsScope,
};
use crate::events::{XmlAttr, XmlElement, XmlNode};
use crate::inject_value::{ref_from_attr, value_lit_from_attr};
use crate::model::{
    AttrPair, Bean, BeanCtx, BeanRef, ByteSpan, ClassRef, ConstructorArg, DiagCode, Diagnostic,
    InjectValue, LookupMethod, MetaEntry, Property, Qualifier, RefKind, ReplacedMethod, Spanned,
};
use crate::property::{finish_property, push_property_value_ref_conflict, resolve_property_name};

// ---------------------------------------------------------------------
// SB-02: <bean> core attributes.
// ---------------------------------------------------------------------

/// Parses one `<bean>` element — a top-level `<beans>` child, or (once
/// U5a's `InjectValue::Inner` lands) an anonymous inner bean re-entering
/// this exact same function (build plan "recursion unification": never reimplemented
/// a second time) — into a [`Bean`].
///
/// `scope` is the namespace scope in effect for `element` *before*
/// overlaying whatever `xmlns`/`xmlns:*` declarations `element` itself
/// carries (the same "caller passes its own pre-overlay scope, callee
/// overlays itself if it needs to recurse" convention
/// `dispatch::dispatch_root_child`'s handler stubs already follow) — this
/// function computes its own overlay once, for both the prefixed-attribute
/// hook and its children.
///
/// `DuplicateBeanId` is deliberately **not** checked here — see
/// `dispatch::dispatch_root_child`'s `"bean"` arm, which is the only call
/// site with visibility into a bean's siblings within the same `<beans>`
/// block.
///
/// `depth` is this bean's own nesting depth (0 for every top-level
/// `<beans>`-child bean, `dispatch::dispatch_root_child`'s own call site;
/// N+1 for an inner `<bean>` reached through a property/constructor-arg
/// value at depth N). It is threaded, unchanged, into [`BeanFrame`] so
/// [`BeanFrame::begin_property`]/[`BeanFrame::begin_constructor_arg`] can
/// pass it on to `inject_value::begin_resolve_value`, the single
/// [`crate::DEPTH_LIMIT`] choke point that bounds the
/// bean→property→inner-bean→property mutual recursion (see that function's
/// own doc comment) — this parameter is what actually closes that loop;
/// without it every `<property>` reset the depth back to 0 regardless of
/// how deep its enclosing bean already was.
///
/// **Stack-diet note** (I3 P0 Windows `STATUS_STACK_OVERFLOW` fix): `Bean`/
/// `BeanCtx` are ~568 bytes each (many `Vec`/`Option<Spanned<_>>` fields) —
/// large enough that, held by value in every frame of the
/// bean→property→inner-bean mutual recursion, `DEPTH_LIMIT` (256) levels
/// blew a 256 KiB (or even a Windows-default 1 MiB) thread stack well
/// before the guard ever fired. Two changes here specifically target that,
/// verified empirically against `tests/scratch_stack_probe.rs` (see this
/// function's own doc comment history / the fix's commit message for
/// before/after numbers, not duplicated here to avoid rot):
/// 1. `ctx` is heap-allocated (`Box<BeanCtx>`) instead of living in this
///    frame — the 568-byte struct moves off the call stack entirely;
///    `&mut BeanCtx` still threads through `dispatch_bean_child` exactly as
///    before (`Box` derefs transparently at call boundaries).
/// 2. This function returns `Box<Bean>`, not `Bean` — every deeply-recursive
///    caller in the cycle ([`crate::inject_value::parse_inner_bean`]) needs
///    a `Box<Bean>` anyway (`InjectValue::Inner(Box<Bean>)`), so returning
///    one directly avoids constructing an unboxed `Bean` in the caller's
///    own frame just to immediately re-box it. The one call site that
///    genuinely wants an owned `Bean` (`dispatch::dispatch_root_child`'s
///    `"bean"` arm, `ctx.beans: Vec<Bean>`) just dereferences once, at
///    depth 0 — not part of the recursive chain, so that single unavoidable
///    unboxed copy costs nothing per nesting level.
///
/// All attribute population (core `<bean>` attributes, the `p:`/`c:`
/// prefixed-attribute hook) is factored into `#[inline(never)]` helpers
/// below ([`populate_bean_core_attrs`]/[`populate_bean_pc_attrs`]) — not
/// for the `#[inline(never)]` hint itself (this crate has no perf
/// requirement calling for it), but because a helper's own locals/MIR
/// temporaries live in *that helper's own stack frame*, fully popped once
/// it returns — **before** this function's own recursive descent into
/// `dispatch_bean_child`/deeper `<bean>`s ever begins. Left inline, an
/// unoptimized (`-O0`, i.e. every `cargo test`/debug build) frame keeps a
/// stack slot reserved for every local declared anywhere in the function
/// for that whole call's lifetime, regardless of whether it's still
/// "logically" needed — so this attribute-parsing code, if left inline,
/// would otherwise sit on the stack at every one of `DEPTH_LIMIT` nested
/// levels simultaneously, not just once per level.
///
/// **I3 P0 stack-diet fallback**: the recursive descent this doc comment's
/// own numbered list above still describes at the level of *this bean's
/// own attributes* is unchanged — but the mutual bean→property→inner-bean
/// recursion that used to happen via a real Rust call back into this exact
/// function (through `dispatch_bean_child`'s `"property"`/`"constructor-arg"`
/// arms) is gone. Frame-dieting alone (the two numbered points above)
/// halved per-level stack cost but could not reach a 256 KiB thread budget
/// at `DEPTH_LIMIT` levels for every hostile shape (measured via
/// `tests/scratch_stack_probe.rs`; see `crate::depth_engine`'s own module
/// doc comment for the full before/after). This function is now a thin
/// wrapper: it pushes one [`BeanFrame`] onto [`crate::depth_engine::run`]
/// and lets that engine drive the whole subtree (this bean, every inner
/// `<bean>`/collection reachable through it, however deep) on the heap
/// instead of the real call stack. See [`BeanFrame`]'s own doc comment for
/// how the recursion itself now works.
pub(crate) fn parse_bean(
    scope: &NsScope,
    element: &XmlElement,
    diagnostics: &mut Vec<Diagnostic>,
    depth: u32,
) -> Box<Bean> {
    let frame = BeanFrame::new(scope, element, diagnostics, depth);
    let stack = vec![crate::depth_engine::Frame::Bean(frame)];
    match crate::depth_engine::run(stack, diagnostics) {
        crate::depth_engine::Completed::Bean(bean) => bean,
        crate::depth_engine::Completed::Value(_) => {
            unreachable!("a top-level BeanFrame always finishes into Completed::Bean")
        }
    }
}

// ---------------------------------------------------------------------
// I3 P0 stack-diet fallback: explicit-stack (heap worklist) iteration for
// the bean<->property/constructor-arg<->inner-bean recursion — see
// `crate::depth_engine`'s own module doc comment for the full picture.
//
// Every non-recursive per-`<bean>` concern (core/`p:`/`c:` attribute
// parsing, `<qualifier>`/`<meta>`/`<lookup-method>`/`<replaced-method>`/
// decorator handling) is unchanged and still reused directly below — none
// of those ever call back into `parse_bean`/`parse_collection_value`, so
// they were never part of the unbounded recursion and don't need to move
// onto the explicit stack; `dispatch_bean_child`'s own frozen match is
// still the single dispatch point for them. Only `<property>`/
// `<constructor-arg>`, whose own value-shaped child can itself be another
// `<bean>` or a collection, are intercepted *before* `dispatch_bean_child`
// (they are consequently not arms of that match at all — see
// `dispatch_bean_child`'s own doc comment) and rerouted through
// `crate::inject_value::begin_resolve_value` — the single choke point that
// decides "resolve immediately" vs. "defer onto the explicit stack".
// ---------------------------------------------------------------------

/// One in-progress `parse_bean` call, suspended on the heap instead of the
/// real call stack — see this section's own doc comment. `waiting`, when
/// `Some`, is the `<property>`/`<constructor-arg>` at `children[idx - 1]`
/// whose own value-shaped child has been pushed onto the engine's stack and
/// hasn't come back yet; this bean's own children loop only resumes past it
/// once [`Self::deliver`] hands the resolved value back.
pub(crate) struct BeanFrame<'a> {
    ctx: Box<BeanCtx>,
    own_scope: NsScope,
    children: &'a [XmlNode],
    idx: usize,
    depth: u32,
    waiting: Option<PendingBeanValue<'a>>,
}

/// A `<property>`/`<constructor-arg>` whose own value-shaped child has been
/// deferred onto the explicit stack — everything `finish_property`/
/// `finish_constructor_arg` need *except* the resolved child value itself
/// (which arrives later via [`BeanFrame::deliver`]), mirroring exactly what
/// `property::parse_property`/`constructor_arg::parse_constructor_arg` used
/// to hold in their own (now-unwound) stack frame across the recursive
/// call.
enum PendingBeanValue<'a> {
    Property {
        span: ByteSpan,
        name: Spanned<String>,
        value_attr: Option<&'a XmlAttr>,
        ref_attr: Option<&'a XmlAttr>,
        meta: Vec<MetaEntry>,
    },
    ConstructorArg {
        span: ByteSpan,
        index: Option<u32>,
        type_ref: Option<Spanned<ClassRef>>,
        name: Option<Spanned<String>>,
        value_attr: Option<&'a XmlAttr>,
        ref_attr: Option<&'a XmlAttr>,
        meta: Vec<MetaEntry>,
    },
}

impl<'a> BeanFrame<'a> {
    /// Starts a new frame for `element` — the exact prologue `parse_bean`
    /// ran inline before its own children loop (core attrs, own overlay,
    /// `p:`/`c:` attrs), unchanged.
    pub(crate) fn new(
        scope: &NsScope,
        element: &'a XmlElement,
        diagnostics: &mut Vec<Diagnostic>,
        depth: u32,
    ) -> Self {
        let mut ctx = new_bean_ctx(element.span);
        populate_bean_core_attrs(&mut ctx, element, diagnostics);
        let own_scope = NsScope::from_element(element, Some(scope));
        populate_bean_pc_attrs(&mut ctx, element, diagnostics, &own_scope);
        BeanFrame {
            ctx,
            own_scope,
            children: &element.children,
            idx: 0,
            depth,
            waiting: None,
        }
    }

    /// Advances this frame by one step: either makes local progress
    /// (`Advance::Continue`, implicitly, by looping again below), finishes
    /// (`Advance::Finished`), or defers a property/constructor-arg's own
    /// value-shaped child onto the stack (`Advance::Push`). Never called
    /// while `self.waiting.is_some()` — that case only ever resumes via
    /// [`Self::deliver`].
    pub(crate) fn step(
        &mut self,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> crate::depth_engine::Advance<'a> {
        use crate::depth_engine::Advance;
        debug_assert!(self.waiting.is_none());
        loop {
            let Some(child) = self.children.get(self.idx) else {
                return Advance::Finished;
            };
            self.idx += 1;
            let XmlNode::Element(child_element) = child else {
                continue;
            };
            let child_scope = NsScope::from_element(child_element, Some(&self.own_scope));
            let qn = resolve_qname(&child_element.name, &child_scope);
            if is_beans_ns(&qn.0) && qn.1 == "property" {
                if let Some(advance) = self.begin_property(child_element, diagnostics) {
                    return advance;
                }
                continue;
            }
            if is_beans_ns(&qn.0) && qn.1 == "constructor-arg" {
                if let Some(advance) = self.begin_constructor_arg(child_element, diagnostics) {
                    return advance;
                }
                continue;
            }
            // Every other bean-child shape is bounded, non-recursive work
            // — reuse the frozen dispatch match unchanged.
            dispatch_bean_child(&mut self.ctx, diagnostics, &self.own_scope, child_element);
        }
    }

    /// Phase A (scan `element`'s own children for `<meta>` + the first
    /// non-meta value-shaped candidate — see [`scan_meta_and_candidate`]'s
    /// own doc comment for why this split is diagnostic-order-safe) plus
    /// Phase B's *attempt*. `None` when the whole property finished
    /// synchronously (already pushed onto `self.ctx.properties` — the
    /// caller's loop should continue); `Some(Advance::Push(..))` when it
    /// needs to suspend.
    fn begin_property(
        &mut self,
        element: &'a XmlElement,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Option<crate::depth_engine::Advance<'a>> {
        use crate::depth_engine::Advance;
        use crate::inject_value::ValueStep;

        let name = resolve_property_name(element);
        let value_attr = find_attr(&element.attrs, "value");
        let ref_attr = find_attr(&element.attrs, "ref");
        if value_attr.is_some() && ref_attr.is_some() {
            push_property_value_ref_conflict(diagnostics, element.span, &name.value);
        }
        let own_scope = NsScope::from_element(element, Some(&self.own_scope));
        let (meta, candidate) = scan_meta_and_candidate(&own_scope, element);

        let Some(candidate_element) = candidate else {
            finish_property(
                &mut self.ctx,
                element.span,
                name,
                value_attr,
                ref_attr,
                None,
                meta,
                diagnostics,
            );
            return None;
        };
        match crate::inject_value::begin_resolve_value(
            &own_scope,
            diagnostics,
            self.depth,
            candidate_element,
        ) {
            ValueStep::Resolved(value) => {
                finish_property(
                    &mut self.ctx,
                    element.span,
                    name,
                    value_attr,
                    ref_attr,
                    value,
                    meta,
                    diagnostics,
                );
                None
            }
            ValueStep::Deferred(frame) => {
                self.waiting = Some(PendingBeanValue::Property {
                    span: element.span,
                    name,
                    value_attr,
                    ref_attr,
                    meta,
                });
                Some(Advance::Push(frame))
            }
        }
    }

    /// Same shape as [`Self::begin_property`], for `<constructor-arg>`.
    fn begin_constructor_arg(
        &mut self,
        element: &'a XmlElement,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Option<crate::depth_engine::Advance<'a>> {
        use crate::depth_engine::Advance;
        use crate::inject_value::ValueStep;

        let (index, type_ref, name) = resolve_constructor_arg_attrs(element);
        let value_attr = find_attr(&element.attrs, "value");
        let ref_attr = find_attr(&element.attrs, "ref");
        if value_attr.is_some() && ref_attr.is_some() {
            push_constructor_arg_value_ref_conflict(diagnostics, element.span);
        }
        let own_scope = NsScope::from_element(element, Some(&self.own_scope));
        let (meta, candidate) = scan_meta_and_candidate(&own_scope, element);

        let Some(candidate_element) = candidate else {
            finish_constructor_arg(
                &mut self.ctx,
                element.span,
                index,
                type_ref,
                name,
                value_attr,
                ref_attr,
                None,
                meta,
                diagnostics,
            );
            return None;
        };
        match crate::inject_value::begin_resolve_value(
            &own_scope,
            diagnostics,
            self.depth,
            candidate_element,
        ) {
            ValueStep::Resolved(value) => {
                finish_constructor_arg(
                    &mut self.ctx,
                    element.span,
                    index,
                    type_ref,
                    name,
                    value_attr,
                    ref_attr,
                    value,
                    meta,
                    diagnostics,
                );
                None
            }
            ValueStep::Deferred(frame) => {
                self.waiting = Some(PendingBeanValue::ConstructorArg {
                    span: element.span,
                    index,
                    type_ref,
                    name,
                    value_attr,
                    ref_attr,
                    meta,
                });
                Some(Advance::Push(frame))
            }
        }
    }

    /// Resumes a suspended property/constructor-arg once its own
    /// value-shaped child has finished resolving — finishes it immediately
    /// (pushes onto `self.ctx.properties`/`constructor_args`) and hands
    /// control back to [`Self::step`] to continue this bean's own children
    /// loop.
    pub(crate) fn deliver(
        &mut self,
        value: Box<InjectValue>,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> crate::depth_engine::Advance<'a> {
        match self
            .waiting
            .take()
            .expect("BeanFrame delivered without a pending property/constructor-arg")
        {
            PendingBeanValue::Property {
                span,
                name,
                value_attr,
                ref_attr,
                meta,
            } => {
                finish_property(
                    &mut self.ctx,
                    span,
                    name,
                    value_attr,
                    ref_attr,
                    Some(value),
                    meta,
                    diagnostics,
                );
            }
            PendingBeanValue::ConstructorArg {
                span,
                index,
                type_ref,
                name,
                value_attr,
                ref_attr,
                meta,
            } => {
                finish_constructor_arg(
                    &mut self.ctx,
                    span,
                    index,
                    type_ref,
                    name,
                    value_attr,
                    ref_attr,
                    Some(value),
                    meta,
                    diagnostics,
                );
            }
        }
        crate::depth_engine::Advance::Continue
    }

    /// Consumes this finished frame into the assembled [`Bean`] — only ever
    /// called once `step`/`deliver` has returned `Advance::Finished` for
    /// it.
    pub(crate) fn finish(self) -> Box<Bean> {
        finish_bean(self.ctx)
    }
}

/// Phase A shared by [`BeanFrame::begin_property`]/`begin_constructor_arg`:
/// scans `element`'s own children once, collecting every `<meta>` entry
/// (unconditional — same as the original interleaved loop) and identifying
/// the first non-meta child element, if any, as the value candidate —
/// resolution of that candidate is Phase B, deliberately not done here.
///
/// This split is diagnostic-order-safe because meta collection
/// (`parse_meta_entry`) never itself pushes a diagnostic, and "which child
/// is the value candidate" never depends on any resolution *outcome*
/// (unlike `collection::EntryScan`'s own `<key>` handling, whose retry-on-
/// `None` rule genuinely needs a resumable scan, not a two-phase split —
/// see that type's own doc comment) — every diagnostic either loop could
/// ever produce comes from resolving that one candidate, wherever in
/// `element`'s own children it sits, so scanning fully first and resolving
/// second produces the exact same diagnostic sequence as the original
/// single interleaved loop.
fn scan_meta_and_candidate<'a>(
    own_scope: &NsScope,
    element: &'a XmlElement,
) -> (Vec<MetaEntry>, Option<&'a XmlElement>) {
    let mut meta = Vec::new();
    let mut candidate = None;
    for child in &element.children {
        let XmlNode::Element(child_element) = child else {
            continue;
        };
        let child_scope = NsScope::from_element(child_element, Some(own_scope));
        let qn = resolve_qname(&child_element.name, &child_scope);
        if qn.1 == "meta" && is_beans_ns(&qn.0) {
            if let Some(entry) = parse_meta_entry(child_element) {
                meta.push(entry);
            }
            continue;
        }
        if candidate.is_none() {
            candidate = Some(child_element);
        }
    }
    (meta, candidate)
}

/// `Box::new(BeanCtx::default())` plus the one field ([`ByteSpan`]) known
/// up front — split out of [`parse_bean`] purely for stack-diet framing.
/// Not just code motion: `-O0` codegen doesn't fuse `Box::new(f())` into
/// "allocate, then construct directly into the allocation" (confirmed
/// empirically via `otool -tv` disassembly of a debug build — a real
/// `BeanCtx`-sized `memcpy` from a stack temp into the fresh allocation),
/// so this constructor's ~568-byte temporary needs its *own* frame — one
/// that's fully popped before `parse_bean`'s own recursive descent begins
/// — rather than `parse_bean`'s.
#[inline(never)]
fn new_bean_ctx(span: ByteSpan) -> Box<BeanCtx> {
    let mut ctx = Box::new(BeanCtx::default());
    ctx.span = span;
    ctx
}

/// `ctx.into_bean()` plus the final re-box — split out of [`parse_bean`]
/// purely for stack-diet framing, same rationale [`new_bean_ctx`] gives:
/// this assembly step's ~568-byte `Bean` temporary (same `-O0`
/// `Box::new(f())` non-fusion) would otherwise sit in `parse_bean`'s own
/// frame for the *entire* call, including while its child loop is still
/// recursing arbitrarily deep — even though this code only actually runs
/// once that recursion has fully returned. A real function-call boundary,
/// not just code motion within one function, is what makes the difference:
/// this helper's frame doesn't exist at all until it's actually invoked.
#[inline(never)]
fn finish_bean(ctx: Box<BeanCtx>) -> Box<Bean> {
    Box::new(ctx.into_bean())
}

/// Every core `<bean>` attribute (`id`/`name`/`class`/`parent`/`scope`/
/// `abstract`/`lazy-init`/`autowire`/`autowire-candidate`/`primary`/
/// `depends-on`/`factory-bean`/`factory-method`/`init-method`/
/// `destroy-method`) plus the `BeanWithoutClassOrParent` check — split out
/// of [`parse_bean`] purely for stack-diet framing (see that function's own
/// doc comment); no behavior change from when this was inline there.
#[inline(never)]
fn populate_bean_core_attrs(
    ctx: &mut BeanCtx,
    element: &XmlElement,
    diagnostics: &mut Vec<Diagnostic>,
) {
    ctx.id = find_attr(&element.attrs, "id").map(spanned_attr);
    ctx.names = find_attr(&element.attrs, "name")
        .map(|attr| split_name_tokens(&attr.value.value, attr.value.span))
        .unwrap_or_default();
    ctx.class = parse_class_ref(&element.attrs);
    ctx.parent = find_attr(&element.attrs, "parent")
        .and_then(|a| spanned_bean_ref(a, RefKind::Bean, diagnostics));
    ctx.scope = resolve_scope(&element.attrs);
    ctx.abstract_ = find_bool_attr(&element.attrs, "abstract").unwrap_or(false);
    ctx.lazy_init = find_bool_attr(&element.attrs, "lazy-init");
    ctx.primary = find_bool_attr(&element.attrs, "primary").unwrap_or(false);
    ctx.autowire = find_attr(&element.attrs, "autowire").map(spanned_attr);
    ctx.autowire_candidate = find_bool_attr(&element.attrs, "autowire-candidate");
    ctx.depends_on = parse_depends_on(&element.attrs);
    ctx.factory_bean = find_attr(&element.attrs, "factory-bean")
        .and_then(|a| spanned_bean_ref(a, RefKind::Bean, diagnostics));
    ctx.factory_method = find_attr(&element.attrs, "factory-method").map(spanned_attr);
    ctx.init_method = find_attr(&element.attrs, "init-method").map(spanned_attr);
    ctx.destroy_method = find_attr(&element.attrs, "destroy-method").map(spanned_attr);

    if ctx.class.is_none() && ctx.parent.is_none() && ctx.factory_bean.is_none() && !ctx.abstract_ {
        diagnostics.push(Diagnostic {
            code: DiagCode::BeanWithoutClassOrParent,
            span: Some(element.span),
            message: "bean has none of class/parent/factory-bean and is not abstract".to_string(),
        });
    }
}

/// The `p:`/`c:`-namespace prefixed-attribute hook (build plan U4 (b)):
/// every attribute passes through [`normalize_pc_attr`] so it can decide,
/// per attribute, whether it belongs to `Property`/`ConstructorArg`. Split
/// out of [`parse_bean`] for the same stack-diet reason
/// [`populate_bean_core_attrs`]'s own doc comment gives.
#[inline(never)]
fn populate_bean_pc_attrs(
    ctx: &mut BeanCtx,
    element: &XmlElement,
    diagnostics: &mut Vec<Diagnostic>,
    own_scope: &NsScope,
) {
    for attr in &element.attrs {
        normalize_pc_attr(ctx, diagnostics, own_scope, attr);
    }
}

/// `class="..."` → a `ClassRef`, or `None` when the attribute is absent
/// **or present-but-empty** — invariant #5 (`ClassRef.raw` never empty) is
/// upheld by simply never constructing one from an empty value, the same
/// way an absent attribute never constructs one; either way, the effect on
/// `BeanWithoutClassOrParent` below is identical (no usable class).
fn parse_class_ref(attrs: &[XmlAttr]) -> Option<Spanned<ClassRef>> {
    let attr = find_attr(attrs, "class")?;
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

/// Builds a `Spanned<BeanRef>` from a single-valued reference attribute
/// (`parent=`/`factory-bean=`). An attribute present but empty violates
/// invariant #5 (`BeanRef.raw` never empty) if turned into a `BeanRef`
/// directly — reported as `RefWithoutTarget` instead (same treatment
/// `LookupMethod`'s own doc comment describes for a missing `bean=`: the
/// reference-shaped attribute carries no usable target, edge not emitted),
/// with `None` returned so the field reads the same as if the attribute
/// had never been written at all.
fn spanned_bean_ref(
    attr: &XmlAttr,
    kind: RefKind,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Spanned<BeanRef>> {
    if attr.value.value.is_empty() {
        diagnostics.push(Diagnostic {
            code: DiagCode::RefWithoutTarget,
            span: Some(attr.value.span),
            message: format!("'{}' attribute is present but empty", attr.name),
        });
        return None;
    }
    Some(Spanned {
        value: BeanRef {
            raw: attr.value.value.clone(),
            kind,
        },
        span: attr.value.span,
    })
}

/// `depends-on="a, b c"` → one `BeanRef` (kind `Bean`, same container the
/// attribute itself resolves within — spec's "settled decisions": `parent=`/
/// `depends-on=`/`factory-bean=` are all `RefKind::Bean`) per token.
/// [`split_name_tokens`] already skips runs of separators, so a malformed
/// `"a,,b"` or `" , "` never manufactures an empty token — invariant #5
/// holds by construction, with no diagnostic needed for that shape
/// (mirrors `StringUtils.tokenizeToStringArray`'s own trim-empty-tokens
/// behavior, which is what Spring itself applies to this attribute).
fn parse_depends_on(attrs: &[XmlAttr]) -> Vec<Spanned<BeanRef>> {
    let Some(attr) = find_attr(attrs, "depends-on") else {
        return Vec::new();
    };
    split_name_tokens(&attr.value.value, attr.value.span)
        .into_iter()
        .map(|token| Spanned {
            value: BeanRef {
                raw: token.value,
                kind: RefKind::Bean,
            },
            span: token.span,
        })
        .collect()
}

/// `scope="..."` when present; otherwise the legacy DTD
/// `singleton="true"/"false"` normalized into the same field
/// (`"singleton"`/`"prototype"`) — spec's "settled decisions": "DTD
/// singleton="true|false" also normalizes into this field". A modern `scope=` attribute
/// always wins over a legacy `singleton=` one when (unusually) both are
/// present on the same element, since `scope` is the newer, more specific
/// attribute — this crate never sees both on real Spring config (the two
/// are mutually exclusive by schema generation, DTD vs. XSD), so this
/// ordering is a defensive default rather than an observed real case.
/// `singleton`'s own value follows the same "recognized true/false, else
/// no opinion" policy as every other boolean attribute
/// ([`find_bool_attr`]'s doc comment).
///
/// Ratification: `scope=""` (present but empty) intentionally resolves to
/// `Some(Spanned { value: String::new(), .. })`, not `None` — `spanned_attr`
/// is called unconditionally on a present `scope=` attribute with no
/// empty-value guard. This is deliberate, not an oversight: `scope` is a
/// raw string field that preserves an empty attribute verbatim, unlike
/// `class`/ref-shaped fields (`ClassRef.raw`/`BeanRef.raw`), which invariant
/// #5 requires to never be constructed empty.
fn resolve_scope(attrs: &[XmlAttr]) -> Option<Spanned<String>> {
    if let Some(scope_attr) = find_attr(attrs, "scope") {
        return Some(spanned_attr(scope_attr));
    }
    let singleton_attr = find_attr(attrs, "singleton")?;
    match singleton_attr.value.value.as_str() {
        "true" => Some(Spanned {
            value: "singleton".to_string(),
            span: singleton_attr.value.span,
        }),
        "false" => Some(Spanned {
            value: "prototype".to_string(),
            span: singleton_attr.value.span,
        }),
        _ => None,
    }
}

/// Splits `text` on `,`/`;`/ASCII-whitespace runs into tokens, each
/// carrying its own absolute span (`span.start` anchors the token offsets
/// computed against `text`, mirroring `events::scan_attrs_for_tag`'s own
/// reasoning for why byte-level scanning on single-byte ASCII separators
/// never lands mid-character: none of `,`/`;`/ASCII-whitespace ever occurs
/// as a continuation or lead byte of a multi-byte UTF-8 sequence, so this
/// is safe on non-ASCII tokens like Korean bean names/ids). Used for both
/// `name="a, b c"` alias tokens and `depends-on="a, b c"` targets — the
/// same delimiter set the spec's edge-case table calls out for `name`
/// (comma/semicolon/whitespace-separated) applies to `depends-on` too
/// (Spring's own `StringUtils` tokenizer uses the identical delimiter set
/// for both attributes). Runs of separators collapse (no empty tokens),
/// so a leading/trailing/doubled separator never produces one either.
///
/// `pub(crate)`: also reused by `dispatch::parse_component_scan` (P4,
/// SB-10) for `base-package="a,b; c"` — Spring's own
/// `ComponentScanBeanDefinitionParser` tokenizes `base-package` against the
/// identical `",; \t\n"` delimiter set, so no bespoke splitter is needed
/// there.
pub(crate) fn split_name_tokens(text: &str, span: ByteSpan) -> Vec<Spanned<String>> {
    fn is_sep(b: u8) -> bool {
        b == b',' || b == b';' || b.is_ascii_whitespace()
    }

    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && is_sep(bytes[i]) {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        while i < bytes.len() && !is_sep(bytes[i]) {
            i += 1;
        }
        let end = i;
        tokens.push(Spanned {
            value: text[start..end].to_string(),
            span: ByteSpan {
                start: span.start + start as u32,
                end: span.start + end as u32,
            },
        });
    }
    tokens
}

// ---------------------------------------------------------------------
// The frozen bean-child dispatch match (build plan "dispatch contract").
// ---------------------------------------------------------------------

/// One `<bean>`-body child element → its handler. **Frozen structure**,
/// same shape as `dispatch::dispatch_root_child`: the beans-namespace
/// element names this unit already owns or reserves are enumerated
/// explicitly, and the decorator catch-all (`parse_decorator`) is the
/// **last** arm, so anything not explicitly claimed above it — every
/// namespace other than `beans` itself (`aop:scoped-proxy`, ...) — falls
/// through to it. A leaf unit (P2/P6/P8) fills exactly one handler
/// function's body; none of them ever needs to touch this match.
///
/// `<property>`/`<constructor-arg>` are **not** arms of this match: both
/// are intercepted by [`BeanFrame::step`] before this function is ever
/// called for them (see that method's own doc comment), so this match only
/// ever sees every *other* bean-child shape — none of which need `depth`,
/// which is consequently not one of this function's own parameters either
/// (unlike [`BeanFrame`], which still threads it through to
/// [`BeanFrame::begin_property`]/[`BeanFrame::begin_constructor_arg`]).
fn dispatch_bean_child(
    ctx: &mut BeanCtx,
    diagnostics: &mut Vec<Diagnostic>,
    scope: &NsScope,
    child: &XmlElement,
) {
    let child_scope = NsScope::from_element(child, Some(scope));
    // Kept as one `qn: (String, String)` binding rather than destructured
    // `let (ns, local) = ..` — stack-diet micro-optimization, see
    // `dispatch::dispatch_root_child`'s own matching comment for the
    // empirical (MIR-dump-confirmed) rationale.
    let qn = resolve_qname(&child.name, &child_scope);
    match qn.1.as_str() {
        "description" if is_beans_ns(&qn.0) => {
            // SB-02 core (this unit), not a leaf stub — same field, same
            // reading, as `BeansFile::description`.
            ctx.description = Some(element_text(child));
        }
        "meta" if is_beans_ns(&qn.0) => parse_meta(ctx, diagnostics, scope, child),
        "qualifier" if is_beans_ns(&qn.0) => parse_qualifier(ctx, diagnostics, scope, child),
        "lookup-method" if is_beans_ns(&qn.0) => {
            parse_lookup_method(ctx, diagnostics, scope, child)
        }
        "replaced-method" if is_beans_ns(&qn.0) => {
            parse_replaced_method(ctx, diagnostics, scope, child)
        }
        // An element inside the first-class `beans` namespace itself that
        // isn't one of the recognized/reserved names above — `UnknownElement`,
        // same policy `dispatch::dispatch_root_child`'s own matching arm
        // documents for the `<beans>`-body level.
        _ if is_beans_ns(&qn.0) => push_unknown_bean_child(diagnostics, child),
        // Decorator catch-all — pinned LAST, per the dispatch contract:
        // every other namespace (`aop:scoped-proxy`, ...) lands here.
        _ => parse_decorator(ctx, diagnostics, scope, child),
    }
}

/// `UnknownElement` diagnostic push for an unrecognized element inside a
/// `<bean>` — split out of [`dispatch_bean_child`]'s match purely for
/// stack-diet framing (its `format!` call has its own temporaries that
/// would otherwise sit in `dispatch_bean_child`'s own frame on every
/// recursive call — this function is on the bean→property→inner-bean
/// mutual recursion's own hot chain, see `bean::parse_bean`'s doc comment).
#[inline(never)]
fn push_unknown_bean_child(diagnostics: &mut Vec<Diagnostic>, child: &XmlElement) {
    diagnostics.push(Diagnostic {
        code: DiagCode::UnknownElement,
        span: Some(child.span),
        message: format!("unrecognized element <{}> inside a <bean>", child.name),
    });
}

// ---------------------------------------------------------------------
// Per-element handler fns + the prefixed-attribute hook (build plan
// "dispatch contract").
//
// Each of these started life as an intentional no-op stub; filling in the
// real body is the named leaf unit's entire job (all five — P6/P6/
// P7-gated-P6/P8/P8 — are now filled), and per the dispatch contract, doing
// so touches only this function — never `dispatch_bean_child`'s match (or
// `parse_bean`'s attribute loop, for `normalize_pc_attr`) above.
// `#[allow(unused_variables, clippy::ptr_arg)]` for the same reason
// `dispatch.rs`'s own six stubs carry it — see that module's matching
// comment.
// ---------------------------------------------------------------------

/// Unit **P6** (`<meta key= value=>`, part of SB-02b) — `BeanCtx::meta`.
///
/// `<meta>` carries no children of its own and no anomaly this unit's
/// edge-case table calls for diagnosing (same "lenient skip, no diagnostic
/// invented" policy [`property::parse_meta_entry`](crate::property) documents
/// for its own copy of this exact shape) — `scope`/`diagnostics` are unused
/// as a result. Deliberately **not** shared with `property`'s/
/// `constructor_arg`'s own `parse_meta_entry` — each pushes into a different
/// accumulator (`BeanCtx::meta` here vs. a local `Vec<MetaEntry>` there), so
/// each module keeps its own copy rather than threading an accumulator
/// parameter through a shared helper for one two-line struct literal.
#[allow(unused_variables, clippy::ptr_arg)]
pub(crate) fn parse_meta(
    ctx: &mut BeanCtx,
    diagnostics: &mut Vec<Diagnostic>,
    scope: &NsScope,
    element: &XmlElement,
) {
    if let Some(entry) = parse_meta_entry(element) {
        ctx.meta.push(entry);
    }
}

/// `<meta key="..." value="...">` → a [`MetaEntry`], or `None` when either
/// attribute is missing — lenient skip, no diagnostic invented for a shape
/// the spec's edge-case table doesn't call out (same policy
/// `property::parse_meta_entry`/`constructor_arg::parse_meta_entry` document
/// for their own identical copies of this helper).
fn parse_meta_entry(element: &XmlElement) -> Option<MetaEntry> {
    let key = find_attr(&element.attrs, "key")?;
    let value = find_attr(&element.attrs, "value")?;
    Some(MetaEntry {
        key: spanned_attr(key),
        value: spanned_attr(value),
    })
}

/// Unit **P6** (`<qualifier type= value=>` + nested `<attribute key=
/// value=>`, part of SB-02b) — `BeanCtx::qualifiers`.
///
/// `type="..."` follows the same "present-but-empty is absent, invariant #5
/// upheld by never constructing one" treatment [`parse_class_ref`] documents
/// for `<bean class=...>` — reused here via [`parse_type_ref`] rather than
/// generalizing `parse_class_ref` itself over an attribute name, since the
/// two call sites' surrounding context (a whole-`<bean>` diagnostic vs. none
/// here) already diverges. `value="..."` is a raw string field with no
/// "never empty" contract (same as `Bean::scope`'s own ratification), so it
/// is read unconditionally, empty or not.
///
/// Each nested `<attribute>` child missing `key=`/`value=` is a lenient skip
/// — no diagnostic invented for an untested edge shape, same policy
/// [`parse_meta_entry`] documents for its own sibling shape. `diagnostics`
/// is unused as a result.
#[allow(unused_variables, clippy::ptr_arg)]
pub(crate) fn parse_qualifier(
    ctx: &mut BeanCtx,
    diagnostics: &mut Vec<Diagnostic>,
    scope: &NsScope,
    element: &XmlElement,
) {
    let own_scope = NsScope::from_element(element, Some(scope));

    let type_ = parse_type_ref(&element.attrs);
    let value = find_attr(&element.attrs, "value").map(spanned_attr);

    let mut attributes = Vec::new();
    for child in &element.children {
        let XmlNode::Element(child_element) = child else {
            // Direct text (whitespace between `<attribute>` children) has
            // nothing to attach to at this level — same drop
            // `bean::parse_bean`'s own child loop documents.
            continue;
        };
        // Overlay `child_element`'s own `xmlns`/`xmlns:*` declarations
        // before resolving its name — a xmlns declaration on `<attribute>`
        // itself applies to that element's own name, same as
        // `dispatch_root_child`/`dispatch_bean_child` do for their own
        // children (see those functions' doc comments, and
        // `collection.rs`'s `parse_map`/`parse_map_entry`/`parse_props`,
        // which document this identical fix for their own nested-element
        // detection).
        let child_scope = NsScope::from_element(child_element, Some(&own_scope));
        let (ns, local) = resolve_qname(&child_element.name, &child_scope);
        if local == "attribute" && is_beans_ns(&ns) {
            if let Some(pair) = parse_attribute_pair(child_element) {
                attributes.push(pair);
            }
        }
    }

    ctx.qualifiers.push(Qualifier {
        span: element.span,
        type_,
        value,
        attributes,
    });
}

/// `type="..."` → a `ClassRef`, or `None` when the attribute is absent or
/// present-but-empty — see [`parse_qualifier`]'s own doc comment.
fn parse_type_ref(attrs: &[XmlAttr]) -> Option<Spanned<ClassRef>> {
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

/// `<attribute key="..." value="...">` → an [`AttrPair`], or `None` when
/// either attribute is missing — see [`parse_qualifier`]'s own doc comment.
fn parse_attribute_pair(element: &XmlElement) -> Option<AttrPair> {
    let key = find_attr(&element.attrs, "key")?;
    let value = find_attr(&element.attrs, "value")?;
    Some(AttrPair {
        key: spanned_attr(key),
        value: spanned_attr(value),
    })
}

/// Unit **P7** (SB-02b's decorator slice — see build plan's "P6
/// qualifier/description/meta/decorator [U4, decorator←P7]") —
/// `BeanCtx::decorators`. Also the catch-all arm in `dispatch_bean_child`
/// above for any non-`beans` namespace directly inside a `<bean>`
/// (`aop:scoped-proxy`, ...).
///
/// `scope` is the PARENT scope, same convention
/// `dispatch::parse_namespaced` documents — both call the same shared
/// builder, [`crate::namespaced::build_namespaced_element`], which
/// re-derives its own overlay.
pub(crate) fn parse_decorator(
    ctx: &mut BeanCtx,
    diagnostics: &mut Vec<Diagnostic>,
    scope: &NsScope,
    element: &XmlElement,
) {
    ctx.decorators
        .push(crate::namespaced::build_namespaced_element(
            scope,
            element,
            diagnostics,
        ));
}

/// Unit **P8** (`<lookup-method name= bean=>`, SB-06b) —
/// `BeanCtx::lookup_methods`.
///
/// `name=` is optional per the frozen model (`LookupMethod::name`) —
/// recorded verbatim (empty or not, same "raw string, no never-empty
/// contract" ratification `resolve_scope`'s doc comment documents for
/// `Bean::scope`) when present, `None` otherwise; no diagnostic invented
/// for an unnamed `<lookup-method>`, a shape the spec's edge-case table
/// doesn't call out.
///
/// `bean=` is different: per `LookupMethod::bean`'s own doc comment, a
/// **missing** `bean=` — not just a present-but-empty one — still raises
/// `RefWithoutTarget` (element preserved, no edge emitted). See
/// [`required_bean_ref`]'s own doc comment for why this diverges from
/// [`spanned_bean_ref`]'s `parent=`/`factory-bean=` treatment.
#[allow(unused_variables, clippy::ptr_arg)]
pub(crate) fn parse_lookup_method(
    ctx: &mut BeanCtx,
    diagnostics: &mut Vec<Diagnostic>,
    scope: &NsScope,
    element: &XmlElement,
) {
    let name = find_attr(&element.attrs, "name").map(spanned_attr);
    let bean = required_bean_ref(&element.attrs, "bean", element, RefKind::Bean, diagnostics);
    ctx.lookup_methods.push(Spanned {
        value: LookupMethod { name, bean },
        span: element.span,
    });
}

/// Unit **P8** (`<replaced-method name= replacer=>` + nested
/// `<arg-type match=>`, SB-06b) — `BeanCtx::replaced_methods`.
///
/// `name=` is **required** by the frozen model (`ReplacedMethod::name` is
/// `Spanned<String>`, not `Option`) — a missing attribute falls back to an
/// empty string at `element`'s own span, the same infallible fallback
/// [`collection::parse_prop_entry`](crate::collection)/
/// [`property::resolve_property_name`](crate::property) apply for their own
/// required-by-model `name=`/`key=` (rule 4: no panics, no invented
/// `DiagCode` for a shape the spec's edge-case table doesn't call out).
///
/// `replacer=` gets the identical "missing is itself `RefWithoutTarget`"
/// treatment `bean=` gets in [`parse_lookup_method`] above — see
/// [`required_bean_ref`]'s own doc comment.
///
/// `<arg-type match="...">` is a **type-match pattern, not a `ClassRef`**
/// (spec's SB-12 exclusion list, and `ReplacedMethod::arg_types`'s own doc
/// comment) — collected as a plain `Spanned<String>`.
///
/// **Arg-type text-content ruling (orchestrator-approved)**: real Spring
/// also accepts a *text-content* form, `<arg-type>java.lang.String</arg-type>`,
/// as equivalent to `match=`. The precedence real Spring implements
/// (`BeanDefinitionParserDelegate.parseReplacedMethodSubElements`) is **not**
/// "attribute present wins" — it's "attribute has non-whitespace text wins,
/// else fall back to the text body", and a present-but-empty or
/// whitespace-only `match=` is treated exactly like `match=` being absent
/// entirely, not recorded verbatim. This mirrors that: `match=` wins only
/// when [`find_attr`] finds it *and* trimming its value leaves something
/// non-empty; otherwise this falls back to [`element_text`]'s trimmed value
/// via [`non_empty_trimmed`]. If neither the attribute nor the text body
/// survives trimming, that's a lenient skip — no entry pushed, no
/// diagnostic invented for this untested edge shape (same policy
/// [`parse_meta_entry`] documents for its own sibling shape) — so, unlike
/// `name=` above, `arg_types` never actually ends up holding an
/// empty-string entry in practice even though the model places no
/// never-empty contract on it. Either way the result stays a plain
/// `String`, never a `ClassRef` — this is still a type-match *pattern*, not
/// a class name.
pub(crate) fn parse_replaced_method(
    ctx: &mut BeanCtx,
    diagnostics: &mut Vec<Diagnostic>,
    scope: &NsScope,
    element: &XmlElement,
) {
    let own_scope = NsScope::from_element(element, Some(scope));

    let name = find_attr(&element.attrs, "name")
        .map(spanned_attr)
        .unwrap_or_else(|| Spanned {
            value: String::new(),
            span: element.span,
        });
    let replacer = required_bean_ref(
        &element.attrs,
        "replacer",
        element,
        RefKind::Bean,
        diagnostics,
    );

    let mut arg_types = Vec::new();
    for child in &element.children {
        let XmlNode::Element(child_element) = child else {
            // Direct text (whitespace between `<arg-type>` children) has
            // nothing to attach to at this level — same drop
            // `bean::parse_bean`'s own child loop documents.
            continue;
        };
        // Overlay `child_element`'s own `xmlns`/`xmlns:*` declarations
        // before resolving its name — same namespace-scoping fix
        // `parse_qualifier`'s own `<attribute>` detection documents.
        let child_scope = NsScope::from_element(child_element, Some(&own_scope));
        let (ns, local) = resolve_qname(&child_element.name, &child_scope);
        if local == "arg-type" && is_beans_ns(&ns) {
            let match_attr = find_attr(&child_element.attrs, "match")
                .filter(|attr| !attr.value.value.trim().is_empty());
            if let Some(match_attr) = match_attr {
                arg_types.push(spanned_attr(match_attr));
            } else if let Some(text) = non_empty_trimmed(element_text(child_element)) {
                arg_types.push(text);
            }
        }
    }

    ctx.replaced_methods.push(Spanned {
        value: ReplacedMethod {
            name,
            replacer,
            arg_types,
        },
        span: element.span,
    });
}

/// Trims ASCII whitespace off both ends of a [`Spanned<String>`], narrowing
/// its span to match, or `None` if nothing survives the trim (empty text,
/// or a body that is whitespace-only) — the arg-type text-content ruling's
/// "empty text -> absent" edge case, folded into
/// [`parse_replaced_method`]'s own lenient skip for a `<arg-type>` with
/// neither `match=` nor usable text. ASCII-only trimming is safe here for
/// the same reason [`crate::dispatch::split_name_tokens`] documents:
/// whitespace never occurs as a continuation or lead byte of a multi-byte
/// UTF-8 sequence, so trimming it never mis-slices a codepoint.
///
/// The byte-offset narrowing below assumes `text.span` maps linearly onto
/// `text.value` — true when `element_text` (which built `text`) saw exactly
/// one text/CDATA run, since a single run's own span already excludes any
/// delimiter bytes (invariant #4). It is **not** true when multiple runs
/// got concatenated (e.g. text split by a `<!--comment-->`, or a plain-text
/// run adjacent to a `<![CDATA[...]]>` run): `element_text` returns a
/// min-max *hull* span over every run, which also covers the bytes of
/// whatever sits between them (comment markup, CDATA delimiters). Slicing
/// that hull at a value-relative byte offset would land inside that
/// in-between markup instead of inside the text — so this only narrows the
/// span when the byte lengths actually agree (a cheap, exact proxy for
/// "single run, no gaps"); otherwise it keeps `text`'s own untrimmed hull
/// span, the same imprecision every other [`element_text`] caller already
/// accepts, rather than producing a span whose slice isn't the value.
fn non_empty_trimmed(text: Spanned<String>) -> Option<Spanned<String>> {
    let bytes = text.value.as_bytes();
    let mut start = 0usize;
    let mut end = bytes.len();
    while start < end && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if end <= start {
        return None;
    }
    let span_len_matches_value = (text.span.end - text.span.start) as usize == text.value.len();
    let span = if span_len_matches_value {
        ByteSpan {
            start: text.span.start + start as u32,
            end: text.span.start + end as u32,
        }
    } else {
        text.span
    };
    Some(Spanned {
        value: text.value[start..end].to_string(),
        span,
    })
}

/// Builds a `Spanned<BeanRef>` from a reference attribute that an
/// element's **entire purpose** is to carry (`<lookup-method bean=>`/
/// `<replaced-method replacer=>`) — unlike [`spanned_bean_ref`]'s
/// `parent=`/`factory-bean=` treatment (where an absent attribute is
/// unremarkable — the bean simply has no parent — and only a
/// present-but-empty one is diagnosed), a **missing** attribute here is
/// itself `RefWithoutTarget`, diagnosed at `element`'s own span since
/// there is no attribute span to anchor to. Same "element preserved, no
/// edge" additive policy every `RefWithoutTarget` site in this crate
/// follows — the caller still pushes the `LookupMethod`/`ReplacedMethod`
/// entry with `bean`/`replacer: None`. Mirrors
/// `inject_value::bean_ref_from_attr`'s identical "missing is itself the
/// diagnosable anomaly" treatment for `<ref>`/`<idref>` missing all of
/// their own reference-shaped attributes.
fn required_bean_ref(
    attrs: &[XmlAttr],
    attr_name: &str,
    element: &XmlElement,
    kind: RefKind,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Spanned<BeanRef>> {
    match find_attr(attrs, attr_name) {
        Some(attr) => spanned_bean_ref(attr, kind, diagnostics),
        None => {
            diagnostics.push(Diagnostic {
                code: DiagCode::RefWithoutTarget,
                span: Some(element.span),
                message: format!("<{}> has no {}=", element.name, attr_name),
            });
            None
        }
    }
}

/// P2 (`p:`/`c:`-namespace attribute normalization, SB-08) —
/// `BeanCtx::properties`/`BeanCtx::constructor_args`. Called once per
/// attribute on `<bean>` itself (build plan U4 (b): "prefixed-attribute
/// hook in the bean-attribute loop") with `scope` already overlaid with `<bean>`'s own
/// `xmlns`/`xmlns:*` declarations (`parse_bean`'s call site passes
/// `own_scope`, not the pre-overlay parent scope every other stub in this
/// file receives — see `parse_bean`'s own doc comment) — every unprefixed
/// attribute (already claimed by one of `parse_bean`'s own `find_attr`
/// calls) and every `xmlns`/`xmlns:*` declaration attribute itself is a
/// no-op pass-through; only a `p:`/`c:`-namespace attribute produces
/// anything.
///
/// Namespace membership accepts either the real declared URI or the raw
/// `p`/`c` prefix text itself when no `xmlns:p`/`xmlns:c` declaration is in
/// scope — same "resolved URI, or raw prefix" fallback policy
/// `dispatch::is_context_ns`/`is_util_ns` already apply for `context:`/
/// `util:`, reused here via [`is_p_ns`]/[`is_c_ns`] so a hand-written
/// fixture that skips the (very common, but not XSD-mandatory) `xmlns:p`
/// declaration still normalizes.
///
/// `p:foo="literal"` → `Property{name: foo, value: Value}`;
/// `p:foo-ref="bean"` → `Property{name: foo, value: Ref}`. `c:_0="literal"`/
/// `c:_0-ref="bean"` → `ConstructorArg{index: Some(0), value: ..}` (an
/// underscore followed by an all-digit run, per Spring's own c-namespace
/// convention — anything else, including a malformed `_0a`, falls back to
/// the named form below rather than silently guessing at an index). Any
/// other local name (after stripping a trailing `-ref`) is the named form:
/// `c:name="literal"`/`c:name-ref="bean"` → `ConstructorArg{name: Some(name),
/// value: ..}`.
///
/// Every produced entry's `span` is the **attribute's own span** (`name=`
/// through the closing quote of its value — see `Property::span`'s and
/// `ConstructorArg::span`'s own doc comments, which already document this
/// exact "or, for a p:/c:-namespace attribute, the attribute's own span"
/// case), not `element.span` — a `p:`/`c:`-namespace entry has no wrapping
/// child element to span instead. `Property.name`/`ConstructorArg.name`'s
/// own span is narrower still: just the bytes of the local name with the
/// `p:`/`c:` prefix and any trailing `-ref` stripped off (via
/// [`local_name_span`]), computed from byte offsets rather than reparsing —
/// safe here the same way `bean::split_name_tokens`'s own doc comment
/// reasons about its delimiter set: `:`/`-` never occur as a continuation
/// or lead byte of a multi-byte UTF-8 sequence, so slicing on their byte
/// positions never lands mid-character even for a non-ASCII property name.
///
/// A `-ref` attribute whose value resolves to nothing (present but empty)
/// already raises `RefWithoutTarget` inside [`ref_from_attr`] — this
/// function still pushes the `Property`/`ConstructorArg` entry in that case
/// (same "preserve both, diagnose once" additive policy every other
/// ref-shaped site in this crate follows), falling back to
/// `InjectValue::Null` at the attribute's *value* span for the otherwise
/// bean-ref-shaped value, mirroring `property::resolve_value`'s identical
/// "nothing resolved" fallback for `<property>` itself.
pub(crate) fn normalize_pc_attr(
    ctx: &mut BeanCtx,
    diagnostics: &mut Vec<Diagnostic>,
    scope: &NsScope,
    attr: &XmlAttr,
) {
    let Some((prefix, rest)) = attr.name.split_once(':') else {
        // Unprefixed attribute — already claimed (or not) by `parse_bean`'s
        // own `find_attr` calls; nothing for the p/c hook to do.
        return;
    };
    if prefix == "xmlns" {
        // `xmlns:p="..."`/`xmlns:c="..."` themselves are namespace
        // bookkeeping (already folded into `scope` by
        // `NsScope::from_element`), never a p:/c: property/ctor-arg
        // attribute in their own right.
        return;
    }

    let (ns, _local) = resolve_qname(&attr.name, scope);
    let is_p = is_p_ns(&ns);
    let is_c = is_c_ns(&ns);
    if !is_p && !is_c {
        return;
    }

    // Whole-attribute span (`name="value"`'s full extent, closing quote
    // included — `attr.value.span` itself stops one byte short of it, same
    // as every other attribute-value span this crate produces) — see this
    // function's own doc comment on why `Property`/`ConstructorArg.span`
    // reads from here, not `element.span`.
    let attr_span = ByteSpan {
        start: attr.name_span.start,
        end: attr.value.span.end + 1,
    };
    // Byte offset where `rest` (the local name, still carrying a possible
    // trailing `-ref`) begins within the source: past the prefix and the
    // `:` separator, both single-byte-per-char ASCII.
    let local_start = attr.name_span.start + prefix.len() as u32 + 1;

    let (local, is_ref) = match rest.strip_suffix("-ref") {
        Some(stripped) => (stripped, true),
        None => (rest, false),
    };

    let value = if is_ref {
        match ref_from_attr(attr, diagnostics) {
            Some(bean_ref) => InjectValue::Ref(bean_ref),
            // Empty ref= already raised RefWithoutTarget inside
            // `ref_from_attr` — still emit the entry (additive diagnostic
            // policy, see this function's own doc comment), opaque `Null`
            // in place of the unresolved reference.
            None => InjectValue::Null(attr.value.span),
        }
    } else {
        InjectValue::Value(value_lit_from_attr(attr))
    };

    if is_p {
        ctx.properties.push(Property {
            span: attr_span,
            name: Spanned {
                value: local.to_string(),
                span: local_name_span(local_start, local),
            },
            value,
            meta: Vec::new(),
        });
        return;
    }

    // `is_c` (the only remaining case, since the two are mutually
    // exclusive namespace URIs/prefixes).
    let (index, name) = parse_c_local(local, local_start);
    ctx.constructor_args.push(ConstructorArg {
        span: attr_span,
        index,
        type_ref: None,
        name,
        value,
        meta: Vec::new(),
    });
}

/// Spring `p`-namespace URI (`p:foo="literal"`/`p:foo-ref="bean"`).
const P_NS_URI: &str = "http://www.springframework.org/schema/p";
/// Spring `c`-namespace URI (`c:_0="literal"`/`c:name-ref="bean"`).
const C_NS_URI: &str = "http://www.springframework.org/schema/c";

/// `true` for the declared `p` URI or (no `xmlns:p` in scope) the raw `p`
/// prefix text itself — see [`normalize_pc_attr`]'s own doc comment for why
/// the raw-prefix fallback matters here.
fn is_p_ns(ns: &str) -> bool {
    ns == P_NS_URI || ns == "p"
}

/// `true` for the declared `c` URI or (no `xmlns:c` in scope) the raw `c`
/// prefix text itself — same fallback [`is_p_ns`] documents.
fn is_c_ns(ns: &str) -> bool {
    ns == C_NS_URI || ns == "c"
}

/// The byte span of `local` (already stripped of its `p:`/`c:` prefix and
/// any trailing `-ref` suffix by [`normalize_pc_attr`]) within the source,
/// given `start` — the offset where `local`'s first byte sits.
fn local_name_span(start: u32, local: &str) -> ByteSpan {
    ByteSpan {
        start,
        end: start + local.len() as u32,
    }
}

/// Splits a `c:`-namespace local name (already stripped of any trailing
/// `-ref`) into the index form (`_0`, `_12`, ...) or the named form
/// (anything else) — see [`normalize_pc_attr`]'s own doc comment for the
/// exact recognition rule. `start` is `local`'s own byte offset, threaded
/// through to [`local_name_span`] for the named form's `Spanned<String>`.
fn parse_c_local(local: &str, start: u32) -> (Option<u32>, Option<Spanned<String>>) {
    if let Some(digits) = local.strip_prefix('_') {
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(index) = digits.parse::<u32>() {
                return (Some(index), None);
            }
        }
    }
    (
        None,
        Some(Spanned {
            value: local.to_string(),
            span: local_name_span(start, local),
        }),
    )
}

// ---------------------------------------------------------------------
// Tests — P6 (SB-02b: qualifier/meta/description; decorator is P7's own
// slice, already exercised by `namespaced.rs`'s + `tests/p7_namespaced.rs`'s
// own suites).
//
// In-module (rather than only `tests/p6_qualifier_meta_decorator.rs`)
// because `parse_bean`/`parse_qualifier`/`parse_meta` are `pub(crate)` — a
// seam not visible from an external integration-test binary, the same
// situation `namespaced.rs`'s own `#[cfg(test)] mod tests` doc comment
// documents. `tests/p6_qualifier_meta_decorator.rs` carries an end-to-end
// smoke test through the public `beans_xml::parse` API, proving the real
// call sites (`dispatch_bean_child`'s `"qualifier"`/`"meta"` arms) actually
// reach these functions in production, not just in these unit tests.
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::build_tree;

    fn parse_bean_fragment(source: &str) -> (Bean, Vec<Diagnostic>) {
        let element = build_tree(source).root.expect("root element found");
        let mut diagnostics = Vec::new();
        // `parse_bean` returns `Box<Bean>` (stack-diet — see its own doc
        // comment); every test in this module wants an owned `Bean` same as
        // before, so unbox once here rather than touching every call site.
        let bean = *parse_bean(&NsScope::default(), &element, &mut diagnostics, 0);
        (bean, diagnostics)
    }

    // -------------------------------------------------------------
    // qualifier + nested attribute.
    // -------------------------------------------------------------

    #[test]
    fn sb02b_qualifier_with_attribute_snapshot() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<qualifier type="com.example.Genuine" value="main">"#,
            r#"<attribute key="priority" value="high"/>"#,
            "</qualifier>",
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        assert_eq!(bean.qualifiers.len(), 1);
        let qualifier = &bean.qualifiers[0];
        assert_eq!(
            qualifier.type_.as_ref().map(|t| t.value.raw.as_str()),
            Some("com.example.Genuine")
        );
        assert_eq!(
            qualifier.value.as_ref().map(|v| v.value.as_str()),
            Some("main")
        );
        assert_eq!(qualifier.attributes.len(), 1);
        assert_eq!(qualifier.attributes[0].key.value, "priority");
        assert_eq!(qualifier.attributes[0].value.value, "high");
        insta::assert_json_snapshot!(bean);
    }

    #[test]
    fn sb02b_qualifier_without_type_or_value_still_parses() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            "<qualifier/>",
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(diagnostics.is_empty());
        assert_eq!(bean.qualifiers.len(), 1);
        let qualifier = &bean.qualifiers[0];
        assert_eq!(qualifier.type_, None);
        assert_eq!(qualifier.value, None);
        assert!(qualifier.attributes.is_empty());
    }

    #[test]
    fn sb02b_qualifier_empty_type_is_treated_as_absent() {
        // Same invariant #5 treatment `parse_class_ref` gives `<bean
        // class="">` — a present-but-empty `type=` must never construct a
        // `ClassRef` with an empty `raw`.
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<qualifier type="" value="main"/>"#,
            "</bean>",
        );
        let (bean, _diagnostics) = parse_bean_fragment(source);
        assert_eq!(bean.qualifiers.len(), 1);
        assert_eq!(bean.qualifiers[0].type_, None);
    }

    #[test]
    fn sb02b_multiple_qualifiers_are_all_preserved() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<qualifier type="com.example.Genuine"/>"#,
            r#"<qualifier value="secondary"/>"#,
            "</bean>",
        );
        let (bean, _diagnostics) = parse_bean_fragment(source);
        assert_eq!(bean.qualifiers.len(), 2);
    }

    #[test]
    fn sb02b_qualifier_attribute_missing_key_or_value_is_lenient_skip() {
        // Symmetric with `sb02b_meta_missing_key_or_value_is_lenient_skip`
        // — `parse_attribute_pair`'s `None` branch (missing `key=` or
        // `value=`) was previously exercised only implicitly.
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<qualifier type="com.example.Genuine">"#,
            r#"<attribute value="onlyValue"/>"#,
            r#"<attribute key="onlyKey"/>"#,
            "</qualifier>",
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "no diagnostic invented for this untested edge shape: {diagnostics:?}"
        );
        assert_eq!(bean.qualifiers.len(), 1);
        assert!(
            bean.qualifiers[0].attributes.is_empty(),
            "an <attribute> missing key= or value= must be skipped, not partially recorded: {:?}",
            bean.qualifiers[0].attributes
        );
    }

    #[test]
    fn sb02b_qualifier_multiple_attributes_are_all_preserved_in_order() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<qualifier type="com.example.Genuine">"#,
            r#"<attribute key="a" value="1"/>"#,
            r#"<attribute key="b" value="2"/>"#,
            "</qualifier>",
            "</bean>",
        );
        let (bean, _diagnostics) = parse_bean_fragment(source);
        assert_eq!(
            bean.qualifiers[0]
                .attributes
                .iter()
                .map(|a| (a.key.value.as_str(), a.value.value.as_str()))
                .collect::<Vec<_>>(),
            vec![("a", "1"), ("b", "2")]
        );
    }

    #[test]
    fn sb02b_qualifier_attribute_empty_value_is_recorded_as_present_but_empty() {
        // Asymmetric with `type=""` (treated as absent, see
        // `sb02b_qualifier_empty_type_is_treated_as_absent`): `<attribute
        // value=""/>`'s `value` field is a raw string with no "never empty"
        // contract, same ratification `resolve_scope`'s doc comment
        // documents for `Bean::scope`.
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<qualifier type="com.example.Genuine">"#,
            r#"<attribute key="priority" value=""/>"#,
            "</qualifier>",
            "</bean>",
        );
        let (bean, _diagnostics) = parse_bean_fragment(source);
        assert_eq!(bean.qualifiers[0].attributes.len(), 1);
        assert_eq!(bean.qualifiers[0].attributes[0].key.value, "priority");
        assert_eq!(bean.qualifiers[0].attributes[0].value.value, "");
    }

    #[test]
    fn sb02b_qualifier_nested_attribute_non_beans_ns_is_dropped() {
        // Pins the namespace-scoping fix: a xmlns declared on `<attribute>`
        // itself (not on `<qualifier>`) must be resolved against its own
        // overlay, not the qualifier's — an `<attribute>` element that
        // redeclares `xmlns` to something other than the beans namespace is
        // dropped, same as `dispatch_bean_child`'s own non-beans-ns
        // handling.
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<qualifier type="com.example.Genuine">"#,
            r#"<attribute xmlns="http://not-beans" key="priority" value="high"/>"#,
            "</qualifier>",
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        assert!(
            bean.qualifiers[0].attributes.is_empty(),
            "a non-beans-ns nested <attribute> must not be captured: {:?}",
            bean.qualifiers[0].attributes
        );
    }

    // -------------------------------------------------------------
    // meta.
    // -------------------------------------------------------------

    #[test]
    fn sb02b_meta_snapshot() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<meta key="buildTool" value="maven"/>"#,
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        assert_eq!(bean.meta.len(), 1);
        assert_eq!(bean.meta[0].key.value, "buildTool");
        assert_eq!(bean.meta[0].value.value, "maven");
        insta::assert_json_snapshot!(bean);
    }

    #[test]
    fn sb02b_multiple_meta_entries_are_all_preserved_in_order() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<meta key="a" value="1"/>"#,
            r#"<meta key="b" value="2"/>"#,
            "</bean>",
        );
        let (bean, _diagnostics) = parse_bean_fragment(source);
        assert_eq!(
            bean.meta
                .iter()
                .map(|m| (m.key.value.as_str(), m.value.value.as_str()))
                .collect::<Vec<_>>(),
            vec![("a", "1"), ("b", "2")]
        );
    }

    #[test]
    fn sb02b_meta_missing_key_or_value_is_lenient_skip() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<meta value="onlyValue"/>"#,
            r#"<meta key="onlyKey"/>"#,
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            bean.meta.is_empty(),
            "a <meta> missing key= or value= must be skipped, not partially recorded: {:?}",
            bean.meta
        );
        assert!(
            diagnostics.is_empty(),
            "no diagnostic invented for this untested edge shape: {diagnostics:?}"
        );
    }

    // -------------------------------------------------------------
    // description (U4-owned, but exercised together here since it shares
    // SB-02b's own spec row and test-design bullet with qualifier/meta).
    // -------------------------------------------------------------

    #[test]
    fn sb02b_description_snapshot() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            "<description>Handles example widgets.</description>",
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        assert_eq!(
            bean.description.as_ref().map(|d| d.value.as_str()),
            Some("Handles example widgets.")
        );
        insta::assert_json_snapshot!(bean);
    }

    // -------------------------------------------------------------
    // scoped-proxy decorator preserved alongside qualifier/meta/description
    // — pins that P6's own additions never clobber P7's `decorators` slice
    // on the same `Bean`.
    // -------------------------------------------------------------

    #[test]
    fn sb02b_scoped_proxy_decorator_preserved_alongside_qualifier_meta_description_snapshot() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService" "#,
            r#"xmlns:aop="http://www.springframework.org/schema/aop">"#,
            "<description>Handles example widgets.</description>",
            r#"<qualifier type="com.example.Genuine" value="main">"#,
            r#"<attribute key="priority" value="high"/>"#,
            "</qualifier>",
            r#"<meta key="buildTool" value="maven"/>"#,
            r#"<aop:scoped-proxy proxy-target-class="true"/>"#,
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        assert_eq!(
            bean.description.as_ref().map(|d| d.value.as_str()),
            Some("Handles example widgets.")
        );
        assert_eq!(bean.qualifiers.len(), 1);
        assert_eq!(bean.meta.len(), 1);
        assert_eq!(bean.decorators.len(), 1);
        assert_eq!(bean.decorators[0].local, "scoped-proxy");
        assert_eq!(
            bean.decorators[0].ns,
            "http://www.springframework.org/schema/aop"
        );
        insta::assert_json_snapshot!(bean);
    }

    // -------------------------------------------------------------
    // P8 — method injection: <lookup-method name= bean=> /
    // <replaced-method name= replacer=> + nested <arg-type match=> (SB-06b).
    // -------------------------------------------------------------

    #[test]
    fn sb06b_lookup_method_with_name_and_bean_snapshot() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<lookup-method name="createCommand" bean="commandBean"/>"#,
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
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
        assert_eq!(
            lookup_method.bean.as_ref().unwrap().value.kind,
            RefKind::Bean
        );
        insta::assert_json_snapshot!(bean);
    }

    #[test]
    fn sb06b_lookup_method_without_name_still_carries_bean() {
        // name= is optional per the frozen model — absent is unremarkable,
        // no diagnostic.
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<lookup-method bean="commandBean"/>"#,
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        assert_eq!(bean.lookup_methods.len(), 1);
        let lookup_method = &bean.lookup_methods[0].value;
        assert_eq!(lookup_method.name, None);
        assert_eq!(
            lookup_method.bean.as_ref().map(|b| b.value.raw.as_str()),
            Some("commandBean")
        );
    }

    #[test]
    fn sb06b_lookup_method_missing_bean_is_ref_without_target_element_preserved() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<lookup-method name="createCommand"/>"#,
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert_eq!(
            bean.lookup_methods.len(),
            1,
            "element must still be preserved"
        );
        let lookup_method = &bean.lookup_methods[0].value;
        assert_eq!(
            lookup_method.name.as_ref().map(|n| n.value.as_str()),
            Some("createCommand")
        );
        assert_eq!(lookup_method.bean, None, "no edge without a bean= target");
        assert!(diagnostics
            .iter()
            .any(|d| d.code == DiagCode::RefWithoutTarget));
    }

    #[test]
    fn sb06b_lookup_method_empty_bean_is_also_ref_without_target() {
        // Present-but-empty bean= is a second, distinct route to the same
        // RefWithoutTarget outcome (via `spanned_bean_ref`, not the
        // missing-attribute branch of `required_bean_ref`).
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<lookup-method name="createCommand" bean=""/>"#,
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert_eq!(bean.lookup_methods.len(), 1);
        assert_eq!(bean.lookup_methods[0].value.bean, None);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == DiagCode::RefWithoutTarget));
    }

    #[test]
    fn sb06b_multiple_lookup_methods_are_all_preserved() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<lookup-method name="a" bean="beanA"/>"#,
            r#"<lookup-method name="b" bean="beanB"/>"#,
            "</bean>",
        );
        let (bean, _diagnostics) = parse_bean_fragment(source);
        assert_eq!(bean.lookup_methods.len(), 2);
    }

    #[test]
    fn sb06b_replaced_method_with_name_replacer_and_arg_types_snapshot() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<replaced-method name="computeValue" replacer="replacerBean">"#,
            r#"<arg-type match="java.lang.String"/>"#,
            r#"<arg-type match="int"/>"#,
            "</replaced-method>",
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
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
        insta::assert_json_snapshot!(bean);
    }

    #[test]
    fn sb06b_replaced_method_without_arg_types_is_empty_vec() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<replaced-method name="computeValue" replacer="replacerBean"/>"#,
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(diagnostics.is_empty());
        assert!(bean.replaced_methods[0].value.arg_types.is_empty());
    }

    #[test]
    fn sb06b_replaced_method_missing_replacer_is_ref_without_target_element_preserved() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<replaced-method name="computeValue"/>"#,
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert_eq!(
            bean.replaced_methods.len(),
            1,
            "element must still be preserved"
        );
        let replaced_method = &bean.replaced_methods[0].value;
        assert_eq!(replaced_method.name.value, "computeValue");
        assert_eq!(
            replaced_method.replacer, None,
            "no edge without a replacer= target"
        );
        assert!(diagnostics
            .iter()
            .any(|d| d.code == DiagCode::RefWithoutTarget));
    }

    #[test]
    fn sb06b_replaced_method_empty_replacer_is_also_ref_without_target() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<replaced-method name="computeValue" replacer=""/>"#,
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert_eq!(bean.replaced_methods[0].value.replacer, None);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == DiagCode::RefWithoutTarget));
    }

    #[test]
    fn sb06b_replaced_method_missing_name_falls_back_to_empty_string_no_diagnostic() {
        // `ReplacedMethod::name` is required by the frozen model
        // (`Spanned<String>`, not `Option`) — a missing `name=` must not
        // panic, and (per the "no invented DiagCode" rule) must not raise
        // one either.
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<replaced-method replacer="replacerBean"/>"#,
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "no diagnostic invented for missing name=: {diagnostics:?}"
        );
        assert_eq!(bean.replaced_methods[0].value.name.value, "");
        assert_eq!(
            bean.replaced_methods[0]
                .value
                .replacer
                .as_ref()
                .map(|r| r.value.raw.as_str()),
            Some("replacerBean")
        );
    }

    #[test]
    fn sb06b_arg_type_missing_match_is_lenient_skip() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<replaced-method name="computeValue" replacer="replacerBean">"#,
            "<arg-type/>",
            r#"<arg-type match="int"/>"#,
            "</replaced-method>",
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "no diagnostic invented for this untested edge shape: {diagnostics:?}"
        );
        assert_eq!(
            bean.replaced_methods[0]
                .value
                .arg_types
                .iter()
                .map(|a| a.value.as_str())
                .collect::<Vec<_>>(),
            vec!["int"]
        );
    }

    #[test]
    fn sb06b_arg_type_present_but_empty_match_is_lenient_skip() {
        // Real Spring's own precedence (`match = StringUtils.hasText(match)
        // ? match : DomUtils.getTextValue(argTypeEle)`, then only
        // `addTypeIdentifier` when that result `hasText`) never records an
        // empty identifier — a present-but-empty `match=""` with no text
        // body falls through exactly like `match=` being fully absent, not
        // a distinct "recorded as empty string" case. This is the
        // negative-space companion to
        // `arg_type_whitespace_match_falls_back_to_text_body` below.
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<replaced-method name="computeValue" replacer="replacerBean">"#,
            r#"<arg-type match=""/>"#,
            r#"<arg-type match="int"/>"#,
            "</replaced-method>",
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "no diagnostic invented for a present-but-empty match=: {diagnostics:?}"
        );
        assert_eq!(
            bean.replaced_methods[0]
                .value
                .arg_types
                .iter()
                .map(|a| a.value.as_str())
                .collect::<Vec<_>>(),
            vec!["int"],
            "an empty match= must never be recorded verbatim — same lenient skip as a fully \
             absent match="
        );
    }

    #[test]
    fn arg_type_whitespace_match_falls_back_to_text_body() {
        // The bug this pins against: `match=` only wins when it has
        // non-whitespace text (real Spring's `StringUtils.hasText`) — a
        // whitespace-only `match=` must fall back to the text body exactly
        // like a fully absent `match=`, not be recorded as-is.
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<replaced-method name="computeValue" replacer="replacerBean">"#,
            "<arg-type match=\"   \">com.example.ArgType</arg-type>",
            "</replaced-method>",
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        assert_eq!(
            bean.replaced_methods[0]
                .value
                .arg_types
                .iter()
                .map(|a| a.value.as_str())
                .collect::<Vec<_>>(),
            vec!["com.example.ArgType"],
            "a whitespace-only match= must fall back to the text body, not be recorded as-is"
        );
    }

    #[test]
    fn p4_arg_type_text_content_form_is_accepted_as_string_pattern() {
        // Arg-type text-content ruling: `<arg-type>java.lang.String</arg-type>`
        // (no `match=`) is accepted the same way real Spring accepts it —
        // trimmed text becomes the `arg_types` entry, still a plain
        // `String`, never a `ClassRef`.
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<replaced-method name="computeValue" replacer="replacerBean">"#,
            "<arg-type>  java.lang.String  </arg-type>",
            r#"<arg-type match="int"/>"#,
            "</replaced-method>",
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "text-content arg-type must not raise a diagnostic: {diagnostics:?}"
        );
        assert_eq!(
            bean.replaced_methods[0]
                .value
                .arg_types
                .iter()
                .map(|a| a.value.as_str())
                .collect::<Vec<_>>(),
            vec!["java.lang.String", "int"],
            "text-content form must be trimmed and land as a plain String pattern"
        );
    }

    #[test]
    fn p4_arg_type_match_attribute_wins_over_text_content_when_both_present() {
        // arg-type text-content ruling's stated precedence: `match=` wins when both the
        // attribute and a text body are present.
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<replaced-method name="computeValue" replacer="replacerBean">"#,
            r#"<arg-type match="java.lang.Integer">java.lang.String</arg-type>"#,
            "</replaced-method>",
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        assert_eq!(
            bean.replaced_methods[0]
                .value
                .arg_types
                .iter()
                .map(|a| a.value.as_str())
                .collect::<Vec<_>>(),
            vec!["java.lang.Integer"],
            "match= must win over the text body, and the text body must not also be pushed"
        );
    }

    #[test]
    fn p4_arg_type_whitespace_only_text_content_is_absent() {
        // arg-type text-content ruling's "empty text -> absent" edge case: a whitespace-only
        // (or fully empty) text body with no `match=` is the same lenient
        // skip as `<arg-type/>` with neither — no entry pushed, no
        // diagnostic invented.
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<replaced-method name="computeValue" replacer="replacerBean">"#,
            "<arg-type>   </arg-type>",
            "<arg-type></arg-type>",
            r#"<arg-type match="int"/>"#,
            "</replaced-method>",
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        assert_eq!(
            bean.replaced_methods[0]
                .value
                .arg_types
                .iter()
                .map(|a| a.value.as_str())
                .collect::<Vec<_>>(),
            vec!["int"],
            "whitespace-only/empty text-content must not push an arg_types entry"
        );
    }

    #[test]
    fn sb06b_arg_type_nested_non_beans_ns_is_dropped() {
        // Same namespace-scoping pin
        // `sb02b_qualifier_nested_attribute_non_beans_ns_is_dropped` applies
        // for `parse_qualifier`'s `<attribute>` loop, exercised here for
        // `parse_replaced_method`'s independently re-derived
        // `own_scope`/`child_scope`: an `<arg-type>` that redeclares
        // `xmlns` to something other than the beans namespace on itself
        // must not be captured, and the genuine beans-ns sibling must
        // still land.
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<replaced-method name="computeValue" replacer="replacerBean">"#,
            r#"<arg-type xmlns="http://not-beans" match="java.lang.String"/>"#,
            r#"<arg-type match="int"/>"#,
            "</replaced-method>",
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        assert_eq!(
            bean.replaced_methods[0]
                .value
                .arg_types
                .iter()
                .map(|a| a.value.as_str())
                .collect::<Vec<_>>(),
            vec!["int"],
            "a non-beans-ns nested <arg-type> must not be captured"
        );
    }

    #[test]
    fn arg_type_text_content_cdata_with_surrounding_whitespace_keeps_span_invariant() {
        // Invariant #4 regression pin: a pretty-printed `<arg-type>` whose
        // text content is split across multiple text/CDATA runs (here,
        // leading/trailing whitespace runs around a CDATA run) must never
        // produce a span whose slice isn't the value — `non_empty_trimmed`
        // must detect the multi-run hull and skip narrowing rather than
        // slicing into the `<![CDATA[`/`]]>` delimiter bytes that sit
        // between the runs.
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<replaced-method name="computeValue" replacer="replacerBean">"#,
            "<arg-type>\n  <![CDATA[java.lang.String]]>\n</arg-type>",
            "</replaced-method>",
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        let arg_type = &bean.replaced_methods[0].value.arg_types[0];
        assert_eq!(arg_type.value, "java.lang.String");
        let slice = &source[arg_type.span.start as usize..arg_type.span.end as usize];
        assert!(
            slice.contains(&arg_type.value),
            "span slice ({slice:?}) must contain the decoded value ({:?}), not markup bytes \
             from between the concatenated text/CDATA runs",
            arg_type.value
        );
    }

    #[test]
    fn sb06b_multiple_arg_types_preserve_document_order() {
        // `sb06b_replaced_method_with_name_replacer_and_arg_types_snapshot`
        // already pins a two-`<arg-type>` happy path, but its two values
        // (`java.lang.String`, `int`) don't rule out an accidental sort —
        // this uses a descending-alphabetical sequence so only
        // document-order preservation, not incidental sort order, makes
        // the assertion pass.
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<replaced-method name="computeValue" replacer="replacerBean">"#,
            r#"<arg-type match="zeta"/>"#,
            r#"<arg-type match="mu"/>"#,
            r#"<arg-type match="alpha"/>"#,
            "</replaced-method>",
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(diagnostics.is_empty());
        assert_eq!(
            bean.replaced_methods[0]
                .value
                .arg_types
                .iter()
                .map(|a| a.value.as_str())
                .collect::<Vec<_>>(),
            vec!["zeta", "mu", "alpha"]
        );
    }

    #[test]
    fn sb06b_lookup_and_replaced_method_coexist_on_same_bean() {
        let source = concat!(
            r#"<bean id="myBean" class="com.example.MyService">"#,
            r#"<lookup-method name="createCommand" bean="commandBean"/>"#,
            r#"<replaced-method name="computeValue" replacer="replacerBean">"#,
            r#"<arg-type match="java.lang.String"/>"#,
            "</replaced-method>",
            "</bean>",
        );
        let (bean, diagnostics) = parse_bean_fragment(source);
        assert!(diagnostics.is_empty());
        assert_eq!(bean.lookup_methods.len(), 1);
        assert_eq!(bean.replaced_methods.len(), 1);
    }
}
