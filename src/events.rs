//! Unit **U1** — event/recovery layer.
//!
//! A quick-xml event wrapper that carries byte spans, a generic
//! tree-builder skeleton, and this crate's recovery rules for malformed
//! XML input. Spec: the internal design spec's "settled decisions" (recovery
//! rules) and invariant #4 (span slice == decoded text, entities/CDATA
//! unresolved). Build order: the internal build plan's U1 row.
//!
//! `pub(crate)` — not part of the published API surface (`parse`/
//! `parse_bytes`/`is_beans_doc` plus the `model` types are). Later units
//! (U3's root-child dispatch, U4's `parse_bean`) walk the [`XmlElement`]
//! tree [`build_tree`] produces to fill in `src/model.rs` types — this
//! module deliberately stops at a namespace-agnostic, beans-agnostic tree
//! (resolving a `prefix:local` name against `xmlns:*` declarations, and
//! deciding what any given element/attribute *means*, is entirely U3/U4's
//! job).
//!
//! Design mirrors the sibling crate `batis-xml`'s `parse.rs` (same
//! quick-xml idioms: `allow_dangling_amp`, a byte-level attribute
//! scanner, the "quick-xml already resynchronized past the error
//! internally" resync property) adapted for a tree-shaped (not
//! statement-shaped) output, with three of `Reader`'s `Config` flags
//! relaxed so *this crate's own* recovery rules — not quick-xml's
//! built-in well-formedness checks — decide what counts as an unclosed
//! or orphan tag:
//!
//! - `allow_dangling_amp = true`: a bare `&`/unresolved entity arrives as
//!   ordinary event content instead of an `Err` that would otherwise
//!   swallow it via the (silent) non-XML-residue path.
//! - `check_end_names = false`, `allow_unmatched_ends = true`: every
//!   closing tag — matched, mismatched, or with no open tag at all —
//!   arrives as `Ok(Event::End)` instead of erroring. [`close_tag`] then
//!   implements recovery rules 1/2 itself against its own open-tag
//!   [`stack`](OpenTag), which is this module's actual source of truth
//!   (not quick-xml's internal one).
//!
//! Recovery rules (spec "settled decisions", batis MM-13/07/08 lineage):
//! 1. Unclosed tag → implicitly closed when an ancestor closes (or at
//!    EOF), plus `UnclosedTag`.
//! 2. Orphan/mismatched closing tag → ignored, plus `UnexpectedCloseTag`.
//! 3. Duplicate attribute → first value wins, plus `DuplicateAttribute`.
//! 4. Non-XML residue → quick-xml has already resynchronized to the next
//!    recognizable token internally by the time it returns the error;
//!    recovering here is just "keep looping", not manual `<`-scanning.
//!    No dedicated `DiagCode` exists for this rule (only the other four
//!    pair with one) — silent per spec.
//! 5. Unresolved entity reference → raw text kept, plus `InvalidEntity`.
//! 6. Unterminated `${`/`#{` placeholder → raw text kept, plus
//!    `UnterminatedPlaceholder`.
//!
//! This tree builder is **iterative** (an explicit [`Vec<OpenTag>`]
//! stack, not recursive function calls), so — unlike the later
//! value-recursion layers (U5a) — *building* it needs no
//! [`crate::DEPTH_LIMIT`] call-stack guard of its own: arbitrarily deep
//! element nesting just grows this stack on the heap, never the call
//! stack. [`build_tree`] does, however, cap the *resulting tree's own
//! structural depth* at [`MAX_TREE_DEPTH`] (P0 fix, invariant #1/SB-16):
//! [`XmlElement`] is an owned recursive tree (`children: Vec<XmlNode>`).
//! Its `Drop` impl is hand-written and iterative (a second P0 fix,
//! Windows-only `STATUS_STACK_OVERFLOW`, SB-16 follow-up) — a
//! *compiler-derived* `Drop` would recurse one call-stack frame per
//! nesting level on teardown, and `MAX_TREE_DEPTH` alone only bounds that
//! to ~2048 frames, which fits a Unix test thread's 2 MiB stack but not
//! Windows' 1 MiB default one. The depth cap below still matters for
//! memory and for every later *walk* of this tree, but teardown itself
//! must never recurse per level on any platform — see `XmlElement`'s own
//! `Drop` impl for the iterative-worklist mechanics. Beyond
//! `MAX_TREE_DEPTH`, `build_tree` stops attaching further
//! descendants (one `NestingLimitExceeded` diagnostic, subtree dropped)
//! rather than building an unbounded tree for a later unit to walk — see
//! `build_tree`'s own doc comment for the mechanics. `MAX_TREE_DEPTH` is
//! deliberately a generous multiple of [`crate::DEPTH_LIMIT`], not
//! `DEPTH_LIMIT` itself: this raw layer can't tell a `<beans>` element
//! from a `<bean>` from a `<list>`, so it counts *every* element uniformly
//! toward one depth, but the later per-kind `DEPTH_LIMIT` guards
//! (`dispatch::parse_beans_body`'s `<beans>`-in-`<beans>` recursion,
//! `inject_value::parse_inner_bean`/`collection::parse_collection_value`'s
//! shared bean/property/collection recursion, `namespaced::harvest_refs`'s
//! own descent) each reset independently — e.g. a `<beans>` chain nested
//! `DEPTH_LIMIT - 1` deep, with one ordinary `<bean>` at the bottom, is
//! entirely legitimate and diagnostic-free (`tests/p10_nested_profile.rs`'s
//! own `sb14_depth_one_below_limit_recurses_fully_with_no_diagnostic`) yet
//! already totals more raw elements than `DEPTH_LIMIT` itself. Capping the
//! raw tree at `DEPTH_LIMIT` verbatim would false-positive on inputs no
//! individual model-layer guard ever objects to; `MAX_TREE_DEPTH` stays
//! far below the tens-of-thousands-deep range confirmed to actually
//! overflow the stack while comfortably clearing every legitimate
//! combination of independently-guarded recursion this crate has.
//! `DEPTH_LIMIT` itself is additionally re-enforced once a later unit
//! recursively *walks* this (now already-bounded) tree to build nested
//! model values, so an adversarial walk can never recurse deeper than
//! `MAX_TREE_DEPTH` regardless.

use crate::model::{ByteSpan, DiagCode, Diagnostic, Spanned};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::HashSet;

/// Longest run this module will scan looking for an entity reference's
/// terminating `;` before giving up and treating the leading `&` as a
/// dangling one. Bounds [`check_entities`]'s per-`&` work at a small
/// constant — real entity/character references are short (the longest
/// named XML entity is a handful of characters; the longest sane numeric
/// one a dozen or so) — so a pathological run of non-`;` bytes after a
/// stray `&` can't turn one `&` into an unbounded scan.
const MAX_ENTITY_SCAN: usize = 64;

/// Structural depth cap [`build_tree`] enforces on its own output tree —
/// see this module's own doc comment (top of file) for why this is a
/// generous multiple of [`crate::DEPTH_LIMIT`] rather than that constant
/// itself. Chosen to comfortably clear every legitimate combination of
/// this crate's independently-guarded model-layer recursion (each capped
/// at `DEPTH_LIMIT` on its own, but summable across kinds within one raw
/// tree) while staying far below the depth empirically confirmed to
/// overflow the stack purely via `XmlElement`'s derived `Drop` (~60,000;
/// `tests/i3_hostile_proptest.rs`'s own P0 regression tests).
const MAX_TREE_DEPTH: usize = crate::DEPTH_LIMIT as usize * 8;

// ---------------------------------------------------------------------
// Tree-builder skeleton output types.
// ---------------------------------------------------------------------

/// One raw XML attribute, scanned from a start/empty tag's own byte
/// range. `name`/`value.value` are the exact raw source slice — no
/// entity resolution (invariant #4: span slice == decoded-but-unresolved
/// text) — so a downstream consumer that wants `&amp;` turned into `&`
/// does that itself.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct XmlAttr {
    pub name: String,
    pub name_span: ByteSpan,
    pub value: Spanned<String>,
}

