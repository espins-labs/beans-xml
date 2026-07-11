//! Unit **U3** — root detection + `BeansFile` header (SB-01), plus the
//! **frozen root-child dispatch skeleton** the internal build plan's
//! "dispatch contract" section requires land before the parallel leaf wave
//! (P1/P3/P4/P5/P7/P10) fans out.
//!
//! Two things this module owns, per the internal build plan's
//! U3 row:
//!
//! 1. **SB-01 itself, fully implemented here** (not stubbed): [`is_beans_root`]
//!    (shared by [`crate::is_beans_doc`] and the real parse pipeline — see
//!    that function's own doc comment on why sharing it is what makes
//!    invariant #7 hold by construction, not by coincidence), and the
//!    `BeansFile` header fields — `profile`/`default-*` attributes plus the
//!    `<description>` child element — read directly in [`parse_beans_body`].
//! 2. **The root-child dispatch match** in [`dispatch_root_child`]: element
//!    name + resolved namespace → one of five per-element handler-fn stubs
//!    (`parse_alias`/`parse_import`/`parse_component_scan`/
//!    `parse_property_source`/`parse_namespaced`), every one of them an
//!    intentional no-op — filling a stub's body is leaf-unit work
//!    (P1/P3/P4/P5/P7 respectively), and per the build plan's stated
//!    contract, a leaf touches **only its own handler function**, never
//!    this match. The 1st-class allowlist (`context:component-scan`/
//!    `context:property-placeholder`, `util:properties`) and the
//!    `NamespacedElement` catch-all — pinned as this match's **last** arm —
//!    are frozen here so they can't be reshuffled or duplicated by a later
//!    leaf.
//!
//! `<bean>` itself is deliberately **not** one of the five stubs: bean
//! parsing (and the bean-child dispatch skeleton around it) is entirely
//! U4's contract (`parse_bean`, build plan U4 row) — this match only
//! reserves `"bean"` a place so it doesn't fall through to `UnknownElement`
//! or `NamespacedElement`, without calling anything not yet built. `<beans>`
//! itself (P10, SB-14, nested `<beans profile="...">`) is not an arm of
//! this match either — it is intercepted by [`BeansBodyFrame::step`] before
//! the match ever runs (see that method's own doc comment for the full P10
//! rationale, and this module's own heap-worklist-engine section doc
//! comment for why).
//!
//! `pub(crate)` — like `events`/`encoding`, not part of the published API
//! surface; `lib.rs`'s `parse`/`parse_bytes`/`is_beans_doc` are the public
//! entry points this module is wired behind.

use crate::bean::split_name_tokens;
use crate::events::{XmlAttr, XmlElement, XmlNode};
use crate::model::{
    Alias, BeansFileCtx, ByteSpan, ComponentScan, DiagCode, Diagnostic, Import, ImportKind,
    PropertySource, ScanFilter, Spanned,
};
use std::collections::HashMap;

/// Spring beans schema namespace URI — the default (usually unprefixed)
/// namespace of a `<beans>` document.
pub(crate) const BEANS_NS_URI: &str = "http://www.springframework.org/schema/beans";
/// Spring `context` namespace URI (`context:component-scan`,
/// `context:property-placeholder`, ...).
pub(crate) const CONTEXT_NS_URI: &str = "http://www.springframework.org/schema/context";
/// Spring `util` namespace URI (`util:properties`, `util:list`, ...).
pub(crate) const UTIL_NS_URI: &str = "http://www.springframework.org/schema/util";

// ---------------------------------------------------------------------
// Namespace resolution.
//
// `events::build_tree` deliberately stops at a namespace-agnostic tree
// (its own doc comment: "resolving a prefix:local name against xmlns:*
// declarations ... is entirely U3/U4's job") -- this is that job, for the
// root-child level.
// ---------------------------------------------------------------------

/// The `xmlns`/`xmlns:*` declarations in effect at some point in the tree.
/// Built by [`NsScope::from_element`], which overlays one element's own
/// declarations onto its parent's scope — the standard XML namespace
/// scoping rule (a declaration applies to the element it's on and every
/// descendant, until a closer declaration overrides it).
#[derive(Debug, Clone, Default)]
pub(crate) struct NsScope {
    default_ns: Option<String>,
    prefixes: HashMap<String, String>,
}

impl NsScope {
    /// Builds the scope in effect for `element`'s own children: start from
    /// `parent` (`None` for the document root, which has no ancestor scope
    /// to inherit) and overlay whatever `xmlns`/`xmlns:*` attributes
    /// `element` carries itself.
    pub(crate) fn from_element(element: &XmlElement, parent: Option<&NsScope>) -> Self {
        let mut scope = parent.cloned().unwrap_or_default();
        for attr in &element.attrs {
            if attr.name == "xmlns" {
                scope.default_ns = Some(attr.value.value.clone());
            } else if let Some(prefix) = attr.name.strip_prefix("xmlns:") {
                scope
                    .prefixes
                    .insert(prefix.to_string(), attr.value.value.clone());
            }
        }
        scope
    }
}

/// Splits a raw qualified name (`"context:component-scan"`) into
/// `(prefix, local)` — `prefix` is `None` for an unprefixed name.
fn split_qname(name: &str) -> (Option<&str>, &str) {
    match name.split_once(':') {
        Some((prefix, local)) => (Some(prefix), local),
        None => (None, name),
    }
}

/// Resolves `name` against `scope` into `(ns, local)`. `ns` is the
/// resolved namespace URI when `scope` declares one for the name's prefix
/// (or the default namespace, for an unprefixed name); otherwise it falls
/// back to the raw prefix text itself (empty string for an unprefixed name
/// with no declared default) — the same "resolved URI, or raw prefix"
/// policy `NamespacedElement::ns`'s own doc comment documents for the
/// published output, reused here for routing so the two can never
/// disagree about what a given element's namespace "is".
pub(crate) fn resolve_qname(name: &str, scope: &NsScope) -> (String, String) {
    let (prefix, local) = split_qname(name);
    let ns = match prefix {
        Some(p) => scope
            .prefixes
            .get(p)
            .cloned()
            .unwrap_or_else(|| p.to_string()),
        None => scope.default_ns.clone().unwrap_or_default(),
    };
    (ns, local.to_string())
}

/// `pub(crate)`: also used by U4's `parse_bean` bean-child dispatch
/// (`src/bean.rs`) to resolve the same beans-namespace membership test for
/// a `<bean>`'s own children, so the two dispatch layers can't disagree on
/// what counts as "in the beans namespace".
pub(crate) fn is_beans_ns(ns: &str) -> bool {
    ns.is_empty() || ns == BEANS_NS_URI
}

fn is_context_ns(ns: &str) -> bool {
    ns == CONTEXT_NS_URI || ns == "context"
}

fn is_util_ns(ns: &str) -> bool {
    ns == UTIL_NS_URI || ns == "util"
}

/// Cheap root-shape check shared by [`crate::is_beans_doc`] and the real
/// `parse`/`parse_bytes` pipeline in `lib.rs` — invariant #7
/// (`is_beans_doc(b) == parse_bytes(b).beans.is_some()`) holds because
/// both call sites resolve the root exactly the same way, not by
/// coincidence of two independently-written checks staying in sync.
///
/// Only the element's own namespace + local name matter: a namespace
/// prefix on the root tag itself (`<spring:beans
/// xmlns:spring="http://www.springframework.org/schema/beans">`) still
/// counts, as does no namespace declaration at all (many hand-written
/// fixtures never declare `xmlns`) — but a genuinely different namespace
/// bound to a `beans`-named local element does not (e.g. `<foo:beans
/// xmlns:foo="urn:not-spring">` is not a beans document just because its
/// local name happens to match).
pub(crate) fn is_beans_root(element: &XmlElement) -> bool {
    let scope = NsScope::from_element(element, None);
    let (ns, local) = resolve_qname(&element.name, &scope);
    local == "beans" && is_beans_ns(&ns)
}

