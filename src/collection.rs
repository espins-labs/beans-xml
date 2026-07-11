//! Unit **U5b** — collections (SB-07): `<list>`/`<set>`/`<array>`/`<map>`/
//! `<props>` → [`Collection`]. Per the build plan's own U5b row: "an item
//! **reuses U5a InjectValue** (value/ref/inner)" — every item, map key/value,
//! and nested collection resolves through `inject_value::begin_resolve_value`
//! unchanged; "collection → U5b (self-recursion)" — a `<list>` nested inside
//! another `<list>` is just another value-shaped child of a `ListLikeFrame`
//! item, so it recurses back through that exact same
//! `begin_resolve_value`/`begin_resolve_collection` classification, never a
//! second reimplementation (see [`ListLikeFrame`]/[`MapFrame`]'s own doc
//! comments, and `crate::depth_engine`'s module doc comment for the engine
//! shape this all runs on).
//!
//! U5a→U5b is a *serial* continuation of the same classification, not a
//! parallel leaf pair (build plan: "U5b (collections) is serial after
//! U5a") — this module wires directly into
//! `inject_value::begin_resolve_value`'s own `Collection` arm (previously
//! reserved with a silent `None`), which is that module's own seam, not one
//! of the frozen root-/bean-child dispatch matches the leaf-conflict-avoidance
//! contract protects.
//!
//! Depth bookkeeping mirrors `inject_value::begin_resolve_value`'s own
//! `"bean"` handling exactly: the incoming `depth` is checked against
//! [`crate::DEPTH_LIMIT`] *before* any recursion happens (downgrading to an
//! opaque `InjectValue::Null` plus `NestingLimitExceeded` instead), and
//! every further descent — a list/set/array item, a map entry's key/value,
//! a nested collection — passes `depth + 1`, exactly one increment per hop
//! from a container to its own content. `<entry>`/`<key>` wrapper elements
//! are not themselves a hop (same non-incrementing treatment
//! `bean::BeanFrame::begin_property` gives its own `<property>` wrapper
//! before calling into `begin_resolve_value`) — only actual
//! value/bean/collection descent counts.
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
use crate::inject_value::{build_value_lit_from_segments, ref_from_attr, value_lit_from_attr};
use crate::model::{
    ClassRef, Collection, DiagCode, Diagnostic, InjectValue, MapEntry, PropEntry, Spanned,
};