/// A generic XML element node, before any beans-specific meaning is
/// attached to it (that's U3/U4's job — dispatching on `name`, resolving
/// a `prefix:local` split against ancestor `xmlns:*` declarations).
/// `name` is the raw qualified name exactly as written (e.g. `"bean"`,
/// `"context:component-scan"`, `"aop:advisor"`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct XmlElement {
    pub name: String,
    /// Opening tag start → matching (or implicitly-closed) end → subtree
    /// end, or the element's own extent for a self-closed tag.
    pub span: ByteSpan,
    pub attrs: Vec<XmlAttr>,
    pub children: Vec<XmlNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum XmlNode {
    Element(XmlElement),
    /// Raw text/CDATA run — span slice == source text verbatim (entities
    /// unresolved, CDATA delimiters excluded; invariant #4).
    Text(Spanned<String>),
}

/// Hand-written **iterative** teardown — the compiler-derived `Drop` this
/// replaces recurses one call-stack frame per nesting level (drop
/// `XmlElement` → drop its `Vec<XmlNode>` → drop each child `XmlElement` →
/// ...), which is exactly the same per-level-recursion shape `build_tree`'s
/// own doc comment already flags for *building*: `MAX_TREE_DEPTH` bounds
/// that recursion to ~2048 frames, comfortably inside a Unix test thread's
/// 2 MiB stack but not inside Windows' 1 MiB default thread stack — this
/// crate observed exactly that as a Windows-only `STATUS_STACK_OVERFLOW` in
/// CI (`tests/i3_hostile_proptest.rs`) even with the depth cap in place.
/// `MAX_TREE_DEPTH` still bounds *build* and every downstream *walk*
/// (belt-and-braces for memory and those call stacks) — this impl only
/// removes recursion from the one place a depth cap alone can't make safe
/// on every platform: teardown.
///
/// The trick: drain `self.children` into a flat `Vec<XmlNode>` used as an
/// explicit worklist, then repeatedly pop a node and, if it's an element,
/// append *its* children onto the same worklist before letting the popped
/// `XmlElement` itself drop. Because that popped element's `children` was
/// already emptied (via `append`, which leaves the source `Vec` empty)
/// before it drops, its own `Drop` recurses zero further levels — the
/// entire tree, regardless of original depth, is torn down by one flat
/// loop over a heap-allocated stack rather than the call stack. No node is
/// ever visited twice and none is skipped: `append` moves (not copies)
/// elements onto the worklist, so ownership transfers exactly once per
/// node.
impl Drop for XmlElement {
    fn drop(&mut self) {
        let mut stack: Vec<XmlNode> = std::mem::take(&mut self.children);
        while let Some(node) = stack.pop() {
            if let XmlNode::Element(mut el) = node {
                stack.append(&mut el.children);
            }
            // `el`/`node` drops here: an `XmlElement`'s `children` is now
            // always empty by the time its own `Drop` runs (either it
            // started empty, or this loop just drained it), so this
            // recursive-looking drop call never actually recurses further.
        }
    }
}

/// [`build_tree`]'s output: the document's one root element (`None` for
/// an empty or all-junk input — no element ever found), plus every
/// diagnostic accumulated while walking it.
#[derive(Debug, Default)]
pub(crate) struct TreeResult {
    pub root: Option<XmlElement>,
    pub diagnostics: Vec<Diagnostic>,
}

/// One element still open while its children are being collected.
struct OpenTag {
    name: String,
    start: u32,
    attrs: Vec<XmlAttr>,
    children: Vec<XmlNode>,
}

// ---------------------------------------------------------------------
// Entry point.
// ---------------------------------------------------------------------