// ---------------------------------------------------------------------
// SB-01: BeansFile header (profile / description / default-*).
// ---------------------------------------------------------------------

/// `pub(crate)`: shared with U4's `parse_bean` (`src/bean.rs`) — the same
/// "find one attribute by exact name" lookup a `<bean>`'s own core
/// attributes need.
pub(crate) fn find_attr<'a>(attrs: &'a [XmlAttr], name: &str) -> Option<&'a XmlAttr> {
    attrs.iter().find(|a| a.name == name)
}

/// `pub(crate)`: shared with U4's `parse_bean` (`src/bean.rs`).
pub(crate) fn spanned_attr(attr: &XmlAttr) -> Spanned<String> {
    Spanned {
        value: attr.value.value.clone(),
        span: attr.value.span,
    }
}

/// `"true"`/`"false"` → `Some(bool)`; anything else (attribute absent, or
/// present with some other value — e.g. the XSD-legal `"default"` literal
/// some `default-*` attributes accept, meaning "no override") → `None`,
/// same as the attribute never having been written at all. This crate
/// never panics on an attribute value it doesn't recognize (rule 4); it
/// simply doesn't have an opinion beyond "not confidently true or false".
/// `pub(crate)`: shared with U4's `parse_bean` (`src/bean.rs`) — `abstract`/
/// `lazy-init`/`primary`/`autowire-candidate` all follow the exact same
/// "true"/"false"/anything-else-is-None reading as `default-*`.
pub(crate) fn find_bool_attr(attrs: &[XmlAttr], name: &str) -> Option<bool> {
    match find_attr(attrs, name)?.value.value.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Every direct text child of `element`, **unmerged** — each entry is
/// exactly one `events::build_tree` `XmlNode::Text` run (one `Text`/
/// `CData`/`GeneralRef` event), so each entry's own `span` slices to
/// exactly its own `value` (invariant #4 holds per-entry by construction,
/// same guarantee `events.rs`'s own per-event spans already carry).
/// [`element_text`] (below) merges these into one display string for
/// callers that only need the concatenated text; callers that need to
/// derive *further* spans from the text (e.g. `inject_value`'s P9
/// `${}`/`#{}` extraction) must scan each segment separately instead —
/// concatenating first and then re-deriving offsets into the merged
/// string is exactly the bug this split avoids: a comment or CDATA
/// delimiter sitting between two text runs is real source bytes that
/// never made it into the merged value, so an offset computed against the
/// merged string does not correspond to the same offset in the source.
pub(crate) fn element_text_segments(element: &XmlElement) -> Vec<Spanned<String>> {
    element
        .children
        .iter()
        .filter_map(|child| match child {
            XmlNode::Text(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Concatenates `segments` (in order) into a single string, spanning from
/// the first segment's start to the last one's end. Falls back to
/// `fallback_span` when `segments` is empty, so the result always has
/// *some* valid span rather than a degenerate one. **Not** itself
/// invariant-#4-safe when `segments` has more than one entry with a gap
/// between them (see [`element_text_segments`]'s doc comment) — only a
/// display/summary string, never a basis for further span math.
pub(crate) fn merge_text_segments(
    segments: &[Spanned<String>],
    fallback_span: ByteSpan,
) -> Spanned<String> {
    let mut value = String::new();
    let mut span: Option<ByteSpan> = None;
    for text in segments {
        value.push_str(&text.value);
        span = Some(match span {
            Some(s) => ByteSpan {
                start: s.start.min(text.span.start),
                end: s.end.max(text.span.end),
            },
            None => text.span,
        });
    }
    Spanned {
        value,
        span: span.unwrap_or(fallback_span),
    }
}

/// Concatenates every direct text child of `element` (there is normally
/// exactly one) into a single string, spanning from the first text child's
/// start to the last one's end. Falls back to `element`'s own span when it
/// has no text child at all (e.g. `<description/>` or `<description></description>`),
/// so the result always has *some* valid span rather than a degenerate one.
/// `pub(crate)`: shared with U4's `parse_bean` (`src/bean.rs`) — a
/// `<bean>`'s own `<description>` child reads text the exact same way the
/// `<beans>` header's does. See [`element_text_segments`]'s doc comment
/// for why callers that need to derive further spans out of the text
/// (rather than just display it) must not use this merged form.
pub(crate) fn element_text(element: &XmlElement) -> Spanned<String> {
    merge_text_segments(&element_text_segments(element), element.span)
}

/// Parses one `<beans>` body — the top-level document root, or (P10, SB-14)
/// a nested `<beans profile="...">` block — into a [`BeansFileCtx`]. This is
/// the "recursion unification" re-entry point the build plan requires: any
/// recursive call (nested profiles, driven by [`BeansBodyFrame::step`]'s own
/// `"beans"` handling — see that method's own doc comment) re-enters *this*
/// function rather than reimplementing root-child dispatch a second time.
///
/// Reads the SB-01 header fields directly (`profile`/`default-*`
/// attributes, `<description>` child) — this unit's own scope — then
/// dispatches every remaining child element through
/// [`dispatch_root_child`]. `diagnostics` accumulates into the caller's
/// shared `Vec` (this crate's `parse`/`parse_bytes` never return `Err`;
/// every anomaly lands there instead).
///
/// `depth` is this call's own nesting level — `0` for the document root
/// (`lib.rs`'s one top-level call), `n + 1` for a nested `<beans
/// profile="...">` block re-entering from its enclosing block's own `depth`
/// of `n` ([`BeansBodyFrame::step`]'s own `"beans"` handling, P10). This is
/// the choke point invariant #1 requires: `events::build_tree` deliberately
/// builds the whole tree iteratively on the heap and carries no
/// `DEPTH_LIMIT` of its own (see that module's doc comment —
/// "`DEPTH_LIMIT` applies once a later unit recursively walks this tree"),
/// and *this* function is that later unit for `<beans>`-in-`<beans>`
/// nesting — historically a genuine native call-stack recursion cycle
/// (`parse_beans_body` → `dispatch_root_child` → `parse_nested_beans` →
/// `parse_beans_body`; see this section's own I3 P0 doc comment further
/// down for why it no longer is one), so it needs the same
/// before-any-recursion [`crate::DEPTH_LIMIT`] check every other recursive
/// walker in this crate has (`collection::parse_collection_value`,
/// `inject_value::parse_inner_bean`, `namespaced::build_namespaced_element`).
/// At the limit: a [`DiagCode::NestingLimitExceeded`] diagnostic plus an
/// empty (but still valid) `BeansFileCtx` — the subtree beyond the limit is
/// dropped rather than walked, same "opaque" treatment those other walkers
/// give their own over-limit subtrees.
///
/// **Stack-diet note** (I3 P0 Windows `STATUS_STACK_OVERFLOW` fix):
/// `BeansFileCtx` is ~376 bytes (many `Vec`/`Option<Spanned<_>>` fields) —
/// large enough that, held by value in every frame of the
/// `parse_beans_body` → `dispatch_root_child` → `parse_nested_beans` →
/// `parse_beans_body` recursion cycle, `DEPTH_LIMIT` (256) levels of nested
/// `<beans profile="...">` blocks blew a 256 KiB thread stack well before
/// the guard fired (`tests/i3_hostile_proptest.rs`'s `deep_profile`
/// fixture). Frame-dieting alone (heap-allocating `ctx` as a
/// `Box<BeansFileCtx>`, returning `Box<BeansFileCtx>` rather than
/// `BeansFileCtx` by value, splitting the `NestingLimitExceeded` early-return
/// and the header-attribute reads into `#[inline(never)]` helpers — the same
/// two-part-plus-helpers fix `bean::parse_bean`'s own matching doc comment
/// describes) reduced this cycle's per-level stack cost but, alone, still
/// could not reach a 256 KiB thread budget at `DEPTH_LIMIT` levels on every
/// platform (this crate's own commit history: it "squeaks past" a 256 KiB
/// thread on macOS — fails at 224 KiB — and a Windows MSVC debug build's
/// fatter frames overflow it outright).
///
/// **I3 P0 stack-diet fallback**: this function is now a thin wrapper —
/// same shape as `bean::parse_bean`'s own — around [`BeansBodyFrame`] +
/// [`run_beans_body`], the heap-worklist engine that drives the whole
/// `<beans>`-in-`<beans>` recursion (this block, every nested `<beans
/// profile="...">` reachable through it, however deep) on the heap instead
/// of the real call stack. See that section's own doc comment (just below)
/// for why this is a separate engine from `crate::depth_engine::run` rather
/// than a fourth `Frame` variant there, and [`BeansBodyFrame`]'s own doc
/// comment for how the recursion itself now works. The `NestingLimitExceeded`
/// early-return here (this function's own entry, `depth >= DEPTH_LIMIT`) and
/// the header-attribute/`ctx`-construction helpers below are otherwise
/// unchanged — still `#[inline(never)]`, still exactly what
/// [`BeansBodyFrame::new`] itself calls for its own prologue.
pub(crate) fn parse_beans_body(
    scope: &NsScope,
    element: &XmlElement,
    diagnostics: &mut Vec<Diagnostic>,
    depth: u32,
) -> Box<BeansFileCtx> {
    if depth >= crate::DEPTH_LIMIT {
        return nesting_limit_exceeded_beans_ctx(element, diagnostics);
    }
    let frame = BeansBodyFrame::new(scope, element, depth);
    run_beans_body(vec![frame], diagnostics)
}

/// `Box::new(BeansFileCtx::default())` plus the one field ([`ByteSpan`])
/// known up front — split out of [`parse_beans_body`] purely for
/// stack-diet framing, same `-O0` `Box::new(f())` non-fusion rationale
/// `bean::new_bean_ctx`'s own doc comment gives (confirmed empirically for
/// that sibling case via `otool -tv` disassembly of a debug build).
#[inline(never)]
fn new_beans_file_ctx(span: ByteSpan) -> Box<BeansFileCtx> {
    let mut ctx = Box::new(BeansFileCtx::default());
    ctx.span = span;
    ctx
}

/// [`DiagCode::NestingLimitExceeded`] early-return path for
/// [`parse_beans_body`] — split out purely for stack-diet framing (see that
/// function's own doc comment): `format!`'s own temporaries would otherwise
/// sit in every recursive call's frame even on the (overwhelmingly common)
/// path that never hits the limit.
#[inline(never)]
#[cold]
fn nesting_limit_exceeded_beans_ctx(
    element: &XmlElement,
    diagnostics: &mut Vec<Diagnostic>,
) -> Box<BeansFileCtx> {
    diagnostics.push(Diagnostic {
        code: DiagCode::NestingLimitExceeded,
        span: Some(element.span),
        message: format!(
            "<beans> nesting exceeded {} levels; subtree treated as opaque",
            crate::DEPTH_LIMIT
        ),
    });
    Box::new(BeansFileCtx {
        span: element.span,
        ..Default::default()
    })
}

/// `<beans profile="..." default-*="...">` header attributes — split out of
/// [`parse_beans_body`] purely for stack-diet framing (see that function's
/// own doc comment); no behavior change from when this was inline there.
#[inline(never)]
fn populate_beans_header_attrs(ctx: &mut BeansFileCtx, element: &XmlElement) {
    ctx.profile = find_attr(&element.attrs, "profile").map(spanned_attr);
    ctx.default_lazy_init = find_bool_attr(&element.attrs, "default-lazy-init");
    ctx.default_autowire = find_attr(&element.attrs, "default-autowire").map(spanned_attr);
    ctx.default_init_method = find_attr(&element.attrs, "default-init-method").map(spanned_attr);
    ctx.default_destroy_method =
        find_attr(&element.attrs, "default-destroy-method").map(spanned_attr);
    ctx.default_merge = find_bool_attr(&element.attrs, "default-merge");
    ctx.default_autowire_candidates =
        find_attr(&element.attrs, "default-autowire-candidates").map(spanned_attr);
}

// ---------------------------------------------------------------------
// I3 P0 stack-diet fallback: explicit-stack (heap worklist) iteration for
// what used to be a `parse_beans_body` → `dispatch_root_child` →
// `parse_nested_beans` → `parse_beans_body` native recursion cycle
// (`<beans profile="...">`-in-`<beans>` nesting; `parse_nested_beans` no
// longer exists as a standalone function — see this section's own doc
// comment further down) — see `crate::depth_engine`'s own module doc
// comment for the general shape of this pattern: a frame owns everything
// its own call's now-unwound stack frame used to hold, `step` drives
// per-level scanning, and a finished frame's result flows back to whichever
// frame is now on top (or is returned, once the stack empties).
//
// This is a separate, self-contained heap-worklist engine
// ([`BeansBodyFrame`] + [`run_beans_body`]), not a fourth
// `crate::depth_engine::Frame` variant: `<beans>`-in-`<beans>` nesting and
// bean/property/constructor-arg/collection nesting are two disjoint
// recursive axes — a nested `<beans profile="...">` is only ever reached
// through another `<beans>` body's own children (SB-14), never through a
// `<bean>`'s own children or a collection's own items/entries, so this
// cycle's frames never need to push onto, or receive a delivery from, a
// `bean::BeanFrame`/`collection::ListLikeFrame`/`collection::MapFrame`, and
// vice versa. Folding them into one `Vec<Frame>`/`Completed` pair would only
// add "can never actually happen" match arms (a `BeansBodyFrame` asked to
// deliver a `Box<InjectValue>`, or a `BeanFrame`/`ListLikeFrame`/`MapFrame`
// asked to deliver a `Box<BeansFileCtx>`) that every one of
// `crate::depth_engine::run`'s existing call sites (`bean::parse_bean`,
// `collection::parse_collection_value`) would then have to match on and
// immediately treat as unreachable, for no benefit.
//
// `dispatch_root_child` has no `"beans"` arm at all: [`BeansBodyFrame::step`]
// intercepts a `"beans"` child *before* calling `dispatch_root_child`, the
// same "intercept ahead of the frozen match" treatment
// `bean::BeanFrame::step`'s own doc comment documents for its
// `"property"`/`"constructor-arg"` interception ahead of
// `dispatch_bean_child` — the P10 unit's own real work (re-entering this
// same `BeansBodyFrame`/`parse_beans_body` shape, profile-expression
// capture, sibling-profile override) lives directly in
// [`BeansBodyFrame::step`]'s own `"beans"` handling now, not behind a
// `parse_nested_beans`-style stub function. Every other root-child shape
// (SB-01's own `<description>`, `<alias>`/`<import>`/`<bean>`/
// `context:component-scan`/`context:property-placeholder`/
// `util:properties`/`NamespacedElement`) is bounded, non-recursive work that
// was never part of the unbounded recursion — it stays exactly as it was,
// still reached through `dispatch_root_child`'s own frozen match, called
// directly (not through this engine) from [`BeansBodyFrame::step`].
// ---------------------------------------------------------------------

/// One in-progress `parse_beans_body` call, suspended on the heap instead of
/// the real call stack — see this section's own doc comment. Mirrors
/// [`crate::bean::BeanFrame`]'s shape (a `children`/`idx` cursor over one
/// element's own children, driven by `step`) but needs no "waiting" slot:
/// unlike a `<property>`/`<constructor-arg>`'s deferred value (which needs
/// its own name/attributes/meta stashed until the value comes back — see
/// `bean::PendingBeanValue`), a nested `<beans>`'s finished result needs
/// nothing beyond `self.ctx` itself to fold into once it comes back
/// ([`Self::deliver`]) — so there is nothing to remember about *which*
/// nested `<beans>` is currently pushed.
struct BeansBodyFrame<'a> {
    ctx: Box<BeansFileCtx>,
    /// The scope in effect for `element`'s own children — i.e. already
    /// overlaid with `element`'s own `xmlns`/`xmlns:*` declarations, exactly
    /// the `scope` [`parse_beans_body`] itself used to receive and pass
    /// straight through to `dispatch_root_child` with no further overlay
    /// (both call sites — `lib.rs`'s top-level call, [`Self::step`]'s own
    /// `"beans"` handling below — already overlay before calling in).
    /// Stored owned (`NsScope` derives `Clone`) since it must outlive this
    /// whole frame's own children loop, not just one `step` call.
    scope: NsScope,
    children: &'a [XmlNode],
    idx: usize,
    depth: u32,
}

/// [`BeansBodyFrame::step`]'s own step result — same "advance in place /
/// descend / return" framing as [`crate::depth_engine::Advance`], specialized
/// to `Box<BeansFileCtx>` (this cycle never produces or consumes an
/// `InjectValue`) — see this section's own doc comment for why that's a
/// separate type rather than reusing `crate::depth_engine::Advance` itself.
enum BeansAdvance<'a> {
    /// Descend: push `frame` and re-enter [`run_beans_body`]'s own loop with
    /// it on top — the frame underneath (which requested this) is left
    /// exactly as it was; it only resumes once the pushed frame eventually
    /// finishes (see [`BeansBodyFrame::deliver`]).
    Push(Box<BeansBodyFrame<'a>>),
    /// Made progress without changing the stack's shape (processed a
    /// non-recursive root child, or immediately folded an over-limit nested
    /// `<beans>`'s downgraded stub into `self.ctx` without ever pushing a
    /// frame for it) — call `step`/`deliver` again on the same (still-top)
    /// frame.
    Continue,
    /// Return: this frame has nothing left to do. [`run_beans_body`] pops
    /// it, converts it via [`BeansBodyFrame::finish`], and delivers the
    /// result to whatever is now on top (or returns it, if the stack is now
    /// empty).
    Finished,
}

impl<'a> BeansBodyFrame<'a> {
    /// Starts a new frame for `element` — the exact prologue
    /// `parse_beans_body` ran inline before its own children loop (SB-01
    /// header attributes), unchanged. `depth` must already be known to be
    /// `< crate::DEPTH_LIMIT` — both call sites ([`parse_beans_body`]'s own
    /// wrapper, and [`Self::step`]'s own `"beans"` interception below) check
    /// that themselves *before* constructing a frame, exactly mirroring
    /// `inject_value::begin_resolve_value`'s "check before push, never push
    /// past the limit" convention — so this constructor itself never needs
    /// to check or downgrade.
    fn new(scope: &NsScope, element: &'a XmlElement, depth: u32) -> Self {
        debug_assert!(depth < crate::DEPTH_LIMIT);
        let mut ctx = new_beans_file_ctx(element.span);
        populate_beans_header_attrs(&mut ctx, element);
        BeansBodyFrame {
            ctx,
            scope: scope.clone(),
            children: &element.children,
            idx: 0,
            depth,
        }
    }

    /// Advances this frame by one step: either makes local progress
    /// (`BeansAdvance::Continue`, implicitly, by looping again below),
    /// finishes (`BeansAdvance::Finished`), or defers a nested `<beans>`
    /// child onto the stack (`BeansAdvance::Push`). Never called while a
    /// push it issued hasn't yet been resolved — that case only ever
    /// resumes via [`Self::deliver`].
    fn step(&mut self, diagnostics: &mut Vec<Diagnostic>) -> BeansAdvance<'a> {
        loop {
            let Some(child) = self.children.get(self.idx) else {
                return BeansAdvance::Finished;
            };
            self.idx += 1;
            let XmlNode::Element(child_element) = child else {
                // Top-level text (whitespace between elements, typically)
                // has nothing to attach to at this level and is dropped,
                // same as `events::build_tree`'s own out-of-element text
                // handling — matches `parse_beans_body`'s former inline
                // children loop exactly.
                continue;
            };
            // Resolve this child's own qname *before* falling through to
            // `dispatch_root_child`, so a `"beans"` child never reaches
            // that match at all (it has no arm for one — see
            // `dispatch_root_child`'s own doc comment).
            let child_scope = NsScope::from_element(child_element, Some(&self.scope));
            let qn = resolve_qname(&child_element.name, &child_scope);
            if qn.1 == "beans" && is_beans_ns(&qn.0) {
                // Unit P10 (nested `<beans profile="...">`, SB-14) —
                // `BeansFileCtx::nested_profiles`. Re-enters this same
                // `BeansBodyFrame`/[`parse_beans_body`] shape (build plan
                // "recursion unification") rather than reimplementing
                // root-child dispatch — never calls `dispatch_root_child`
                // or duplicates its match. That re-entry is also what
                // makes this unit's two owned pieces come for free instead
                // of needing bespoke code:
                //
                // - Profile-expression capture: [`BeansBodyFrame::new`]
                //   already reads `profile=` into `ctx.profile` as a plain
                //   `Spanned<String>` (SB-01, this unit's own
                //   header-attribute handling, unchanged for the nested
                //   case) — the raw text is never parsed as boolean logic,
                //   so multi-value (`"dev,test"`), negation (`"!prod"`),
                //   and full boolean-expression forms (`"(dev & !prod) |
                //   qa"`, whatever Spring's `Profiles.of` grammar accepts)
                //   all land verbatim in `nested_profiles[i].profile.value`
                //   unchanged — evaluating/parsing that expression is a
                //   consumer's job (spec's "SpEL/`${}` **evaluation**
                //   (collection only)" non-goal, same policy applied to
                //   profile expressions).
                // - Sibling-profile override, not `DuplicateBeanId`: that
                //   check lives in `dispatch_root_child`'s `"bean"` arm and
                //   only ever compares a new `<bean>` against *its own*
                //   `ctx.beans`. Each nested `<beans profile>` block gets a
                //   fresh `BeansFileCtx` ([`BeansBodyFrame::new`] always
                //   starts from `BeansFileCtx::default()`) — so two sibling
                //   nested blocks (e.g. `<beans profile="dev">` and a
                //   second, later `<beans profile="dev">` at the same
                //   nesting level, or `<beans profile="dev">`/`<beans
                //   profile="test">` each defining a bean with the same id)
                //   never share a `ctx.beans` to compare against each
                //   other. The same id appearing in two sibling profile
                //   blocks is therefore never flagged `DuplicateBeanId` —
                //   each block's `Bean` is independently preserved in its
                //   own `nested_profiles[i].beans`, and a consumer picking
                //   one active profile's block treats the later match as
                //   the effective (override) definition. Nothing extra
                //   needs implementing here for that to hold — it falls
                //   out of "each recursive call gets an independent ctx" by
                //   construction, not a special case this branch writes.
                //
                // `depth + 1` is this nested block's own depth, checked
                // *before* recursing (never after) — same convention
                // `inject_value::begin_resolve_value`'s own `Bean`/
                // `Collection` arms follow ahead of every `Advance::Push`.
                if self.depth + 1 >= crate::DEPTH_LIMIT {
                    let nested_ctx = nesting_limit_exceeded_beans_ctx(child_element, diagnostics);
                    push_nested_beans_file(&mut self.ctx, nested_ctx);
                } else {
                    let frame = BeansBodyFrame::new(&child_scope, child_element, self.depth + 1);
                    return BeansAdvance::Push(Box::new(frame));
                }
                continue;
            }
            // Every other root-child shape is bounded, non-recursive work
            // — reuse the frozen dispatch match unchanged.
            dispatch_root_child(&mut self.ctx, diagnostics, &self.scope, child_element);
        }
    }

    /// Resumes this frame once a nested `<beans>` child it pushed has
    /// finished resolving — folds the result into `self.ctx.nested_profiles`
    /// via [`push_nested_beans_file`] (the same helper [`Self::step`]'s own
    /// `"beans"` handling calls for the over-limit, synchronous case) and
    /// hands control back to [`Self::step`] to continue this block's own
    /// children loop.
    fn deliver(&mut self, nested_ctx: Box<BeansFileCtx>) -> BeansAdvance<'a> {
        push_nested_beans_file(&mut self.ctx, nested_ctx);
        BeansAdvance::Continue
    }

    /// Consumes this finished frame into its assembled `Box<BeansFileCtx>`
    /// — only ever called once `step`/`deliver` has returned
    /// `BeansAdvance::Finished` for it.
    fn finish(self) -> Box<BeansFileCtx> {
        self.ctx
    }
}

/// Drives `stack` to completion — see this section's own doc comment for the
/// full step/descend/return framing, and [`crate::depth_engine::run`]'s own
/// doc comment for the general pattern this mirrors. `stack` must start with
/// exactly one frame (the call this whole engine run is standing in for);
/// every further frame is pushed/popped internally as nested `<beans>`
/// resolution demands.
fn run_beans_body(
    mut stack: Vec<BeansBodyFrame<'_>>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Box<BeansFileCtx> {
    debug_assert!(!stack.is_empty(), "engine started with no initial frame");
    let mut incoming: Option<Box<BeansFileCtx>> = None;
    loop {
        let top = stack
            .last_mut()
            .expect("engine stack emptied while still running");
        let advance = match incoming.take() {
            Some(nested_ctx) => top.deliver(nested_ctx),
            None => top.step(diagnostics),
        };
        match advance {
            BeansAdvance::Push(frame) => stack.push(*frame),
            BeansAdvance::Continue => {}
            BeansAdvance::Finished => {
                let frame = stack.pop().expect("the frame that just advanced");
                let finished_ctx = frame.finish();
                if stack.is_empty() {
                    return finished_ctx;
                }
                incoming = Some(finished_ctx);
            }
        }
    }
}

// ---------------------------------------------------------------------
// The frozen root-child dispatch match (build plan "dispatch contract").
// ---------------------------------------------------------------------

/// One `<beans>`-body child element → its handler. **Frozen structure**:
/// the 1st-class allowlist (`context:component-scan`/
/// `context:property-placeholder`, `util:properties`, plus `<bean>` itself)
/// is enumerated explicitly; the
/// [`NamespacedElement`](crate::model::NamespacedElement) catch-all
/// (`parse_namespaced`) is the **last** arm, so anything not explicitly
/// claimed above it — including every other `context:*`/`util:*` element
/// (`context:annotation-config`, `util:list`, ...) and every other namespace
/// entirely (`aop:*`, `tx:*`, `jee:*`, ...) — falls through to it. A leaf
/// unit (P1/P3/P4/P5/P10/P7) fills exactly one handler function's body;
/// none of them ever needs to touch this match.
///
/// `<beans>` is **not** an arm of this match: it is intercepted by
/// [`BeansBodyFrame::step`] before this function is ever called for it (see
/// this section's own doc comment) — so this match only ever sees every
/// *other* root-child shape, none of which need `depth`, which is
/// consequently not one of this function's own parameters either (unlike
/// [`BeansBodyFrame`], which still threads it through to
/// [`BeansBodyFrame::new`]'s own recursive descent).
fn dispatch_root_child(
    ctx: &mut BeansFileCtx,
    diagnostics: &mut Vec<Diagnostic>,
    scope: &NsScope,
    child: &XmlElement,
) {
    // A child can declare its own `xmlns`/`xmlns:*` right on itself (legal
    // and common, e.g. a per-element prefix instead of declaring it on the
    // root) — per standard XML namespace scoping, such a declaration
    // applies to the element it's on, not just its descendants. Resolving
    // the child's own tag name against the *parent* scope alone (skipping
    // this overlay) would disagree with `is_beans_root`, which does overlay
    // the root element's own declarations before resolving the root's own
    // name (see that function's doc comment) — so the same rule is applied
    // here for consistency.
    let child_scope = NsScope::from_element(child, Some(scope));
    // Kept as one `qn: (String, String)` binding rather than destructured
    // `let (ns, local) = ..` — stack-diet micro-optimization: at `-O0`,
    // destructuring a tuple rvalue into two separately-named locals
    // generates an extra move/copy into those locals *on top of* the
    // tuple's own temporary (confirmed empirically via MIR dump —
    // `rustc -Z unpretty=mir` — showing both the tuple local and the two
    // destructured locals coexisting), whereas field projection
    // (`qn.0`/`qn.1`) reads directly out of the one temporary that's
    // already there. Every other site in this crate with the same shape
    // (`bean::dispatch_bean_child`, `collection::child_is_map_entry`/
    // `child_is_map_key`, `inject_value::classify_inject_value_child`)
    // points back to this comment rather than repeating it.
    let qn = resolve_qname(&child.name, &child_scope);
    match qn.1.as_str() {
        "description" if is_beans_ns(&qn.0) => {
            // SB-01 core (this unit), not a leaf stub: the header's own
            // `<description>` child element.
            ctx.description = Some(element_text(child));
        }
        "import" if is_beans_ns(&qn.0) => parse_import(ctx, diagnostics, scope, child),
        "alias" if is_beans_ns(&qn.0) => parse_alias(ctx, diagnostics, scope, child),
        // U4's real wiring: `<bean>` parsing itself (and the bean-child
        // dispatch skeleton around it) is entirely U4's contract
        // (`crate::bean::parse_bean`, build plan U4 row). Factored into
        // `dispatch_bean_element` below (`#[inline(never)]`) rather than
        // inline here purely for stack-diet framing (I3 P0 fix, see
        // `bean::parse_bean`'s own doc comment for the full rationale): its
        // `DuplicateBeanId` bookkeeping has real locals of its own
        // (`format!`'s own temporaries in particular), and at `-O0` an
        // unoptimized frame reserves stack for every local declared
        // anywhere in a function regardless of which `match` arm actually
        // ran — leaving this inline would bloat *every* call to
        // `dispatch_root_child`, for no benefit.
        "bean" if is_beans_ns(&qn.0) => dispatch_bean_element(ctx, diagnostics, scope, child),
        // `<beans>` is **not** an arm of this match: a nested `<beans>`
        // child is intercepted by `BeansBodyFrame::step` (this section's
        // own heap-worklist engine, defined above `dispatch_root_child`)
        // before this function is ever called for it — see that method's
        // own doc comment, and this section's own doc comment for the full
        // recursion-engine picture.
        "component-scan" if is_context_ns(&qn.0) => {
            parse_component_scan(ctx, diagnostics, scope, child)
        }
        "property-placeholder" if is_context_ns(&qn.0) => {
            parse_property_source(ctx, diagnostics, scope, child)
        }
        "properties" if is_util_ns(&qn.0) => parse_property_source(ctx, diagnostics, scope, child),
        // An element inside the first-class `beans` namespace itself that
        // isn't one of the recognized names above (a typo, a future
        // element this build doesn't know yet, ...) — `UnknownElement`,
        // per that `DiagCode` variant's own doc comment distinguishing it
        // from an out-of-scope *namespace* (which goes to
        // `NamespacedElement` instead, never here).
        _ if is_beans_ns(&qn.0) => push_unknown_element(diagnostics, child),
        // NamespacedElement catch-all — pinned LAST, per the dispatch
        // contract: every other `context:*`/`util:*` element and every
        // other namespace entirely lands here.
        _ => parse_namespaced(ctx, diagnostics, scope, child),
    }
}

/// The `"bean"` arm's real body (build plan U4 row: `crate::bean::parse_bean`)
/// plus the one policy that belongs at *this* level, not `parse_bean`'s:
/// `DuplicateBeanId` is scoped to a single `<beans>` block (this
/// `ctx.beans`, spec's `DuplicateBeanId` doc comment), which `parse_bean`
/// itself has no visibility into — it only ever sees one `<bean>` element
/// at a time, never its siblings. Split out of `dispatch_root_child`'s
/// match purely for stack-diet framing — see that match's own `"bean"` arm
/// comment.
#[inline(never)]
fn dispatch_bean_element(
    ctx: &mut BeansFileCtx,
    diagnostics: &mut Vec<Diagnostic>,
    scope: &NsScope,
    child: &XmlElement,
) {
    let bean = crate::bean::parse_bean(scope, child, diagnostics, 0);
    if let Some(id) = bean.id.as_ref() {
        let is_duplicate = ctx.beans.iter().any(|existing| {
            existing.id.as_ref().map(|e| e.value.as_str()) == Some(id.value.as_str())
        });
        if is_duplicate {
            diagnostics.push(Diagnostic {
                code: DiagCode::DuplicateBeanId,
                span: Some(id.span),
                message: format!("duplicate bean id '{}' within this <beans> block", id.value),
            });
        }
    }
    // Both beans are preserved either way (spec's edge-case table:
    // "duplicate id (both kept + diagnostic)") — the diagnostic is additive, never a
    // reason to drop one.
    ctx.beans.push(*bean);
}

/// `UnknownElement` diagnostic push for an unrecognized element inside the
/// first-class `beans` namespace itself — split out of `dispatch_root_child`'s
/// match purely for stack-diet framing (its `format!` call has its own
/// temporaries that would otherwise sit in `dispatch_root_child`'s own
/// frame on every call, per that function's `"bean"` arm comment).
#[inline(never)]
fn push_unknown_element(diagnostics: &mut Vec<Diagnostic>, child: &XmlElement) {
    diagnostics.push(Diagnostic {
        code: DiagCode::UnknownElement,
        span: Some(child.span),
        message: format!(
            "unrecognized element <{}> inside the beans namespace",
            child.name
        ),
    });
}

// ---------------------------------------------------------------------
// Per-element handler fns (build plan "dispatch contract").
//
// All six leaf units (P1/P3/P4/P5/P7/P10) have now landed their own real
// body here — none of them is a no-op stub any more. Per the dispatch
// contract, each leaf touches only its own handler function below — never
// `dispatch_root_child`'s match above. The frozen handler signature
// (`ctx: &mut BeansFileCtx, diagnostics: &mut Vec<Diagnostic>, scope:
// &NsScope, element: &XmlElement`, plus `depth: u32` for the two recursive
// ones) is unchanged from the original stub scaffolding — a leaf fills the
// body, never the signature. A handler that ends up not needing
// `diagnostics`/`scope` keeps `#[allow(clippy::ptr_arg)]` on `diagnostics`
// (`&mut Vec<Diagnostic>` — a slice can't `push`, so clippy's "a slice
// would do" suggestion doesn't apply here, but it can't tell that from an
// unread param) without also silencing `unused_variables` on a param it
// does read.
// ---------------------------------------------------------------------

/// Unit **P1** (`<alias name= alias=>`, SB-03) — `BeansFileCtx::aliases`.
///
/// `name=` is the existing bean's registered name this `<alias>` declares
/// a second name for, and `alias=` is that new name — both read raw
/// (spec's "references are raw only" policy) as plain `Spanned<String>`, never a
/// `BeanRef`: unlike `ref=`/`parent=`/`factory-bean=`, `<alias name=>` is
/// not itself one of `RefKind`'s three cases (spec's `BeanRef`/`RefKind`
/// doc — alias targets are folded into the id/name/alias union a consumer
/// matches a `ref=` string against, per the spec's "name index" consumer
/// note, not carried as a `BeanRef` here). The target bean named by
/// `name=` need not be defined in this same file — spec's SB-03 edge case
/// "reference to a bean in another file" (a `<beans>` assembled via `<import>` from several
/// files may alias a bean this file never defines) — so, same as every
/// other raw reference in this crate, no existence check is attempted;
/// resolving it is a consumer's job.
///
/// Either attribute absent falls back to an empty spanned string at
/// `element`'s own span — same "infallible fallback, no diagnostic
/// invented for an untested edge shape" policy `parse_import` documents
/// for its own missing `resource=` (this crate never panics/errors on a
/// malformed-against-the-XSD `<alias>`, rule 4). Unlike `BeanRef.raw`/
/// `ClassRef.raw` (invariant #5), `Alias.name`/`Alias.alias` are plain
/// strings with no "never empty" contract to uphold, so no diagnostic is
/// pushed for the empty-fallback case either — same treatment
/// `resolve_scope` gives a present-but-empty `scope=`.
///
/// `scope`/`diagnostics` are unused: `<alias>` has no children to recurse
/// into and no anomaly this unit's edge-case table calls for diagnosing.
#[allow(unused_variables, clippy::ptr_arg)]
pub(crate) fn parse_alias(
    ctx: &mut BeansFileCtx,
    diagnostics: &mut Vec<Diagnostic>,
    scope: &NsScope,
    element: &XmlElement,
) {
    let name = find_attr(&element.attrs, "name")
        .map(spanned_attr)
        .unwrap_or_else(|| Spanned {
            value: String::new(),
            span: element.span,
        });
    let alias = find_attr(&element.attrs, "alias")
        .map(spanned_attr)
        .unwrap_or_else(|| Spanned {
            value: String::new(),
            span: element.span,
        });
    ctx.aliases.push(Spanned {
        value: Alias { name, alias },
        span: element.span,
    });
}

/// Unit **P3** (`<import resource=...>`, SB-09) — `BeansFileCtx::imports`.
///
/// `resource=` is read raw and classified into an [`ImportKind`] by prefix
/// shape only — no `${}`/`#{}` evaluation (spec's "SpEL/`${}` **evaluation** (collection only)"
/// non-goal): a placeholder anywhere in the string (`classpath:${env}/ctx.xml`,
/// `${config.dir}/services.xml`, ...) is preserved verbatim in `resource.value`
/// and plays no part in classification — only the resource string's own
/// literal prefix does. See [`classify_import_kind`] for the prefix rules
/// this delegates to (`classpath:`/`classpath*:` before any generic URL-scheme
/// check, so `classpath*:` never misclassifies as a `Url` scheme match).
///
/// `resource=` absent (malformed against the Spring XSD, which requires it)
/// falls back to an empty spanned string at `element`'s own span — same
/// "infallible fallback, no diagnostic invented for an untested edge shape"
/// policy `property::resolve_property_name` documents for its own missing
/// `name=` — `classify_import_kind` then reads that empty string as
/// `ImportKind::Other`.
/// `scope`/`diagnostics` are unused: `<import>` has no children to recurse
/// into and no anomaly this unit's edge-case table calls for diagnosing.
#[allow(unused_variables, clippy::ptr_arg)]
pub(crate) fn parse_import(
    ctx: &mut BeansFileCtx,
    diagnostics: &mut Vec<Diagnostic>,
    scope: &NsScope,
    element: &XmlElement,
) {
    let resource = find_attr(&element.attrs, "resource")
        .map(spanned_attr)
        .unwrap_or_else(|| Spanned {
            value: String::new(),
            span: element.span,
        });
    let kind = classify_import_kind(&resource.value);
    ctx.imports.push(Spanned {
        value: Import { resource, kind },
        span: element.span,
    });
}

/// Classifies an `<import resource=...>` value into an [`ImportKind`] by
/// literal prefix shape only (spec's "raw only" policy — no placeholder
/// evaluation). Order matters between the `classpath:` prefix check and
/// the generic URL-scheme check below: `classpath:` alone is itself a
/// syntactically valid URI scheme (an ASCII letter followed by letters/
/// digits/`+`/`-`/`.` then `:`) and would be swallowed by
/// [`has_url_scheme`] as a generic `Url` if checked after it. `classpath*:`
/// needs no such ordering safeguard against [`has_url_scheme`] — `*` is
/// not a legal URI scheme character, so `has_url_scheme("classpath*:...")`
/// already returns `false` on its own regardless of check order; it still
/// gets its own dedicated prefix check here because it maps to a distinct
/// `ImportKind` (`ClasspathStar`, not `Classpath`).
///
/// `Url` covers every other URL scheme, per `ImportKind::Url`'s own doc
/// comment — `file:...`, `http://...`, `jar:file:...`, and so on all share
/// the one generic [`has_url_scheme`] check rather than each getting a
/// bespoke prefix test.
///
/// An empty resource string (including the "attribute absent" fallback
/// above) is the one shape this classifies as `ImportKind::Other` — it
/// matches none of the recognized shapes, per that variant's own "total
/// fallback" doc comment. Everything else non-empty that isn't `classpath`/
/// `classpath*`/URL-shaped is `Relative` — a bare filesystem-style path
/// (`services.xml`, `../config/other.xml`, `${config.dir}/services.xml`,
/// an absolute `/etc/app/services.xml`, ...), which is the common case for
/// a same-directory or relative sibling import.
fn classify_import_kind(resource: &str) -> ImportKind {
    if resource.is_empty() {
        ImportKind::Other
    } else if let Some(rest) = resource.strip_prefix("classpath*:") {
        let _ = rest; // prefix match only — the remainder is kept raw in `resource`.
        ImportKind::ClasspathStar
    } else if resource.starts_with("classpath:") {
        ImportKind::Classpath
    } else if has_url_scheme(resource) {
        ImportKind::Url
    } else {
        ImportKind::Relative
    }
}

/// `true` when `resource` starts with a syntactically valid URI scheme
/// (RFC 3986 `scheme = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"`) —
/// an ASCII letter, then zero or more ASCII letters/digits/`+`/`-`/`.`,
/// then a `:`. Generic on purpose: this crate doesn't enumerate every URL
/// scheme Spring's `ResourceLoader` might resolve (`file:`, `http:`,
/// `https:`, `jar:`, `ftp:`, ...) — any string shaped like `scheme:...`
/// that isn't already `classpath:`/`classpath*:` (checked first by
/// [`classify_import_kind`], the only caller) is a `Url` by this same
/// generic rule, per `ImportKind::Url`'s own doc comment ("covers every
/// URL scheme").
fn has_url_scheme(resource: &str) -> bool {
    let mut chars = resource.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    for c in chars {
        if c == ':' {
            return true;
        }
        if !(c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
            return false;
        }
    }
    false
}

/// Unit **P4** (`context:component-scan`, SB-10) —
/// `BeansFileCtx::component_scans`.
///
/// `base-package=` is read raw and split on `,`/`;`/whitespace via the
/// shared [`split_name_tokens`] (`bean.rs`) — the exact delimiter set
/// Spring's own `ComponentScanBeanDefinitionParser` tokenizes this
/// attribute against, so multiple packages (`base-package="com.example.a,
/// com.example.b"`) land as one `Spanned<String>` per package, each with
/// its own absolute span. Attribute absent (malformed against the XSD,
/// which requires it) falls back to an empty `Vec` — same "infallible,
/// no diagnostic invented for an untested edge shape" policy `parse_import`
/// documents for its own missing `resource=`.
///
/// `use-default-filters="false"` is read via the shared
/// [`find_bool_attr`] — `Some(false)` for the literal `"false"`,
/// `Some(true)` for `"true"`, `None` for absent (Spring's own default is
/// `true`, but that default is a consumer's concern, not this parser's —
/// same "preserve, don't resolve" policy `default_lazy_init`/friends
/// already follow for the `<beans>` header's own `default-*` attributes).
///
/// Include/exclude filters are this element's own `context:include-filter`/
/// `context:exclude-filter` children (all four `type=` shapes — `annotation`/
/// `assignable`/`regex`/`aspectj` — read raw into `ScanFilter::filter_type`,
/// never validated against that closed set: a `type="custom"` or a typo is
/// preserved verbatim rather than rejected, same "closed enum only where the
/// spec says raw" policy applied everywhere else in this crate). A child
/// outside the `context` namespace, or a `context:*` child that isn't one of
/// those two names, is silently skipped here — it's not this element's own
/// dispatch responsibility to diagnose (an `UnknownElement`/`NamespacedElement`
/// classification only applies at the root-/bean-child dispatch level, per
/// those `DiagCode` variants' own doc comments; a component-scan's *own*
/// children are a narrower, non-recursive-dispatch shape this unit owns
/// outright).
#[allow(clippy::ptr_arg)]
pub(crate) fn parse_component_scan(
    ctx: &mut BeansFileCtx,
    _diagnostics: &mut Vec<Diagnostic>,
    scope: &NsScope,
    element: &XmlElement,
) {
    let base_packages = find_attr(&element.attrs, "base-package")
        .map(|attr| split_name_tokens(&attr.value.value, attr.value.span))
        .unwrap_or_default();
    let use_default_filters = find_bool_attr(&element.attrs, "use-default-filters");

    let scan_scope = NsScope::from_element(element, Some(scope));
    let mut include_filters = Vec::new();
    let mut exclude_filters = Vec::new();
    for child in &element.children {
        let XmlNode::Element(child_element) = child else {
            continue;
        };
        // A filter child can declare its own `xmlns`/`xmlns:*` right on
        // itself (legal and common — e.g. a per-element prefix instead of
        // relying on the component-scan element's declarations), so the
        // overlay has to happen per child, not once for the whole loop:
        // resolving `child_element`'s name against `scan_scope` alone would
        // miss such a self-declared namespace. Same rule `dispatch_root_child`
        // applies one level up (see that function's doc comment) — applied
        // here for consistency.
        let child_scope = NsScope::from_element(child_element, Some(&scan_scope));
        let (ns, local) = resolve_qname(&child_element.name, &child_scope);
        if !is_context_ns(&ns) {
            continue;
        }
        match local.as_str() {
            "include-filter" => include_filters.push(scan_filter_from_element(child_element)),
            "exclude-filter" => exclude_filters.push(scan_filter_from_element(child_element)),
            _ => {}
        }
    }

    ctx.component_scans.push(Spanned {
        value: ComponentScan {
            base_packages,
            use_default_filters,
            include_filters,
            exclude_filters,
        },
        span: element.span,
    });
}

/// Builds one [`ScanFilter`] from a `<context:include-filter>`/
/// `<context:exclude-filter>` element — `type=`/`expression=` read raw,
/// each falling back to an empty spanned string at `element`'s own span
/// when absent (malformed against the XSD, which requires both), same
/// infallible-fallback policy [`parse_alias`] documents for its own two
/// attributes.
fn scan_filter_from_element(element: &XmlElement) -> ScanFilter {
    let filter_type = find_attr(&element.attrs, "type")
        .map(spanned_attr)
        .unwrap_or_else(|| Spanned {
            value: String::new(),
            span: element.span,
        });
    let expression = find_attr(&element.attrs, "expression")
        .map(spanned_attr)
        .unwrap_or_else(|| Spanned {
            value: String::new(),
            span: element.span,
        });
    ScanFilter {
        filter_type,
        expression,
    }
}

/// Unit **P5** (`context:property-placeholder` / `util:properties`,
/// SB-11) — `BeansFileCtx::property_sources`.
///
/// One handler for both element shapes (the dispatch match routes both
/// here, `dispatch_root_child`'s `"property-placeholder"`/`"properties"`
/// arms), distinguished by `element.name`'s local part — never by
/// namespace alone, since `is_util_ns`/`is_context_ns` don't distinguish
/// *which* element within a namespace, only membership in it:
///
/// - `context:property-placeholder` → [`PropertySource::Placeholder`]:
///   `location=` is a comma-delimited list of resource paths (Spring's own
///   `PropertyPlaceholderBeanDefinitionParser` reads it via
///   `StringUtils.commaDelimitedListToStringArray` — comma only, unlike
///   `base-package`'s wider delimiter set above) — split here via
///   [`split_comma_list`], each token trimmed of surrounding whitespace and
///   carrying its own absolute span. Attribute absent falls back to an
///   empty `Vec` (same infallible-fallback policy as everywhere else in
///   this crate).
/// - `util:properties` → [`PropertySource::Properties`]: `id=` (when
///   present) registers a bean — spec's own doc comment on this variant —
///   and `location=` is the single resource path; both are plain
///   `Option<Spanned<String>>`, `None` when the attribute is absent (no
///   empty-string fallback needed here, since both fields are already
///   `Option` in the frozen model rather than a bare `Spanned<String>`).
///
/// Bean-declared `PropertyPlaceholderConfigurer`/`PropertiesFactoryBean`
/// (`<bean class="org.springframework.beans.factory.config.
/// PropertyPlaceholderConfigurer">`) is deliberately **not** special-cased
/// here or anywhere else — spec's SB-11 edge case "declarative
/// `PropertyPlaceholderConfigurer` stays on the plain `Bean` path": it
/// never reaches this function at all, since `dispatch_root_child`'s
/// `"bean"` arm (U4's territory) routes every `<bean>` through
/// `bean::parse_bean` regardless of its `class=` value — this crate does
/// no FQN-based special-casing of ordinary bean declarations.
#[allow(clippy::ptr_arg)]
pub(crate) fn parse_property_source(
    ctx: &mut BeansFileCtx,
    _diagnostics: &mut Vec<Diagnostic>,
    _scope: &NsScope,
    element: &XmlElement,
) {
    let (_, local) = split_qname(&element.name);
    let value = if local == "properties" {
        let id = find_attr(&element.attrs, "id").map(spanned_attr);
        let location = find_attr(&element.attrs, "location").map(spanned_attr);
        PropertySource::Properties { id, location }
    } else {
        let locations = find_attr(&element.attrs, "location")
            .map(|attr| split_comma_list(&attr.value.value, attr.value.span))
            .unwrap_or_default();
        PropertySource::Placeholder { locations }
    };
    ctx.property_sources.push(Spanned {
        value,
        span: element.span,
    });
}

/// Splits `text` on `,` only (unlike [`split_name_tokens`]'s wider
/// comma/semicolon/whitespace delimiter set) into tokens, each trimmed of
/// surrounding ASCII whitespace and carrying its own absolute span —
/// `context:property-placeholder`'s `location=` follows Spring's own
/// `StringUtils.commaDelimitedListToStringArray` convention (comma-only),
/// distinct from `base-package`'s wider tokenizer
/// ([`parse_component_scan`]'s own doc comment). A run of consecutive
/// commas, or leading/trailing whitespace around a token, never produces an
/// empty token — same "no empty tokens" policy `split_name_tokens` follows.
/// ASCII-only trimming/splitting is safe on non-ASCII path segments for the
/// same reason `split_name_tokens` documents: `,` and ASCII whitespace
/// never occur as a continuation or lead byte of a multi-byte UTF-8
/// sequence.
fn split_comma_list(text: &str, span: ByteSpan) -> Vec<Spanned<String>> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i <= bytes.len() {
        if i == bytes.len() || bytes[i] == b',' {
            let mut s = start;
            let mut e = i;
            while s < e && bytes[s].is_ascii_whitespace() {
                s += 1;
            }
            while e > s && bytes[e - 1].is_ascii_whitespace() {
                e -= 1;
            }
            if e > s {
                tokens.push(Spanned {
                    value: text[s..e].to_string(),
                    span: ByteSpan {
                        start: span.start + s as u32,
                        end: span.start + e as u32,
                    },
                });
            }
            start = i + 1;
        }
        i += 1;
    }
    tokens
}

/// `ctx.nested_profiles.push(nested_ctx.into_beans_file())` — the P10 unit's
/// (nested `<beans profile="...">`, SB-14) final assembly step, called from
/// both of [`BeansBodyFrame`]'s own recursive-descent sites: synchronously
/// in [`BeansBodyFrame::step`]'s own `"beans"` handling (see that method's
/// own doc comment for the full P10 rationale — profile-expression capture,
/// sibling-profile override) when the nesting limit is already exceeded,
/// and from [`BeansBodyFrame::deliver`] once a nested block pushed onto the
/// engine's own stack has finished resolving. Split out purely for
/// stack-diet framing: `into_beans_file()` returns a `BeansFile` by value
/// (~376 bytes) that has to materialize somewhere before the `push`, and
/// this is on the `<beans>`-in-`<beans>` recursive chain
/// (`tests/i3_hostile_proptest.rs`'s `deep_profile` fixture) — same "give a
/// large sequential construction its own transient frame" rationale
/// `bean::finish_bean`'s doc comment gives.
#[inline(never)]
fn push_nested_beans_file(ctx: &mut BeansFileCtx, nested_ctx: Box<BeansFileCtx>) {
    ctx.nested_profiles.push(nested_ctx.into_beans_file());
}

/// Unit **P7** (`NamespacedElement` + allowlisted ref recursion, SB-02c) —
/// `BeansFileCtx::namespaced`. The `NamespacedElement` catch-all arm in
/// `dispatch_root_child` above.
///
/// `scope` is the PARENT scope — the one in effect for `element`'s parent,
/// *before* overlaying whatever `xmlns`/`xmlns:*` declarations `element`
/// itself carries — [`crate::namespaced::build_namespaced_element`] (this
/// unit's shared builder, also used by `bean::parse_decorator`) re-derives
/// its own overlay from it, same convention `bean::parse_bean` follows for
/// its own recursion.
pub(crate) fn parse_namespaced(
    ctx: &mut BeansFileCtx,
    diagnostics: &mut Vec<Diagnostic>,
    scope: &NsScope,
    element: &XmlElement,
) {
    ctx.namespaced
        .push(crate::namespaced::build_namespaced_element(
            scope,
            element,
            diagnostics,
        ));
}