/// Resolves one already-identified collection element (`<list>`/`<set>`/
/// `<array>`/`<map>`/`<props>`) into an `InjectValue::Collection`. `scope`
/// is the caller's pre-overlay scope (same convention every recursive entry
/// point in this crate follows — see this module's own doc comment); this
/// function re-derives its own overlay via `NsScope::from_element` wherever
/// it needs one, rather than the caller doing it.
///
/// **Stack-diet note** (I3 P0 Windows `STATUS_STACK_OVERFLOW` fix): the
/// `NestingLimitExceeded` early-return's `format!` call and each `match`
/// arm's own `Collection`-variant construction are both factored into
/// `#[inline(never)]` helpers below — see `bean::parse_bean`'s doc comment
/// for the "-O0 reserves every local for the whole function" framing
/// (confirmed empirically via `otool -tv` disassembly of a debug build for
/// this exact function too — this is the choke point for the list/map
/// self-recursion `tests/i3_hostile_proptest.rs`'s `deep_list`/`deep_map`
/// fixtures stress).
///
/// **I3 P0 stack-diet fallback**: the recursive list/map self-descent this
/// doc comment's own stack-diet note used to describe (`resolve_list_collection`/
/// `resolve_set_collection`/`resolve_array_collection`/`parse_map`, all
/// removed) is gone — frame-dieting alone could not reach a 256 KiB thread
/// budget at `DEPTH_LIMIT` levels for these shapes (measured via
/// `tests/scratch_stack_probe.rs`; see `crate::depth_engine`'s own module
/// doc comment for the full before/after). This function is now a thin
/// wrapper around [`crate::inject_value::begin_resolve_collection`] (the
/// same depth-check-and-classify primitive `ListLikeFrame`/`MapFrame`'s own
/// item/entry resolution funnels through) plus, for the two collection
/// shapes that do need further recursion (`list`/`set`/`array`/`map`, not
/// `props`), [`crate::depth_engine::run`] to drive that recursion on the
/// heap instead of the real call stack — the same "push one frame, run the
/// engine, unwrap the result" shape [`crate::bean::parse_bean`] uses for its
/// own top-level entry point.
///
/// `#[cfg(test)]`: every real production call site resolves a collection
/// value-shaped child through [`crate::inject_value::begin_resolve_value`]/
/// `begin_resolve_collection` directly (`bean::BeanFrame::begin_property`/
/// `begin_constructor_arg`, `ListLikeFrame`/`MapFrame`'s own item/entry
/// resolution) and drives the resulting deferred frame on its *own*
/// existing engine run rather than starting a fresh nested one here — a
/// `<list>`/`<map>`/etc. is only ever reached as another element's
/// value-shaped child, never a standalone top-level parse target, so this
/// function has no production call site of its own. It stays as this
/// module's own directly-callable entry point purely for this crate's
/// existing test suite (this file's own `#[cfg(test)] mod tests`, dozens of
/// call sites below), the same "test-only, in production terms" treatment
/// `inject_value::parse_inject_value_child`'s own doc comment documents for
/// its sibling case.
#[cfg(test)]
pub(crate) fn parse_collection_value(
    scope: &NsScope,
    diagnostics: &mut Vec<Diagnostic>,
    depth: u32,
    element: &XmlElement,
) -> InjectValue {
    match crate::inject_value::begin_resolve_collection(scope, diagnostics, depth, element) {
        crate::inject_value::ValueStep::Resolved(Some(value)) => *value,
        // `begin_resolve_collection` only ever returns `Resolved(None)` for
        // `NotBeansNs`/`Unknown` element *kinds* — a distinction it never
        // makes itself (its own `CollectionKind::Unknown` fallback already
        // resolves to `Resolved(Some(..))`, an empty list, same as this
        // function's own former defensive fallback). Kept as a match arm
        // rather than `.unwrap()` so this stays infallible (rule 4) even if
        // that invariant is ever violated by a future edit.
        crate::inject_value::ValueStep::Resolved(None) => InjectValue::Null(element.span),
        crate::inject_value::ValueStep::Deferred(frame) => {
            match crate::depth_engine::run(vec![*frame], diagnostics) {
                crate::depth_engine::Completed::Value(value) => *value,
                crate::depth_engine::Completed::Bean(_) => {
                    unreachable!("a ListLike/Map frame always finishes into Completed::Value")
                }
            }
        }
    }
}

/// [`parse_collection_value`]'s own classification result — see the
/// stack-diet note on [`classify_collection_element`] for why this is a
/// small enum rather than the raw qname `local` string. `pub(crate)`:
/// shared with `inject_value::begin_resolve_collection`, the cross-module
/// half of the I3 P0 stack-diet fallback (`crate::depth_engine`'s own
/// module doc comment) — that function needs this same classification to
/// decide "resolve immediately" (`Props`/`Unknown`) vs. "push a frame"
/// (`List`/`Set`/`Array`/`Map`).
pub(crate) enum CollectionKind {
    List,
    Set,
    Array,
    Map,
    Props,
    Unknown,
}

/// `element`'s own overlay + `resolve_qname` classification — split out of
/// [`parse_collection_value`] purely for stack-diet framing: `own_scope`
/// (72 bytes) and the qname tuple this needs are used *only* to pick a
/// `CollectionKind` — every arm above is called with the *original*
/// `scope` parameter, not `own_scope` (each arm's own callee re-derives its
/// own overlay fresh, same convention `inject_value::begin_resolve_value`
/// documents), so unlike an `own_scope` that has to stay alive across a
/// recursive call (boxed instead elsewhere in this crate's hot chain), this
/// pair can be dropped entirely once classification is done — confining
/// its cost to a frame that's fully popped before `parse_collection_value`'s
/// own recursive descent (through whichever arm matched) begins.
#[inline(never)]
pub(crate) fn classify_collection_element(scope: &NsScope, element: &XmlElement) -> CollectionKind {
    let own_scope = NsScope::from_element(element, Some(scope));
    let (_, local) = resolve_qname(&element.name, &own_scope);
    match local.as_str() {
        "list" => CollectionKind::List,
        "set" => CollectionKind::Set,
        "array" => CollectionKind::Array,
        "map" => CollectionKind::Map,
        "props" => CollectionKind::Props,
        _ => CollectionKind::Unknown,
    }
}