/// Parses `source` (already-decoded UTF-8 text — encoding detection is
/// U2's job, not this layer's) into a generic tree plus recovery
/// diagnostics. Never panics (invariant #1) — every anomaly this layer
/// can detect becomes a [`Diagnostic`] instead, per this crate's
/// `AGENTS.md` rule 4.
pub(crate) fn build_tree(source: &str) -> TreeResult {
    // quick-xml's `Reader` silently skips a *leading* BOM itself and
    // reports `buffer_position()` relative to the post-BOM content —
    // verified directly against quick-xml 0.41 (a proptest fixture
    // starting with U+FEFF panicked on this exact skew before this fix).
    // Every span this function computes must stay valid against the
    // original `source` the caller handed in, so a leading BOM (looped,
    // in case of more than one — quick-xml's own skip is one-shot, same
    // property batis-xml's `parse_str` documents for its identical fix)
    // is stripped for the `Reader` only, with `base_offset` added back
    // into every position `reader.buffer_position()`/`error_position()`
    // reports before it's used to index into `source`.
    let mut working = source;
    let mut base_offset: u32 = 0;
    while let Some(rest) = working.strip_prefix('\u{FEFF}') {
        base_offset += (working.len() - rest.len()) as u32;
        working = rest;
    }

    let mut reader = configured_reader(working);

    let mut diagnostics = Vec::new();
    let mut root: Option<XmlElement> = None;
    let mut stack: Vec<OpenTag> = Vec::new();
    // Bounds this tree's own maximum depth at `MAX_TREE_DEPTH` (invariant
    // #1, SB-16): `XmlElement { children: Vec<XmlNode> }` is an owned
    // recursive tree, so its compiler-derived `Drop` recurses one
    // call-stack frame per nesting level on *teardown* — arbitrarily deep
    // input (well under `MAX_INPUT_BYTES`) can overflow the stack purely
    // on `Drop`, even though `build_tree` itself is iterative (this
    // module's own doc comment above) and every later walker already
    // depth-guards its own *recursion*. `suppressed_depth` counts how many
    // further Start/End pairs are still open once `stack`'s own depth
    // first reaches `MAX_TREE_DEPTH`: the Start that crosses the limit
    // gets a single `NestingLimitExceeded` diagnostic and neither it nor
    // anything inside its subtree (further elements, text) is attached to
    // the tree at all — not merely capped one level deeper, so `stack`/the
    // tree it builds can never exceed `MAX_TREE_DEPTH` frames, bounding
    // build, every walk, and `Drop` alike. Once nesting returns to
    // at-or-under the limit (`suppressed_depth` back to `0`), parsing
    // resumes normally — a sibling after the over-deep subtree still
    // parses.
    let mut suppressed_depth: u32 = 0;
    // Entity-split harvesting regression fix (M1, coalesced with
    // `inject_value`'s own identical fix — see that module's
    // `extract_placeholders_and_spel_refs` doc comment): `&amp;`/other
    // entity references arrive as their own `GeneralRef` event, splitting
    // one logical run of element text into several byte-contiguous
    // `Text`/`GeneralRef`/`CData` runs. Scanning each run in isolation for
    // `${}`/`#{}` used to miss (and misdiagnose as `UnterminatedPlaceholder`)
    // any expression whose opener and closer landed in different runs —
    // `#{flagA &amp;&amp; flagB}` is exactly that shape. `pending_placeholder_span`
    // accumulates the current run of byte-contiguous text (`prev.span.end ==
    // next.span.start`, checked in [`extend_pending_placeholder_span`])
    // rather than scanning each `Text`/`GeneralRef`/`CData` event the moment
    // it arrives; the coalesced span is only actually scanned once
    // contiguity breaks (a non-text event, a comment/CDATA-delimiter gap, or
    // end of input) or at [`flush_pending_placeholder_span`]. This changes
    // *when* `UnterminatedPlaceholder` is diagnosed relative to other
    // diagnostics from intervening events, but not *whether* — every
    // existing single-run case coalesces trivially (one run, flushed
    // immediately after by the very next non-contiguous event).
    let mut pending_placeholder_span: Option<ByteSpan> = None;

    loop {
        let start = base_offset + reader.buffer_position() as u32;
        match reader.read_event() {
            Ok(Event::Eof) => {
                flush_pending_placeholder_span(
                    &mut pending_placeholder_span,
                    source,
                    &mut diagnostics,
                );
                close_unclosed_at_eof(&mut stack, &mut root, source.len() as u32, &mut diagnostics);
                break;
            }
            Ok(Event::Start(tag)) => {
                if suppressed_depth > 0 {
                    // Already inside an over-deep subtree: this Start is
                    // dropped along with the rest of it — just track the
                    // extra open/close pair so the matching End doesn't
                    // resume normal processing too early.
                    suppressed_depth += 1;
                } else if stack.len() >= MAX_TREE_DEPTH {
                    // First Start to cross `MAX_TREE_DEPTH`: bound the
                    // tree right here — one diagnostic, this element (and
                    // everything inside it) never becomes a tree node.
                    let end = base_offset + reader.buffer_position() as u32;
                    diagnostics.push(Diagnostic {
                        code: DiagCode::NestingLimitExceeded,
                        span: Some(ByteSpan { start, end }),
                        message: format!(
                            "element nesting exceeded {MAX_TREE_DEPTH} levels; deeper subtree \
                             dropped to bound the tree's own depth"
                        ),
                    });
                    suppressed_depth = 1;
                } else {
                    let end = base_offset + reader.buffer_position() as u32;
                    let name = qname_string(tag.name().as_ref());
                    let attrs =
                        scan_attrs_for_tag(source, start as usize, end as usize, &mut diagnostics);
                    stack.push(OpenTag {
                        name,
                        start,
                        attrs,
                        children: Vec::new(),
                    });
                }
            }
            Ok(Event::Empty(tag)) => {
                if suppressed_depth == 0 {
                    if stack.len() >= MAX_TREE_DEPTH {
                        // A self-closed element sitting exactly at the
                        // limit: dropped on its own (no persistent
                        // suppression needed — it has no separate End).
                        let end = base_offset + reader.buffer_position() as u32;
                        diagnostics.push(Diagnostic {
                            code: DiagCode::NestingLimitExceeded,
                            span: Some(ByteSpan { start, end }),
                            message: format!(
                                "element nesting exceeded {MAX_TREE_DEPTH} levels; element \
                                 dropped to bound the tree's own depth"
                            ),
                        });
                    } else {
                        let end = base_offset + reader.buffer_position() as u32;
                        let name = qname_string(tag.name().as_ref());
                        let attrs = scan_attrs_for_tag(
                            source,
                            start as usize,
                            end as usize,
                            &mut diagnostics,
                        );
                        let element = XmlElement {
                            name,
                            span: ByteSpan { start, end },
                            attrs,
                            children: Vec::new(),
                        };
                        push_completed(element, &mut stack, &mut root);
                    }
                }
                // else: dropped along with the rest of the over-deep
                // subtree it's nested in.
            }
            Ok(Event::End(tag)) => {
                if suppressed_depth > 0 {
                    // `saturating_sub`, not a bare `-= 1`: a malformed
                    // over-deep subtree with more closes than opens (an
                    // orphan close per recovery rule 2) must never
                    // underflow this counter (invariant #1) — worst case
                    // it exits suppression one End early, which just
                    // resumes ordinary `close_tag` recovery a bit sooner
                    // rather than panicking.
                    suppressed_depth = suppressed_depth.saturating_sub(1);
                } else {
                    let end = base_offset + reader.buffer_position() as u32;
                    let name = qname_string(tag.name().as_ref());
                    close_tag(&name, start, end, &mut stack, &mut root, &mut diagnostics);
                }
            }
            Ok(Event::Text(text)) => {
                if suppressed_depth == 0 {
                    let end = base_offset + reader.buffer_position() as u32;
                    let raw_span = ByteSpan { start, end };
                    let raw_text = &source[raw_span.start as usize..raw_span.end as usize];
                    check_entities(raw_text, raw_span, &mut diagnostics);
                    extend_pending_placeholder_span(
                        &mut pending_placeholder_span,
                        raw_span,
                        source,
                        &mut diagnostics,
                    );
                    push_text(raw_text.to_string(), raw_span, &mut stack);
                }
                let _ = text;
            }
            Ok(Event::GeneralRef(_entity_ref)) => {
                // An entity/character reference (`&name;` / `&#NN;` /
                // `&#xNN;`) — its own event as of quick-xml 0.41, rather
                // than embedded in the surrounding `Text` event. By
                // construction its raw content is exactly one `&...;`
                // reference, no room for a `${`/`#{` opener or closer to
                // appear inside it — but a real expression can still *span*
                // across it (`#{flagA &amp;&amp; flagB}`), so this run still
                // extends `pending_placeholder_span` exactly like `Text`/
                // `CData`, it just never contributes an opener/closer of its
                // own.
                if suppressed_depth == 0 {
                    let end = base_offset + reader.buffer_position() as u32;
                    let raw_span = ByteSpan { start, end };
                    let raw_text = &source[raw_span.start as usize..raw_span.end as usize];
                    check_entities(raw_text, raw_span, &mut diagnostics);
                    extend_pending_placeholder_span(
                        &mut pending_placeholder_span,
                        raw_span,
                        source,
                        &mut diagnostics,
                    );
                    push_text(raw_text.to_string(), raw_span, &mut stack);
                }
            }
            Ok(Event::CData(_cdata)) => {
                if suppressed_depth == 0 {
                    let end = (base_offset + reader.buffer_position() as u32) as usize;
                    // Event span includes the `<![CDATA[` (9 bytes) / `]]>`
                    // (3 bytes) delimiters; the segment span is the inner
                    // content only. `.max(inner_start)`/`saturating_sub`
                    // guard against a malformed/too-short span rather than
                    // ever underflowing (defends invariant #1).
                    let inner_start = (start as usize + 9).min(end);
                    let inner_end = end.saturating_sub(3).max(inner_start);
                    let raw_span = ByteSpan {
                        start: inner_start as u32,
                        end: inner_end as u32,
                    };
                    let raw_text = &source[raw_span.start as usize..raw_span.end as usize];
                    // CDATA content is never entity-interpreted by XML
                    // itself (that's the point of CDATA) — only the
                    // placeholder scan applies here, not `check_entities`.
                    extend_pending_placeholder_span(
                        &mut pending_placeholder_span,
                        raw_span,
                        source,
                        &mut diagnostics,
                    );
                    push_text(raw_text.to_string(), raw_span, &mut stack);
                }
            }
            Ok(_) => {
                // Comment / processing instruction / XML declaration /
                // DOCTYPE: not part of the tree, recorded nowhere —
                // deliberately out of scope (they carry no beans-xml
                // content; U3 only cares about the root element).
            }
            Err(_err) => {
                // Recovery rule 4 (non-XML residue): with
                // `allow_dangling_amp`/`check_end_names`/
                // `allow_unmatched_ends` all relaxed above, every
                // tag-matching anomaly this crate has its own recovery
                // rule for (1/2/3/5) already arrives as an `Ok` event
                // instead of an `Err` — see each setting's own doc
                // comment on `quick_xml::reader::Config` for exactly this
                // guarantee. What's left erroring out here is markup
                // quick-xml's tokenizer couldn't make sense of as XML at
                // all (e.g. `<!zzz>`, not a valid comment/CDATA/DOCTYPE
                // opener) — and unlike the IllFormed anomalies above,
                // verified directly that quick-xml does *not* resynchronize
                // past this on its own: the very next `read_event()` call
                // returns `Eof` at the *same* position rather than picking
                // up at the next token, since tokenization itself failed
                // (there's no well-formed event to skip past). So this
                // layer does its own resync exactly as the spec describes:
                // skip ahead to the next `<` and restart a fresh `Reader`
                // there. `skip_to` is always `>= reader.error_position() +
                // 1` (in `working`'s coordinate space), so `base_offset`
                // strictly increases on every resync — the loop can only
                // resync at most `source.len()` times before either
                // finding no further `<` (handled below) or reaching a
                // genuine `Eof`, so this always terminates without needing
                // a separate stuck-position guard.
                let err_pos = reader.error_position() as usize;
                let mut search_from = (err_pos + 1).min(working.len());
                while search_from < working.len() && !working.is_char_boundary(search_from) {
                    search_from += 1;
                }
                match working[search_from..].find('<') {
                    Some(rel) => {
                        let skip_to = search_from + rel;
                        working = &working[skip_to..];
                        base_offset += skip_to as u32;
                        reader = configured_reader(working);
                    }
                    None => {
                        // Nothing left in the remainder could ever become
                        // a recognizable token again.
                        flush_pending_placeholder_span(
                            &mut pending_placeholder_span,
                            source,
                            &mut diagnostics,
                        );
                        close_unclosed_at_eof(
                            &mut stack,
                            &mut root,
                            source.len() as u32,
                            &mut diagnostics,
                        );
                        break;
                    }
                }
            }
        }
    }

    TreeResult { root, diagnostics }
}

/// Builds a `Reader` with this module's recovery-rule `Config` flags
/// applied — shared by `build_tree`'s initial reader and every
/// resynchronized one it constructs after a rule-4 recovery (see that
/// arm's own doc comment), so the two can't drift out of sync.
fn configured_reader(s: &str) -> Reader<&[u8]> {
    let mut reader = Reader::from_str(s);
    let config = reader.config_mut();
    config.allow_dangling_amp = true;
    config.check_end_names = false;
    config.allow_unmatched_ends = true;
    reader
}

fn qname_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Attaches a completed element either as a child of the new stack top
/// (if any element is still open) or as the document root (the *first*
/// top-level element only — mirrors batis-xml's `scan_trailing_content`:
/// deliberately narrow, extra top-level content after the root is
/// silently ignored rather than overwriting it or erroring).
fn push_completed(element: XmlElement, stack: &mut [OpenTag], root: &mut Option<XmlElement>) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(XmlNode::Element(element));
    } else if root.is_none() {
        *root = Some(element);
    }
}

/// Attaches a text run to the currently-open element, if any. Text
/// outside every element (before the root opens, or after it closes) has
/// nowhere valid to go and is dropped — same narrow scope as
/// `push_completed`'s trailing-content case.
fn push_text(text: String, span: ByteSpan, stack: &mut [OpenTag]) {
    if let Some(parent) = stack.last_mut() {
        parent
            .children
            .push(XmlNode::Text(Spanned { value: text, span }));
    }
}

/// Recovery rules 1 + 2. Searches `stack` (innermost last) for an open
/// tag named `name`:
/// - Found at some depth: every tag *above* that depth is still open and
///   never got its own closing tag — each is implicitly closed right here
///   (rule 1: `UnclosedTag`) before the actual match is closed normally.
/// - Not found anywhere on the stack: `name`'s closing tag has no
///   corresponding open tag at all — an orphan/mismatched close (rule 2:
///   `UnexpectedCloseTag`), ignored, stack untouched.
fn close_tag(
    name: &str,
    close_start: u32,
    close_end: u32,
    stack: &mut Vec<OpenTag>,
    root: &mut Option<XmlElement>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(depth) = stack.iter().rposition(|open| open.name == name) else {
        diagnostics.push(Diagnostic {
            code: DiagCode::UnexpectedCloseTag,
            span: Some(ByteSpan {
                start: close_start,
                end: close_end,
            }),
            message: format!("closing tag </{name}> has no matching open tag; ignored"),
        });
        return;
    };

    while stack.len() > depth + 1 {
        let Some(unclosed) = stack.pop() else { break };
        diagnostics.push(Diagnostic {
            code: DiagCode::UnclosedTag,
            span: Some(ByteSpan {
                start: unclosed.start,
                end: close_start,
            }),
            message: format!(
                "<{}> was never explicitly closed; implicitly closed here",
                unclosed.name
            ),
        });
        let element = XmlElement {
            name: unclosed.name,
            span: ByteSpan {
                start: unclosed.start,
                end: close_start,
            },
            attrs: unclosed.attrs,
            children: unclosed.children,
        };
        push_completed(element, stack, root);
    }

    if let Some(matched) = stack.pop() {
        let element = XmlElement {
            name: matched.name,
            span: ByteSpan {
                start: matched.start,
                end: close_end,
            },
            attrs: matched.attrs,
            children: matched.children,
        };
        push_completed(element, stack, root);
    }
}

/// EOF (or an unrecoverable stuck parse error) reached with elements
/// still open: every remaining stack frame is unclosed (rule 1), closed
/// implicitly at `doc_end`, innermost first — so each nests correctly
/// into its own still-open parent as it's popped.
fn close_unclosed_at_eof(
    stack: &mut Vec<OpenTag>,
    root: &mut Option<XmlElement>,
    doc_end: u32,
    diagnostics: &mut Vec<Diagnostic>,
) {
    while let Some(unclosed) = stack.pop() {
        diagnostics.push(Diagnostic {
            code: DiagCode::UnclosedTag,
            span: Some(ByteSpan {
                start: unclosed.start,
                end: doc_end,
            }),
            message: format!("<{}> was never closed before end of input", unclosed.name),
        });
        let element = XmlElement {
            name: unclosed.name,
            span: ByteSpan {
                start: unclosed.start,
                end: doc_end,
            },
            attrs: unclosed.attrs,
            children: unclosed.children,
        };
        push_completed(element, stack, root);
    }
}

// ---------------------------------------------------------------------
// Attribute scanning (recovery rule 3: duplicate attribute).
// ---------------------------------------------------------------------

/// Tokenizes a start/empty tag's raw byte range `[tag_start, tag_end)`
/// into its attributes — byte-level scan (not quick-xml's own attribute
/// iterator) so a duplicate name's *second* occurrence still gets a
/// precise span, mirroring batis-xml's `scan_attributes`/
/// `attr_value_spanned` split. Every boundary this loop stops on (`=`,
/// ASCII whitespace, `>`, `/`, a quote character) is itself single-byte
/// ASCII, and none of those byte values ever occur as a continuation or
/// lead byte of a multi-byte UTF-8 sequence — so every slice taken here
/// always lands on a `source` char boundary, regardless of non-ASCII
/// content elsewhere in an attribute name/value (same property batis-xml
/// relies on for its identical scanner).
fn scan_attrs_for_tag(
    source: &str,
    tag_start: usize,
    tag_end: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<XmlAttr> {
    let bytes = source.as_bytes();
    let mut i = tag_start;

    // Skip `<` and the element's own (possibly prefixed) name.
    if i < tag_end && bytes[i] == b'<' {
        i += 1;
    }
    while i < tag_end && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' && bytes[i] != b'/' {
        i += 1;
    }

    let mut attrs: Vec<XmlAttr> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    loop {
        while i < tag_end && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= tag_end || bytes[i] == b'>' || bytes[i] == b'/' {
            break;
        }

        let name_start = i;
        while i < tag_end
            && bytes[i] != b'='
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'>'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        let name_end = i;

        while i < tag_end && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= tag_end || bytes[i] != b'=' {
            // Bare valueless attribute (malformed) — skip it, keep
            // scanning for whatever real attributes follow rather than
            // abandoning the whole tag's worth of them.
            continue;
        }
        i += 1;
        while i < tag_end && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let Some(&quote) = bytes.get(i).filter(|b| **b == b'"' || **b == b'\'') else {
            break;
        };
        i += 1;
        let value_start = i;
        while i < tag_end && bytes[i] != quote {
            i += 1;
        }
        if i >= tag_end {
            break; // unterminated attribute value
        }
        let value_end = i;
        i += 1; // consume closing quote

        let name = source[name_start..name_end].to_string();
        let value_text = source[value_start..value_end].to_string();
        let value_span = ByteSpan {
            start: value_start as u32,
            end: value_end as u32,
        };

        if !seen.insert(name.clone()) {
            diagnostics.push(Diagnostic {
                code: DiagCode::DuplicateAttribute,
                span: Some(ByteSpan {
                    start: name_start as u32,
                    end: name_end as u32,
                }),
                message: format!("duplicate '{name}' attribute; first value wins"),
            });
            continue; // recovery rule 3: first value wins, duplicate dropped
        }

        check_entities(&value_text, value_span, diagnostics);
        check_unterminated_placeholders(&value_text, value_span, diagnostics);

        attrs.push(XmlAttr {
            name,
            name_span: ByteSpan {
                start: name_start as u32,
                end: name_end as u32,
            },
            value: Spanned {
                value: value_text,
                span: value_span,
            },
        });
    }

    attrs
}

// ---------------------------------------------------------------------
// Recovery rule 5: unresolved entity reference.
// ---------------------------------------------------------------------