/// `InjectValue::Collection(Spanned { value: collection, span })` assembly
/// — split out of [`parse_collection_value`] purely for stack-diet framing:
/// this sequential wrap (`Collection` → `Spanned<Collection>` →
/// `InjectValue`, confirmed via `otool -tv` disassembly of a debug build to
/// cost three separate `memcpy`s at `-O0`) always runs (not one arm among
/// several), but still benefits from its own frame rather than parse_collection_value's
/// own — same "give large sequential construction its own transient frame"
/// rationale `bean::finish_bean`'s doc comment gives.
#[inline(never)]
pub(crate) fn box_collection_inject_value(
    collection: Collection,
    span: crate::model::ByteSpan,
) -> InjectValue {
    InjectValue::Collection(Spanned {
        value: collection,
        span,
    })
}

/// `NestingLimitExceeded` diagnostic push for [`parse_collection_value`]'s
/// own depth-limit branch — split out purely for stack-diet framing, see
/// that function's own doc comment.
#[inline(never)]
#[cold]
pub(crate) fn nesting_limit_exceeded_collection(
    diagnostics: &mut Vec<Diagnostic>,
    span: crate::model::ByteSpan,
) -> InjectValue {
    diagnostics.push(Diagnostic {
        code: DiagCode::NestingLimitExceeded,
        span: Some(span),
        message: format!(
            "collection nesting exceeded {} levels; subtree treated as opaque",
            crate::DEPTH_LIMIT
        ),
    });
    InjectValue::Null(span)
}

// ---------------------------------------------------------------------
// <list>/<set>/<array> — identical shape (build plan/model: `ListLike`).
//
// I3 P0 stack-diet fallback: `resolve_list_collection`/`resolve_set_collection`/
// `resolve_array_collection`/`parse_list_like` (former recursive item loop,
// removed) are replaced by `ListLikeFrame` — see `crate::depth_engine`'s own
// module doc comment for the full picture. `list`/`set`/`array` still share
// one frame type, differing only in which `Collection` variant
// `ListLikeFrame::finish` builds — `list`/`set`/`array`'s own item loop
// itself never differed either, so nothing beyond the finished variant
// distinguishes them here, same as before this fix.
// ---------------------------------------------------------------------

/// Which `Collection` variant a [`ListLikeFrame`] builds once finished.
pub(crate) enum ListLikeKind {
    List,
    Set,
    Array,
}

/// One in-progress `<list>`/`<set>`/`<array>` — see this section's own doc
/// comment. Every direct child element resolves through
/// `inject_value::begin_resolve_value` (values, refs, inner beans, and
/// nested collections), each one level deeper than this collection itself
/// (`depth + 1`, [`Self::step`]) — same depth-hop rule `parse_list_like`
/// (removed) documented. An unrecognized child (`None`) is silently
/// skipped, same "this function only ever resolves, never opines" policy
/// `parse_inject_value_child`'s own doc comment states.
pub(crate) struct ListLikeFrame<'a> {
    kind: ListLikeKind,
    own_scope: NsScope,
    children: &'a [XmlNode],
    idx: usize,
    depth: u32,
    items: Vec<InjectValue>,
    value_type: Option<Spanned<ClassRef>>,
    merge: Option<bool>,
    span: crate::model::ByteSpan,
}

impl<'a> ListLikeFrame<'a> {
    /// `depth` here is the containing collection's own (unincremented)
    /// depth — same as `parse_list_like`'s former `depth` parameter; items
    /// are resolved at `depth + 1` (see [`Self::step`]), matching that
    /// function's own `depth + 1` call.
    pub(crate) fn new(
        kind: ListLikeKind,
        scope: &NsScope,
        element: &'a XmlElement,
        depth: u32,
    ) -> Self {
        let own_scope = NsScope::from_element(element, Some(scope));
        let value_type = class_ref_from_attr(&element.attrs, "value-type");
        let merge = find_bool_attr(&element.attrs, "merge");
        ListLikeFrame {
            kind,
            own_scope,
            children: &element.children,
            idx: 0,
            depth,
            items: Vec::new(),
            value_type,
            merge,
            span: element.span,
        }
    }

    pub(crate) fn step(
        &mut self,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> crate::depth_engine::Advance<'a> {
        use crate::depth_engine::Advance;
        use crate::inject_value::ValueStep;
        loop {
            let Some(child) = self.children.get(self.idx) else {
                return Advance::Finished;
            };
            self.idx += 1;
            let XmlNode::Element(child_element) = child else {
                continue;
            };
            match crate::inject_value::begin_resolve_value(
                &self.own_scope,
                diagnostics,
                self.depth + 1,
                child_element,
            ) {
                ValueStep::Resolved(Some(value)) => self.items.push(*value),
                ValueStep::Resolved(None) => {}
                ValueStep::Deferred(frame) => return Advance::Push(frame),
            }
        }
    }

    // `value: Box<InjectValue>`, not `InjectValue` — matches
    // `crate::depth_engine::Frame::deliver`'s own shared dispatch (one
    // `value` binding forwarded to whichever frame kind is on top), even
    // though this particular frame kind only ever unboxes it immediately.
    #[allow(clippy::boxed_local)]
    pub(crate) fn deliver(&mut self, value: Box<InjectValue>) -> crate::depth_engine::Advance<'a> {
        self.items.push(*value);
        crate::depth_engine::Advance::Continue
    }

    pub(crate) fn finish(self) -> Box<InjectValue> {
        let collection = match self.kind {
            ListLikeKind::List => Collection::List {
                items: self.items,
                value_type: self.value_type,
                merge: self.merge,
            },
            ListLikeKind::Set => Collection::Set {
                items: self.items,
                value_type: self.value_type,
                merge: self.merge,
            },
            ListLikeKind::Array => Collection::Array {
                items: self.items,
                value_type: self.value_type,
                merge: self.merge,
            },
        };
        Box::new(box_collection_inject_value(collection, self.span))
    }
}

// ---------------------------------------------------------------------
// <map>.
//
// I3 P0 stack-diet fallback: `parse_map` (former recursive entry loop,
// removed) is replaced by `MapFrame` below — see `crate::depth_engine`'s
// own module doc comment for the full picture.
// ---------------------------------------------------------------------

/// `<map key-type="..." value-type="..." merge="...">` — `key-type` and
/// (map-level, default-for-every-entry) `value-type` both live on the
/// `Collection::Map` variant itself; a per-`<entry>` `value-type` override
/// lands on that entry's own `MapEntry::value_type` instead (see
/// `parse_map_entry`).
///
/// One in-progress `<map>` — see this section's own doc comment.
/// `entry_state`, when `Some`, is the in-progress `<entry>` at
/// `children[idx - 1]`; `<map>`'s own children loop only advances past an
/// `<entry>` once that entry's own scan (key + value, each possibly
/// recursive — see [`EntryScan`]) has fully finished, matching the former
/// `parse_map`/`parse_map_entry` pair's own strict "one entry at a time, in
/// document order" serialization.
pub(crate) struct MapFrame<'a> {
    own_scope: NsScope,
    children: &'a [XmlNode],
    idx: usize,
    depth: u32,
    entries: Vec<MapEntry>,
    key_type: Option<Spanned<ClassRef>>,
    value_type: Option<Spanned<ClassRef>>,
    merge: Option<bool>,
    span: crate::model::ByteSpan,
    entry_state: Option<EntryScan<'a>>,
}