/// Scans `text` for every `&...;`-shaped run and validates it against
/// `quick_xml::escape::unescape` — a resolvable reference (the five
/// predefined XML entities, or a valid numeric/hex character reference)
/// produces no diagnostic; an unresolvable one (`&nbsp;`, an invalid
/// numeric codepoint) or a dangling `&` with no terminating `;` within
/// [`MAX_ENTITY_SCAN`] bytes both produce `InvalidEntity`. `text` itself
/// is never rewritten either way — the raw source text is always what
/// this crate's model stores (invariant #4); this function only decides
/// whether an anomaly gets reported alongside it.
///
/// Applied uniformly to attribute values and ordinary element text (not
/// CDATA — see `build_tree`'s `CData` arm) regardless of whether
/// quick-xml itself delivered the surrounding bytes as one `Text` event
/// or split a reference out into its own `GeneralRef` event: either way,
/// this is a byte-level re-scan of the exact same raw span, so the two
/// call sites can't drift on what counts as resolvable.
fn check_entities(text: &str, span: ByteSpan, diagnostics: &mut Vec<Diagnostic>) {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'&' {
            i += 1;
            continue;
        }
        let ref_start = i;
        let mut j = i + 1;
        let mut semi = None;
        while j < bytes.len() && j - i <= MAX_ENTITY_SCAN {
            match bytes[j] {
                b';' => {
                    semi = Some(j);
                    break;
                }
                // A second unescaped `&`, whitespace, or `<` before a `;`
                // means this was never a well-formed reference to begin
                // with — bail out early rather than scanning past it.
                b'&' | b'<' => break,
                b if b.is_ascii_whitespace() => break,
                _ => j += 1,
            }
        }
        match semi {
            Some(semi) => {
                let raw_ref = &text[ref_start..=semi];
                if quick_xml::escape::unescape(raw_ref).is_err() {
                    diagnostics.push(Diagnostic {
                        code: DiagCode::InvalidEntity,
                        span: Some(ByteSpan {
                            start: span.start + ref_start as u32,
                            end: span.start + semi as u32 + 1,
                        }),
                        message: format!(
                            "unresolvable entity reference '{raw_ref}'; kept as literal text"
                        ),
                    });
                }
                i = semi + 1;
            }
            None => {
                diagnostics.push(Diagnostic {
                    code: DiagCode::InvalidEntity,
                    span: Some(ByteSpan {
                        start: span.start + ref_start as u32,
                        end: span.start + ref_start as u32 + 1,
                    }),
                    message: "dangling '&' without a terminating ';' is not a well-formed \
                              entity reference; kept as literal text"
                        .to_string(),
                });
                i = ref_start + 1;
            }
        }
    }
}

// ---------------------------------------------------------------------
// Recovery rule 6: unterminated `${`/`#{` placeholder.
// ---------------------------------------------------------------------