impl<'a> MapFrame<'a> {
    /// `depth` here is the map's own (unincremented) depth — same as
    /// `parse_map`'s former `depth` parameter; entries are scanned at
    /// `depth + 1` (see [`EntryScan::new`]'s call site in [`Self::step`]),
    /// matching that function's own `depth + 1` call into `parse_map_entry`.
    pub(crate) fn new(scope: &NsScope, element: &'a XmlElement, depth: u32) -> Self {
        let own_scope = NsScope::from_element(element, Some(scope));
        let (key_type, value_type, merge) = resolve_map_attrs(element);
        MapFrame {
            own_scope,
            children: &element.children,
            idx: 0,
            depth,
            entries: Vec::new(),
            key_type,
            value_type,
            merge,
            span: element.span,
            entry_state: None,
        }
    }

    pub(crate) fn step(
        &mut self,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> crate::depth_engine::Advance<'a> {
        use crate::depth_engine::Advance;
        loop {
            if let Some(entry) = &mut self.entry_state {
                match entry.step(diagnostics) {
                    Advance::Finished => {
                        let entry = self.entry_state.take().expect("just matched Some above");
                        entry.finish(diagnostics, &mut self.entries);
                        continue;
                    }
                    other => return other,
                }
            }
            let Some(child) = self.children.get(self.idx) else {
                return Advance::Finished;
            };
            self.idx += 1;
            let XmlNode::Element(child_element) = child else {
                continue;
            };
            if child_is_map_entry(&self.own_scope, child_element) {
                self.entry_state = Some(EntryScan::new(
                    &self.own_scope,
                    child_element,
                    self.depth + 1,
                    diagnostics,
                ));
            }
        }
    }

    pub(crate) fn deliver(
        &mut self,
        value: Box<InjectValue>,
        _diagnostics: &mut Vec<Diagnostic>,
    ) -> crate::depth_engine::Advance<'a> {
        self.entry_state
            .as_mut()
            .expect("MapFrame delivered without an in-progress <entry>")
            .deliver(value)
    }

    pub(crate) fn finish(self) -> Box<InjectValue> {
        Box::new(box_collection_inject_value(
            Collection::Map {
                entries: self.entries,
                key_type: self.key_type,
                value_type: self.value_type,
                merge: self.merge,
            },
            self.span,
        ))
    }
}

/// `<map key-type= value-type= merge=>` attribute reads — unchanged from
/// before this fix (still shared by [`MapFrame::new`]).
#[inline(never)]
fn resolve_map_attrs(
    element: &XmlElement,
) -> (
    Option<Spanned<ClassRef>>,
    Option<Spanned<ClassRef>>,
    Option<bool>,
) {
    let key_type = class_ref_from_attr(&element.attrs, "key-type");
    let value_type = class_ref_from_attr(&element.attrs, "value-type");
    let merge = find_bool_attr(&element.attrs, "merge");
    (key_type, value_type, merge)
}

/// Whether `child_element` resolves (under `scope`) to an `<entry>` element
/// in the beans namespace — split out of [`parse_map`]'s loop purely for
/// stack-diet framing, see that loop's own call-site comment. Per standard
/// XML namespace scoping, a `xmlns`/`xmlns:*` declaration on `<entry>`
/// itself applies to that element's own name — overlay the child's own
/// declarations before resolving it, same as `dispatch_root_child`/
/// `dispatch_bean_child` do for their own children (see those functions'
/// doc comments).
#[inline(never)]
fn child_is_map_entry(scope: &NsScope, child_element: &XmlElement) -> bool {
    let child_scope = NsScope::from_element(child_element, Some(scope));
    // Kept as one `qn: (String, String)` binding rather than destructured
    // `let (ns, local) = ..` — stack-diet micro-optimization, see
    // `dispatch::dispatch_root_child`'s own matching comment for the
    // empirical (MIR-dump-confirmed) rationale.
    let qn = resolve_qname(&child_element.name, &child_scope);
    qn.1 == "entry" && is_beans_ns(&qn.0)
}