/// Quote-aware scan for one `${`/`#{` placeholder's matching close brace,
/// starting at `open_pos` (the index in `bytes` of the `$`/`#` byte
/// itself — caller guarantees `bytes[open_pos + 1] == b'{'`). Returns the
/// index just past the matching `}` when the opener closes (`Some`), or
/// `None` when it never does before `bytes` ends.
///
/// Brace characters inside a single- or double-quoted SpEL string literal
/// (`#{'{'}`, `#{map['{']}`) don't count toward nesting depth — an
/// M0a-deferred U1 finding: without this, a literal `{`/`}` sitting inside
/// a quoted string produced a spurious `UnterminatedPlaceholder` even
/// though the expression's own braces were perfectly balanced. A quote
/// character is tracked by simple toggle (opens on the first `'`/`"` seen
/// outside any quote, closes on the next matching one) — SpEL's `''`
/// escaped-quote-inside-a-string-literal syntax isn't specially handled,
/// but every brace strictly *between* a quote's open and its next matching
/// quote is exactly what both worked examples above need ignored.
///
/// Quote-as-string-delimiter is a **SpEL** (`#{}`) notion only — inside a
/// plain `${prop:default}` property placeholder, `'`/`"` are ordinary
/// literal characters (Spring default values routinely contain an
/// apostrophe, e.g. `${admin.name:O'Reilly}`). `bytes[open_pos]` tells us
/// which opener this is, so quote-tracking is only enabled for `#{` —
/// `${` always matches braces literally, same as before quote-awareness
/// was added.
///
/// Nested non-quoted placeholders (`${a.${b}}`) are still tracked by depth,
/// not by the first unquoted `}` found — unchanged from before this fix.
///
/// **M1 carry-over fix**: a *nested* `${`/`#{` opener gets its own
/// quote-awareness, matching its own two-char prefix, rather than
/// inheriting whichever opener started the whole scan. Before this fix, a
/// `${}` default value wrapping a `#{'}'}` SpEL sub-expression (a SpEL
/// string literal whose value is a literal `}`) broke: the *outer* `${`
/// scan isn't quote-aware, so it counted every `{`/`}` byte literally —
/// including the ones protected by the *inner* `#{}`'s own quotes — and
/// closed one `}` early, leaving the true closing brace stranded as
/// trailing text. Concretely, `${x:#{'}'}}` now closes at the very last
/// `}` (matching plain-string-`}` needs a `#{...}` sub-scan to know it's
/// protected), not at the first unquoted-looking one. Implemented as an
/// explicit heap-allocated stack of `(quote_aware, depth, quote)` frames —
/// one pushed per nested `${`/`#{` opener encountered, each carrying its
/// own quote-awareness — rather than a recursive function call, so
/// pathologically deep opener nesting inside a single literal (a hostile
/// SB-16 input) grows the heap, not the call stack: no stack-overflow
/// abort no matter how deep the input nests. Nesting depth is additionally
/// capped at [`crate::DEPTH_LIMIT`] (reusing the same constant this crate's
/// tree recursion already bounds itself by) — beyond that many nested
/// openers in one literal, the scan gives up and reports unterminated
/// (`None`), same outcome as any other malformed/hostile input; no new
/// `DiagCode` needed for this, `UnterminatedPlaceholder` already covers
/// "couldn't find a well-formed close" for its caller.
///
/// `pub(crate)`: shared with P9's own extraction
/// (`inject_value::build_value_lit`, via
/// `extract_placeholders_and_spel_refs`) — the two layers scan different
/// text (this one scans as `build_tree` walks raw XML text/attribute
/// values; P9 scans the already-assembled `ValueLit` text) but must agree
/// on what counts as a *closed* `${}`/`#{}` expression, so the boundary
/// logic itself lives once, here.
pub(crate) fn scan_braced_expr(bytes: &[u8], open_pos: usize) -> Option<usize> {
    let mut quote_aware = bytes[open_pos] == b'#';
    let mut depth = 1i32;
    let mut quote: Option<u8> = None;
    // Saved (quote_aware, depth, quote) for each enclosing opener, pushed
    // when a nested `${`/`#{` is entered — see this function's own doc
    // comment for why this is an explicit stack rather than a recursive
    // call.
    let mut parents: Vec<(bool, i32, Option<u8>)> = Vec::new();
    let mut j = open_pos + 2;
    while j < bytes.len() {
        let b = bytes[j];
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
            j += 1;
            continue;
        }
        if (b == b'$' || b == b'#') && j + 1 < bytes.len() && bytes[j + 1] == b'{' {
            if parents.len() >= crate::DEPTH_LIMIT as usize {
                return None; // pathologically deep nested opener chain — unterminated
            }
            parents.push((quote_aware, depth, quote));
            quote_aware = b == b'#';
            depth = 1;
            quote = None;
            j += 2;
            continue;
        }
        match b {
            b'\'' | b'"' if quote_aware => quote = Some(b),
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    match parents.pop() {
                        Some((pq, pd, pquote)) => {
                            quote_aware = pq;
                            depth = pd;
                            quote = pquote;
                        }
                        None => return Some(j + 1),
                    }
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// Scans `text` for every `${`/`#{` opener and checks (via
/// [`scan_braced_expr`]) that its own brace nesting closes before `text`
/// ends — quote-aware only for a `#{` (SpEL) opener; a `${` property
/// placeholder always matches braces literally (see `scan_braced_expr`'s
/// own doc comment for why). An opener that never closes gets
/// `UnterminatedPlaceholder`, spanning from the opener to the end of this
/// text run; `text` is kept verbatim either way (this layer only collects
/// the *diagnostic* — extracting the actual placeholder key is P9's job,
/// once `ValueLit`'s single builder exists to hold it).
fn check_unterminated_placeholders(text: &str, span: ByteSpan, diagnostics: &mut Vec<Diagnostic>) {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if (bytes[i] == b'$' || bytes[i] == b'#') && bytes[i + 1] == b'{' {
            match scan_braced_expr(bytes, i) {
                Some(next) => {
                    i = next;
                    continue;
                }
                None => {
                    diagnostics.push(Diagnostic {
                        code: DiagCode::UnterminatedPlaceholder,
                        span: Some(ByteSpan {
                            start: span.start + i as u32,
                            end: span.end,
                        }),
                        message: format!(
                            "'{}' placeholder is missing its closing '}}'; kept as literal text",
                            &text[i..i + 2]
                        ),
                    });
                    return; // nothing left in this run can close it either
                }
            }
        }
        i += 1;
    }
}

/// Extends [`build_tree`]'s `pending_placeholder_span` accumulator with one
/// more raw `Text`/`GeneralRef`/`CData` run's span — the entity-split
/// harvesting regression fix (see `build_tree`'s own doc comment on that
/// field). `raw_span` is **byte-contiguous** with the pending run
/// (`pending.end == raw_span.start`) exactly when no other source bytes
/// (a comment, a `<![CDATA[`/`]]>` delimiter, an intervening element's own
/// markup) sit between them in `source` — in that case it's folded into the
/// same pending run rather than scanned on its own, so an opener and closer
/// split across an entity reference (`#{flagA &amp;&amp; flagB}`) are seen
/// as one contiguous slice of `source`. A **non**-contiguous `raw_span`
/// (or no pending run yet) first flushes whatever was pending — scanning it
/// now, since nothing later can ever join it — then starts a fresh pending
/// run at `raw_span`.
fn extend_pending_placeholder_span(
    pending: &mut Option<ByteSpan>,
    raw_span: ByteSpan,
    source: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match *pending {
        Some(p) if p.end == raw_span.start => {
            *pending = Some(ByteSpan {
                start: p.start,
                end: raw_span.end,
            });
        }
        Some(p) => {
            check_unterminated_placeholders(
                &source[p.start as usize..p.end as usize],
                p,
                diagnostics,
            );
            *pending = Some(raw_span);
        }
        None => {
            *pending = Some(raw_span);
        }
    }
}

/// Scans and clears whatever run [`extend_pending_placeholder_span`] has
/// accumulated so far, if any — called once contiguity can no longer
/// continue (a non-text event) or at end of input, so a trailing pending
/// run is never left unscanned.
fn flush_pending_placeholder_span(
    pending: &mut Option<ByteSpan>,
    source: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(span) = pending.take() {
        check_unterminated_placeholders(
            &source[span.start as usize..span.end as usize],
            span,
            diagnostics,
        );
    }
}

// ---------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------
//
// These stay in-module (rather than an external `tests/u1_events.rs`)
// because every type/function this unit exports is `pub(crate)` — a
// seam not visible from an external integration-test binary, same
// rationale `src/model.rs`'s own `#[cfg(test)] mod tests` documents for
// `BeansFileCtx`/`BeanCtx`. `tests/u1_events.rs` carries a pointer here.

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(source: &str, span: ByteSpan) -> &str {
        &source[span.start as usize..span.end as usize]
    }

    fn diag_codes(result: &TreeResult) -> Vec<DiagCode> {
        result.diagnostics.iter().map(|d| d.code).collect()
    }

    // -------------------------------------------------------------
    // Recovery rule table.
    // -------------------------------------------------------------

    #[test]
    fn u1_recovery_unclosed_tag_implicit_close_at_parent_end() {
        let source = "<beans><bean id=\"a\"></beans>";
        let result = build_tree(source);
        assert_eq!(diag_codes(&result), vec![DiagCode::UnclosedTag]);
        let root = result.root.expect("root element found");
        assert_eq!(root.name, "beans");
        assert_eq!(root.children.len(), 1);
        let XmlNode::Element(bean) = &root.children[0] else {
            panic!("expected an element child")
        };
        assert_eq!(bean.name, "bean");
        assert!(bean.children.is_empty());
    }

    #[test]
    fn u1_recovery_unclosed_tag_implicit_close_at_eof() {
        let source = "<beans><bean id=\"a\">";
        let result = build_tree(source);
        // Both <bean> and <beans> are unclosed at EOF.
        assert_eq!(
            diag_codes(&result),
            vec![DiagCode::UnclosedTag, DiagCode::UnclosedTag]
        );
        let root = result.root.expect("root element found despite truncation");
        assert_eq!(root.name, "beans");
        assert_eq!(root.span.end, source.len() as u32);
        assert_eq!(root.children.len(), 1);
    }

    #[test]
    fn u1_recovery_orphan_close_tag_ignored() {
        let source = "<beans></foo></beans>";
        let result = build_tree(source);
        assert_eq!(diag_codes(&result), vec![DiagCode::UnexpectedCloseTag]);
        let root = result.root.expect("root element found");
        assert_eq!(root.name, "beans");
        // The orphan </foo> produced no structure at all.
        assert!(root.children.is_empty());
        assert_eq!(root.span.end, source.len() as u32);
    }

    #[test]
    fn u1_recovery_duplicate_attribute_first_wins() {
        let source = "<bean id=\"a\" id=\"b\"/>";
        let result = build_tree(source);
        assert_eq!(diag_codes(&result), vec![DiagCode::DuplicateAttribute]);
        let root = result.root.expect("root element found");
        assert_eq!(root.attrs.len(), 1);
        assert_eq!(root.attrs[0].name, "id");
        assert_eq!(root.attrs[0].value.value, "a");
    }

    #[test]
    fn u1_recovery_non_xml_residue_resyncs_and_structure_recovers() {
        // `<!zzz>` is not a valid comment/CDATA/DOCTYPE/PI opener — quick-xml
        // rejects it, and recovery rule 4 says: resynchronize at the next
        // recognizable token (no dedicated diagnostic code exists for this
        // rule). Both sibling <bean> elements must still show up.
        let source = "<beans><bean id=\"a\"/><!zzz><bean id=\"b\"/></beans>";
        let result = build_tree(source);
        let root = result.root.expect("root element found");
        assert_eq!(root.name, "beans");
        let element_children: Vec<&str> = root
            .children
            .iter()
            .filter_map(|c| match c {
                XmlNode::Element(e) => Some(e.name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(element_children, vec!["bean", "bean"]);
    }

    #[test]
    fn u1_recovery_span_after_resync_slices_correct_source_text() {
        // Guards the `base_offset` arithmetic through a rule-4 resync
        // specifically (not just the structural recovery
        // `u1_recovery_non_xml_residue_resyncs_and_structure_recovers`
        // already covers): after `<!zzz>` forces `build_tree` to skip ahead
        // and construct a fresh `Reader` starting further into `working`,
        // every span the resynchronized reader reports must still be
        // translated back into *this* `source`'s own coordinate space —
        // the BOM-skew bug class this arithmetic is also responsible for
        // (see `build_tree`'s own doc comment on `base_offset`).
        let source = "<beans><bean id=\"a\"/><!zzz><bean id=\"recovered\" note=\"x\"/></beans>";
        let result = build_tree(source);
        let root = result.root.expect("root element found");
        let element_children: Vec<&XmlElement> = root
            .children
            .iter()
            .filter_map(|c| match c {
                XmlNode::Element(e) => Some(e),
                _ => None,
            })
            .collect();
        assert_eq!(element_children.len(), 2);
        let recovered = element_children[1];
        assert_eq!(recovered.name, "bean");
        assert_eq!(
            slice(source, recovered.span),
            "<bean id=\"recovered\" note=\"x\"/>"
        );
        let id_attr = recovered
            .attrs
            .iter()
            .find(|a| a.name == "id")
            .expect("id attribute present");
        assert_eq!(slice(source, id_attr.value.span), "recovered");
        let note_attr = recovered
            .attrs
            .iter()
            .find(|a| a.name == "note")
            .expect("note attribute present");
        assert_eq!(slice(source, note_attr.value.span), "x");
    }

    #[test]
    fn u1_span_bom_prefixed_input_slices_correct_source_text() {
        // `build_tree` strips a leading BOM from `working` before handing
        // it to `Reader`, then adds `base_offset` back into every position
        // the reader reports before indexing into `source` (which still has
        // the BOM). The existing proptests only cover panic-freedom on
        // arbitrary/BOM-laced input; this pins that the compensation
        // actually lands on the *correct* slice, not just that it doesn't
        // panic or underflow.
        let source = "\u{FEFF}<beans><bean id=\"a\">hello</bean></beans>";
        let result = build_tree(source);
        let root = result.root.expect("root element found");
        assert_eq!(root.name, "beans");
        assert_eq!(
            slice(source, root.span),
            "<beans><bean id=\"a\">hello</bean></beans>"
        );
        let XmlNode::Element(bean) = &root.children[0] else {
            panic!("expected an element child")
        };
        assert_eq!(slice(source, bean.span), "<bean id=\"a\">hello</bean>");
        assert_eq!(slice(source, bean.attrs[0].value.span), "a");
        let XmlNode::Text(text) = &bean.children[0] else {
            panic!("expected a text child")
        };
        assert_eq!(slice(source, text.span), "hello");
    }

    #[test]
    fn u1_recovery_invalid_entity_kept_raw_and_diagnosed() {
        // Negative case (build plan U1 test (a)): an unresolvable named
        // entity reference is kept verbatim in the text, plus InvalidEntity.
        let source = "<value>&badentity;</value>";
        let result = build_tree(source);
        assert_eq!(diag_codes(&result), vec![DiagCode::InvalidEntity]);
        let root = result.root.expect("root element found");
        assert_eq!(root.children.len(), 1);
        let XmlNode::Text(text) = &root.children[0] else {
            panic!("expected a text child")
        };
        assert_eq!(text.value, "&badentity;");
        assert_eq!(slice(source, text.span), "&badentity;");
    }

    #[test]
    fn u1_recovery_valid_predefined_entity_no_diagnostic() {
        let source = "<value>a &amp; b</value>";
        let result = build_tree(source);
        assert!(
            result.diagnostics.is_empty(),
            "predefined entity should not be diagnosed: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn u1_recovery_dangling_ampersand_kept_raw_and_diagnosed() {
        let source = "<value>a & b</value>";
        let result = build_tree(source);
        assert_eq!(diag_codes(&result), vec![DiagCode::InvalidEntity]);
    }

    #[test]
    fn u1_recovery_unterminated_dollar_placeholder_kept_raw_and_diagnosed() {
        // Negative case (build plan U1 test (a)): literal test fixture
        // named directly in the spec/build plan.
        let source = "<value>${unterminated</value>";
        let result = build_tree(source);
        assert_eq!(diag_codes(&result), vec![DiagCode::UnterminatedPlaceholder]);
        let root = result.root.expect("root element found");
        let XmlNode::Text(text) = &root.children[0] else {
            panic!("expected a text child")
        };
        assert_eq!(text.value, "${unterminated");
    }

    #[test]
    fn u1_recovery_unterminated_spel_placeholder_kept_raw_and_diagnosed() {
        let source = "<value>#{unterminated</value>";
        let result = build_tree(source);
        assert_eq!(diag_codes(&result), vec![DiagCode::UnterminatedPlaceholder]);
    }

    #[test]
    fn u1_recovery_terminated_placeholder_no_diagnostic() {
        let source = "<value>${a.b} and #{beanA.m()}</value>";
        let result = build_tree(source);
        assert!(
            result.diagnostics.is_empty(),
            "terminated placeholders should not be diagnosed: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn u1_recovery_spel_string_literal_brace_not_flagged_unterminated() {
        // M0a-deferred finding, fixed here: a literal `{`/`}` sitting
        // inside a quoted SpEL string literal must not be mistaken for
        // unbalanced placeholder nesting — the expression's own braces
        // (`#{` ... `}`) are perfectly matched.
        let source = "<value>#{'{'}</value>";
        let result = build_tree(source);
        assert!(
            result.diagnostics.is_empty(),
            "quoted brace literal must not be flagged: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn u1_recovery_spel_map_index_string_literal_brace_not_flagged_unterminated() {
        let source = "<value>#{map['{']}</value>";
        let result = build_tree(source);
        assert!(
            result.diagnostics.is_empty(),
            "quoted brace literal inside a map index must not be flagged: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn u1_recovery_spel_double_quoted_string_literal_brace_not_flagged_unterminated() {
        let source = "<value>#{\"{\"}</value>";
        let result = build_tree(source);
        assert!(
            result.diagnostics.is_empty(),
            "double-quoted brace literal must not be flagged: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn u1_recovery_genuinely_unterminated_placeholder_after_quoted_literal_still_flagged() {
        // Guards against the quote-awareness fix over-correcting into never
        // flagging anything: a quoted string literal's own brace is
        // ignored, but the expression as a whole genuinely never closes.
        let source = "<value>#{'{' + unterminated</value>";
        let result = build_tree(source);
        assert_eq!(diag_codes(&result), vec![DiagCode::UnterminatedPlaceholder]);
    }

    #[test]
    fn u1_recovery_property_placeholder_with_apostrophe_default_no_diagnostic() {
        // Quote-as-string-delimiter is a SpEL (`#{}`) notion only. A `${}`
        // property placeholder's default value routinely contains an
        // apostrophe (Spring's `${prop:default}` syntax) — that must not be
        // mistaken for a SpEL string-literal opener, or the real closing
        // `}` gets swallowed and a spurious UnterminatedPlaceholder fires
        // even though the braces are perfectly balanced.
        let source = "<value>${admin.name:O'Reilly}</value>";
        let result = build_tree(source);
        assert!(
            result.diagnostics.is_empty(),
            "a balanced ${{}} containing an apostrophe must not be flagged: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn u1_recovery_property_placeholder_with_double_quote_default_no_diagnostic() {
        let source = "<value>${msg:say \"hi\"}</value>";
        let result = build_tree(source);
        assert!(
            result.diagnostics.is_empty(),
            "a balanced ${{}} containing a double quote must not be flagged: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn u1_recovery_nested_terminated_placeholder_no_diagnostic() {
        let source = "<value>${a.${b}}</value>";
        let result = build_tree(source);
        assert!(
            result.diagnostics.is_empty(),
            "a fully-closed nested placeholder should not be diagnosed: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn u1_recovery_dollar_default_containing_quoted_brace_spel_no_diagnostic() {
        // M1 carry-over fix (P9/SB-13): a `${}` default value wrapping a
        // `#{'}'}` SpEL sub-expression (a string literal whose value is a
        // literal `}`) — the outer `${` isn't itself quote-aware, but its
        // own nested `#{` opener must get its *own* quote-awareness so the
        // quoted `}` inside doesn't get mistaken for the outer's close.
        let source = "<value>${x:#{'}'}}</value>";
        let result = build_tree(source);
        assert!(
            result.diagnostics.is_empty(),
            "a nested quote-protected brace must not be flagged: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn u1_recovery_hash_default_containing_quoted_brace_dollar_no_diagnostic() {
        // Same carry-over case, opener types swapped: a `#{}` wrapping a
        // nested `${...}` whose own further-nested `#{'}'}` protects a
        // literal `}` via quotes two levels down.
        let source = "<value>#{${x:#{'}'}}}</value>";
        let result = build_tree(source);
        assert!(
            result.diagnostics.is_empty(),
            "a two-level nested quote-protected brace must not be flagged: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn u1_recovery_unterminated_placeholder_in_attribute_value() {
        let source = "<property name=\"x\" value=\"${unterminated\"/>";
        let result = build_tree(source);
        assert_eq!(diag_codes(&result), vec![DiagCode::UnterminatedPlaceholder]);
        let root = result.root.expect("root element found");
        let value_attr = root
            .attrs
            .iter()
            .find(|a| a.name == "value")
            .expect("value attribute present");
        assert_eq!(value_attr.value.value, "${unterminated");
    }

    #[test]
    fn u1_recovery_unterminated_placeholder_inside_cdata_kept_raw_and_diagnosed() {
        // The `CData` arm's own doc comment claims the placeholder scan
        // still applies inside CDATA (only `check_entities` is skipped
        // there) — pin that behavior directly rather than leaving it
        // asserted only in a comment.
        let source = "<value><![CDATA[${unterminated]]></value>";
        let result = build_tree(source);
        assert_eq!(diag_codes(&result), vec![DiagCode::UnterminatedPlaceholder]);
        let root = result.root.expect("root element found");
        let XmlNode::Text(text) = &root.children[0] else {
            panic!("expected a text child")
        };
        assert_eq!(text.value, "${unterminated");
        assert_eq!(slice(source, text.span), "${unterminated");
    }

    #[test]
    fn u1_recovery_entity_like_text_inside_cdata_not_checked() {
        // Invariant #4 / this module's doc comment: CDATA content is never
        // entity-interpreted, so the same raw text that triggers
        // `InvalidEntity` as ordinary element text (see
        // `u1_recovery_invalid_entity_kept_raw_and_diagnosed`) must produce
        // no diagnostic at all when it instead arrives inside a CDATA
        // section, and must be kept byte-for-byte verbatim including the
        // literal `&`.
        let source = "<value><![CDATA[&badentity;]]></value>";
        let result = build_tree(source);
        assert!(
            result.diagnostics.is_empty(),
            "CDATA content must not be entity-checked: {:?}",
            result.diagnostics
        );
        let root = result.root.expect("root element found");
        let XmlNode::Text(text) = &root.children[0] else {
            panic!("expected a text child")
        };
        assert_eq!(text.value, "&badentity;");
    }

    // -------------------------------------------------------------
    // Entity-split harvesting regression (M1c fix): `&amp;` splits one
    // logical text run into several byte-contiguous `Text`/`GeneralRef`
    // runs — `pending_placeholder_span` (this module's own doc comment on
    // `build_tree`) must coalesce them before scanning for an unterminated
    // `${}`/`#{}`, or a perfectly well-formed expression whose opener and
    // closer land in different runs gets spuriously flagged.
    // -------------------------------------------------------------

    #[test]
    fn u1_entity_split_spel_expression_not_flagged_unterminated() {
        let source = "<value>#{flagA &amp;&amp; flagB}</value>";
        let result = build_tree(source);
        assert!(
            result.diagnostics.is_empty(),
            "entity-split but well-formed #{{}} must not be diagnosed: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn u1_entity_split_dollar_placeholder_not_flagged_unterminated() {
        let source = "<value>${url:http://x?a=1&amp;b=2}</value>";
        let result = build_tree(source);
        assert!(
            result.diagnostics.is_empty(),
            "entity-split but well-formed ${{}} must not be diagnosed: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn u1_entity_split_still_diagnoses_a_genuinely_unterminated_placeholder() {
        // Regression guard the other direction: coalescing contiguous runs
        // must not suppress a real UnterminatedPlaceholder just because an
        // entity reference happens to sit somewhere inside the (still
        // unterminated) run.
        let source = "<value>#{flagA &amp;&amp; unterminated</value>";
        let result = build_tree(source);
        assert_eq!(diag_codes(&result), vec![DiagCode::UnterminatedPlaceholder]);
    }

    // -------------------------------------------------------------
    // Span slice == event text (table).
    // -------------------------------------------------------------

    #[test]
    fn u1_span_start_tag_slice_matches_source_text() {
        let source = "<bean id=\"a\">text</bean>";
        let result = build_tree(source);
        let root = result.root.unwrap();
        assert_eq!(slice(source, root.span), "<bean id=\"a\">text</bean>");
    }

    #[test]
    fn u1_span_self_closed_tag_slice_matches_source_text() {
        let source = "<bean id=\"a\"/>";
        let result = build_tree(source);
        let root = result.root.unwrap();
        assert_eq!(slice(source, root.span), source);
    }

    #[test]
    fn u1_span_attribute_value_slice_matches_source_text() {
        let source = "<bean id=\"myBean\"/>";
        let result = build_tree(source);
        let root = result.root.unwrap();
        assert_eq!(slice(source, root.attrs[0].value.span), "myBean");
    }

    #[test]
    fn u1_span_text_node_slice_matches_source_text() {
        let source = "<description>hello world</description>";
        let result = build_tree(source);
        let root = result.root.unwrap();
        let XmlNode::Text(text) = &root.children[0] else {
            panic!("expected a text child")
        };
        assert_eq!(slice(source, text.span), "hello world");
    }

    #[test]
    fn u1_span_multibyte_text_slice_matches_source_text() {
        // Multibyte (Korean) content — invariant #2/#4 must hold on
        // decoded-UTF-8 offsets, not naive char counts.
        let source = "<description>안녕하세요 세계</description>";
        let result = build_tree(source);
        let root = result.root.unwrap();
        let XmlNode::Text(text) = &root.children[0] else {
            panic!("expected a text child")
        };
        assert_eq!(slice(source, text.span), "안녕하세요 세계");
        assert_eq!(text.value, "안녕하세요 세계");
    }

    #[test]
    fn u1_span_cdata_slice_matches_inner_content_only() {
        // The `CData` arm hand-computes the inner-content span (excludes
        // the `<![CDATA[`/`]]>` delimiters via hardcoded offsets) — pin
        // that arithmetic against a concrete fixture rather than leaving
        // it verified only by inline commentary.
        let source = "<value><![CDATA[hello]]></value>";
        let result = build_tree(source);
        let root = result.root.unwrap();
        let XmlNode::Text(text) = &root.children[0] else {
            panic!("expected a text child")
        };
        assert_eq!(text.value, "hello");
        assert_eq!(slice(source, text.span), "hello");
    }

    #[test]
    fn u1_span_empty_cdata_slice_is_empty_and_does_not_underflow() {
        // Degenerate case for the `.min(inner_start)`/`.saturating_sub(3)`
        // guards: an empty CDATA section's inner span must collapse to a
        // valid zero-length range, not underflow or panic.
        let source = "<value><![CDATA[]]></value>";
        let result = build_tree(source);
        let root = result.root.unwrap();
        let XmlNode::Text(text) = &root.children[0] else {
            panic!("expected a text child")
        };
        assert_eq!(text.value, "");
        assert_eq!(text.span.start, text.span.end);
        assert_eq!(slice(source, text.span), "");
    }

    #[test]
    fn u1_span_multibyte_cdata_slice_matches_source_text() {
        // Same invariant #2/#4 multibyte requirement as ordinary text
        // (`u1_span_multibyte_text_slice_matches_source_text`), but through
        // the CDATA arm's own byte-offset arithmetic rather than the plain
        // `Text` arm's.
        let source = "<value><![CDATA[한글]]></value>";
        let result = build_tree(source);
        let root = result.root.unwrap();
        let XmlNode::Text(text) = &root.children[0] else {
            panic!("expected a text child")
        };
        assert_eq!(text.value, "한글");
        assert_eq!(slice(source, text.span), "한글");
    }

    #[test]
    fn u1_span_nested_element_slice_matches_source_text() {
        let source = "<beans><bean id=\"a\"><property name=\"p\" /></bean></beans>";
        let result = build_tree(source);
        let root = result.root.unwrap();
        let XmlNode::Element(bean) = &root.children[0] else {
            panic!("expected an element child")
        };
        assert_eq!(
            slice(source, bean.span),
            "<bean id=\"a\"><property name=\"p\" /></bean>"
        );
        let XmlNode::Element(property) = &bean.children[0] else {
            panic!("expected an element child")
        };
        assert_eq!(slice(source, property.span), "<property name=\"p\" />");
    }

    // -------------------------------------------------------------
    // proptest: panic-free on arbitrary input (invariant #1, partial).
    // -------------------------------------------------------------

    // -------------------------------------------------------------
    // P0 regression: deep-input stack overflow on `Drop` (invariant #1).
    // -------------------------------------------------------------

    #[test]
    fn u1_deeply_nested_input_does_not_overflow_stack_on_drop() {
        // Root-cause regression test for the confirmed-live P0 bug: this
        // module builds its tree iteratively (no call-stack recursion while
        // *building*), but `XmlElement { children: Vec<XmlNode> }` is an
        // owned recursive tree, so its compiler-derived `Drop` recurses one
        // call-stack frame per nesting level on *teardown*. 60_000 nested
        // elements (~660 KB, comfortably under `MAX_INPUT_BYTES`) reliably
        // overflowed the stack and aborted the process before `build_tree`
        // itself capped tree depth at `MAX_TREE_DEPTH` — confirmed by
        // running this exact test against the pre-fix code (`git stash` it,
        // `cargo test --lib u1_deeply_nested_input_does_not_overflow_stack_on_drop`):
        // it aborted with "thread ... has overflowed its stack"; with the
        // fix it passes.
        const N: usize = 60_000;
        let mut source = String::with_capacity(N * 11);
        for _ in 0..N {
            source.push_str("<foo>");
        }
        for _ in 0..N {
            source.push_str("</foo>");
        }

        let result = build_tree(&source);

        assert!(result.root.is_some(), "root element still found");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == DiagCode::NestingLimitExceeded),
            "expected a NestingLimitExceeded diagnostic: {:?}",
            result.diagnostics
        );
        // `result` (and its `Option<XmlElement>`) drops here, at the end of
        // this scope — that drop, not `build_tree` itself, is what must
        // stay within a bounded number of stack frames.
    }

    proptest::proptest! {
        #[test]
        fn u1_proptest_arbitrary_unicode_str_never_panics(s in ".{0,400}") {
            let _ = build_tree(&s);
        }

        #[test]
        fn u1_proptest_arbitrary_bytes_lossy_decoded_never_panics(
            bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..400)
        ) {
            let s = String::from_utf8_lossy(&bytes).into_owned();
            let _ = build_tree(&s);
        }

        #[test]
        fn u1_proptest_beans_like_shapes_never_panic(
            depth in 0usize..30,
            junk in ".{0,20}",
        ) {
            let mut s = String::new();
            for _ in 0..depth {
                s.push_str("<bean id=\"x\">");
            }
            s.push_str(&junk);
            for _ in 0..depth {
                s.push_str("</bean>");
            }
            let _ = build_tree(&s);
        }
    }
}