/// One in-progress `<entry key= key-ref= value= value-ref= value-type=>`,
/// plus the `<key>` element form and a value-shaped child element —
/// replaces the former `parse_map_entry`/`resolve_first_child_value` pair
/// (removed; see `crate::depth_engine`'s own module doc comment for the
/// full picture). Precedence (both key and value independently): the
/// literal attribute wins, then the `-ref` attribute, then a resolved
/// child, then (nothing at all present) an opaque `InjectValue::Null` at
/// the entry's own span — same "never a missing value, never a panic"
/// fallback `property::resolve_value` documents for `<property>`. Both
/// `key`+`key-ref` and `value`+`value-ref` present together raise
/// `ConflictingValueAndRef` (additive — some deterministic value/key is
/// still produced), the same diagnostic `property::parse_property` raises
/// for its own `value=`/`ref=` pair.
///
/// `depth` (see [`Self::new`]) is already one hop past the owning `<map>`
/// (`MapFrame::step`'s own call site) — passed through unchanged into both
/// the `<key>` child and the entry's own value child, since `<entry>`/`<key>`
/// are wrapper elements, not themselves a nesting hop (this module's own
/// doc comment).
///
/// Re-entrant `<key>` resolution rule (preserved exactly from
/// `parse_map_entry`'s own former loop): a `<key>` element is only ever
/// *attempted* while `key_child` is still `None` — since that stays `None`
/// whether no `<key>` has been seen yet *or* the first `<key>` found had no
/// resolvable child of its own (`<key></key>`, `<key>text</key>`), a
/// **second** `<key>` sibling later in `element`'s own children can still
/// get its own attempt. This is why [`Self::step`] below is a genuine
/// resumable scan over `element`'s own children (not a two-phase
/// classify-then-resolve split, unlike [`crate::bean::BeanFrame`]'s own
/// `<meta>`/value-candidate handling, whose "first non-meta child" choice
/// never depends on that candidate's own resolution *outcome* the way this
/// one's `<key>` retry does).
struct EntryScan<'a> {
    own_scope: NsScope,
    key_attr: Option<&'a XmlAttr>,
    key_ref_attr: Option<&'a XmlAttr>,
    value_attr: Option<&'a XmlAttr>,
    value_ref_attr: Option<&'a XmlAttr>,
    value_type: Option<Spanned<ClassRef>>,
    span: crate::model::ByteSpan,
    children: &'a [XmlNode],
    idx: usize,
    depth: u32,
    key_child: Option<Box<InjectValue>>,
    value_child: Option<Box<InjectValue>>,
    seen_value_child: bool,
    waiting: Option<EntryWaiting>,
}

/// Which of an [`EntryScan`]'s two slots a deferred sub-resolution (pushed
/// via [`EntryScan::step`]) will fill in once it's delivered back.
enum EntryWaiting {
    Key,
    Value,
}

impl<'a> EntryScan<'a> {
    fn new(
        scope: &NsScope,
        element: &'a XmlElement,
        depth: u32,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Self {
        let own_scope = NsScope::from_element(element, Some(scope));
        let key_attr = find_attr(&element.attrs, "key");
        let key_ref_attr = find_attr(&element.attrs, "key-ref");
        if key_attr.is_some() && key_ref_attr.is_some() {
            push_map_entry_key_conflict(diagnostics, element.span);
        }
        let value_attr = find_attr(&element.attrs, "value");
        let value_ref_attr = find_attr(&element.attrs, "value-ref");
        if value_attr.is_some() && value_ref_attr.is_some() {
            push_map_entry_value_conflict(diagnostics, element.span);
        }
        let value_type = class_ref_from_attr(&element.attrs, "value-type");
        EntryScan {
            own_scope,
            key_attr,
            key_ref_attr,
            value_attr,
            value_ref_attr,
            value_type,
            span: element.span,
            children: &element.children,
            idx: 0,
            depth,
            key_child: None,
            value_child: None,
            seen_value_child: false,
            waiting: None,
        }
    }

    fn step(&mut self, diagnostics: &mut Vec<Diagnostic>) -> crate::depth_engine::Advance<'a> {
        use crate::depth_engine::Advance;
        use crate::inject_value::ValueStep;
        debug_assert!(self.waiting.is_none());
        loop {
            let Some(child) = self.children.get(self.idx) else {
                return Advance::Finished;
            };
            self.idx += 1;
            let XmlNode::Element(child_element) = child else {
                continue;
            };
            if child_is_map_key(&self.own_scope, child_element) {
                if self.key_child.is_some() {
                    continue;
                }
                // `resolve_first_child_value`'s former job: only the
                // `<key>` element's own *first* child element is ever
                // resolved, under `<key>`'s own overlay (not the entry's) —
                // `<key>` may carry its own `xmlns`/`xmlns:*` declarations.
                let key_scope = NsScope::from_element(child_element, Some(&self.own_scope));
                let Some(inner) = first_child_element(child_element) else {
                    continue; // key_child stays None; a later <key> may still try.
                };
                match crate::inject_value::begin_resolve_value(
                    &key_scope,
                    diagnostics,
                    self.depth,
                    inner,
                ) {
                    ValueStep::Resolved(value) => self.key_child = value,
                    ValueStep::Deferred(frame) => {
                        self.waiting = Some(EntryWaiting::Key);
                        return Advance::Push(frame);
                    }
                }
                continue;
            }
            // The first non-`<key>` child element is this entry's value —
            // mirrors `property::parse_property`'s "only the first ...
            // child is resolved" leniency (the XSD only ever allows one).
            if !self.seen_value_child {
                self.seen_value_child = true;
                match crate::inject_value::begin_resolve_value(
                    &self.own_scope,
                    diagnostics,
                    self.depth,
                    child_element,
                ) {
                    ValueStep::Resolved(value) => self.value_child = value,
                    ValueStep::Deferred(frame) => {
                        self.waiting = Some(EntryWaiting::Value);
                        return Advance::Push(frame);
                    }
                }
            }
        }
    }

    fn deliver(&mut self, value: Box<InjectValue>) -> crate::depth_engine::Advance<'a> {
        match self
            .waiting
            .take()
            .expect("EntryScan delivered without a pending key/value wait")
        {
            EntryWaiting::Key => self.key_child = Some(value),
            EntryWaiting::Value => self.value_child = Some(value),
        }
        crate::depth_engine::Advance::Continue
    }

    /// Consumes this finished entry, pushing the assembled [`MapEntry`]
    /// onto `entries` — `MapFrame::step`'s own call site, once this entry's
    /// `step` has returned `Advance::Finished`.
    fn finish(self, diagnostics: &mut Vec<Diagnostic>, entries: &mut Vec<MapEntry>) {
        finish_map_entry(
            self.span,
            self.key_attr,
            self.key_ref_attr,
            self.value_attr,
            self.value_ref_attr,
            self.value_type,
            self.key_child,
            self.value_child,
            diagnostics,
            entries,
        );
    }
}

/// The first direct child *element* of `element` (skipping text/CDATA
/// runs), or `None` if it has none — [`EntryScan::step`]'s own `<key>`
/// handling, same "only the first child element found is ever resolved"
/// leniency the former `resolve_first_child_value` documented.
fn first_child_element(element: &XmlElement) -> Option<&XmlElement> {
    element.children.iter().find_map(|c| match c {
        XmlNode::Element(e) => Some(e),
        XmlNode::Text(_) => None,
    })
}

/// Whether `child_element` resolves (under `scope`) to a `<key>` element in
/// the beans namespace — same namespace-scoping fix `parse_map`'s own
/// `<entry>` detection (`child_is_map_entry`) applies (see that function's
/// own doc comment).
#[inline(never)]
fn child_is_map_key(scope: &NsScope, child_element: &XmlElement) -> bool {
    let child_scope = NsScope::from_element(child_element, Some(scope));
    // Kept as one `qn: (String, String)` binding rather than destructured
    // `let (ns, local) = ..` — stack-diet micro-optimization, see
    // `dispatch::dispatch_root_child`'s own matching comment for the
    // empirical (MIR-dump-confirmed) rationale.
    let qn = resolve_qname(&child_element.name, &child_scope);
    qn.1 == "key" && is_beans_ns(&qn.0)
}

/// `key=`/`key-ref=` `ConflictingValueAndRef` diagnostic push — split out of
/// [`parse_map_entry`] purely for stack-diet framing, see that function's
/// own doc comment.
#[inline(never)]
fn push_map_entry_key_conflict(diagnostics: &mut Vec<Diagnostic>, span: crate::model::ByteSpan) {
    diagnostics.push(Diagnostic {
        code: DiagCode::ConflictingValueAndRef,
        span: Some(span),
        message: "<entry> specifies both key= and key-ref=".to_string(),
    });
}

/// `value=`/`value-ref=` `ConflictingValueAndRef` diagnostic push — split
/// out of [`parse_map_entry`] purely for stack-diet framing, see that
/// function's own doc comment.
#[inline(never)]
fn push_map_entry_value_conflict(diagnostics: &mut Vec<Diagnostic>, span: crate::model::ByteSpan) {
    diagnostics.push(Diagnostic {
        code: DiagCode::ConflictingValueAndRef,
        span: Some(span),
        message: "<entry> specifies both value= and value-ref=".to_string(),
    });
}

/// Final key/value precedence resolution + [`MapEntry`] assembly — split
/// out of [`parse_map_entry`] purely for stack-diet framing, same rationale
/// `property::finish_property` documents (only ever runs after this
/// entry's own child loop, and any recursion reached through it, has fully
/// returned). The key/value precedence resolution itself is further split
/// into [`resolve_map_entry_key`]/[`resolve_map_entry_value`] — measured
/// (via `otool -tv` disassembly of a debug build) at a combined ~2KB for
/// this function before that split: each `.map(..).or_else(..).or(..)`
/// chain constructs up to three separate `InjectValue`-shaped payloads
/// (~120 bytes apiece — `Value`/`Ref`/the resolved child), and with *two*
/// such chains (key and value) inline in one function, all of those
/// temporaries summed into one frame at `-O0`, same "match arm" reservation
/// issue `inject_value::begin_resolve_value`'s doc comment documents for a
/// different function shape.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn finish_map_entry(
    span: crate::model::ByteSpan,
    key_attr: Option<&XmlAttr>,
    key_ref_attr: Option<&XmlAttr>,
    value_attr: Option<&XmlAttr>,
    value_ref_attr: Option<&XmlAttr>,
    value_type: Option<Spanned<ClassRef>>,
    key_child: Option<Box<InjectValue>>,
    value_child: Option<Box<InjectValue>>,
    diagnostics: &mut Vec<Diagnostic>,
    entries: &mut Vec<MapEntry>,
) {
    let key = resolve_map_entry_key(key_attr, key_ref_attr, key_child, diagnostics, span);
    let value = resolve_map_entry_value(value_attr, value_ref_attr, value_child, diagnostics, span);

    entries.push(MapEntry {
        span,
        key,
        value,
        value_type,
    });
}

/// This entry's `key` precedence chain — split out of [`finish_map_entry`]
/// purely for stack-diet framing, see that function's own doc comment.
#[inline(never)]
fn resolve_map_entry_key(
    key_attr: Option<&XmlAttr>,
    key_ref_attr: Option<&XmlAttr>,
    key_child: Option<Box<InjectValue>>,
    diagnostics: &mut Vec<Diagnostic>,
    span: crate::model::ByteSpan,
) -> InjectValue {
    key_attr
        .map(|attr| InjectValue::Value(value_lit_from_attr(attr)))
        .or_else(|| {
            key_ref_attr.and_then(|attr| ref_from_attr(attr, diagnostics).map(InjectValue::Ref))
        })
        .or(key_child.map(|b| *b))
        .unwrap_or(InjectValue::Null(span))
}

/// This entry's `value` precedence chain — split out of [`finish_map_entry`]
/// purely for stack-diet framing, see that function's own doc comment.
#[inline(never)]
fn resolve_map_entry_value(
    value_attr: Option<&XmlAttr>,
    value_ref_attr: Option<&XmlAttr>,
    value_child: Option<Box<InjectValue>>,
    diagnostics: &mut Vec<Diagnostic>,
    span: crate::model::ByteSpan,
) -> InjectValue {
    value_attr
        .map(|attr| InjectValue::Value(value_lit_from_attr(attr)))
        .or_else(|| {
            value_ref_attr.and_then(|attr| ref_from_attr(attr, diagnostics).map(InjectValue::Ref))
        })
        .or(value_child.map(|b| *b))
        .unwrap_or(InjectValue::Null(span))
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
pub(crate) fn parse_props(scope: &NsScope, element: &XmlElement) -> Collection {
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
