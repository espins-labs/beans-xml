//! Unit **U5a** — `InjectValue` core (SB-06): `value`/`ref`/`idref`/`inner`/
//! `null` — every `InjectValue` variant except `Collection` (SB-07, U5b,
//! the *serial* next step on this same spine, not a parallel leaf — now
//! landed: see `crate::collection`, wired into this module's own
//! [`parse_inject_value_child`] match below). Build plan's own description
//! of this unit: "recursion heart (inner → **shared `parse_bean`**). M0a" — this is
//! the first unit whose own recursion needs a [`crate::DEPTH_LIMIT`] guard
//! (`events::build_tree`'s own doc comment: "`DEPTH_LIMIT` applies once a
//! later unit recursively walks this tree to build nested model values" —
//! this is that later unit).
//!
//! Two things this module owns, per the internal build plan's
//! U5a row:
//!
//! 1. **The single `ValueLit` builder** ([`build_value_lit`]): every literal
//!    value this crate ever produces — a `<value>` child, a `value=`
//!    shorthand attribute, a p/c-namespace literal attribute (P2), and a
//!    collection item/`<prop>`/`<entry>` literal (U5b) — flows through this
//!    one function. **P9 (SB-13)** augments this exact function's body with
//!    `${}`/`#{}` extraction ([`extract_placeholders_and_spel_refs`]) —
//!    build plan: "augment U5's single `ValueLit` builder" (a separate post-walk
//!    is explicitly ruled out) — so every one of those call sites gets
//!    `placeholders`/`spel_refs` populated uniformly, for free, without
//!    each one needing its own extraction call.
//! 2. **The recursive value-child dispatcher** ([`parse_inject_value_child`]):
//!    given one already-identified "value-shaped" child element (`<value>`/
//!    `<ref>`/`<idref>`/`<null>`/`<bean>`), resolves it to an `InjectValue`.
//!    `<bean>` delegates to the shared [`crate::bean::parse_bean`] (build
//!    plan "recursion unification": never reimplemented) rather than re-walking a
//!    bean's own attributes/children a second time. `depth` is this
//!    function's own [`crate::DEPTH_LIMIT`] guard: once U6/U7 wire
//!    `<property>`/`<constructor-arg>` to call back into this function at
//!    `depth + 1` for their own value — and that value can itself be
//!    another `<bean>` whose own future properties recurse again — this is
//!    the single choke point that keeps that mutual recursion bounded, so
//!    it lives here rather than in `bean::parse_bean` itself.
//!
//! Two composable pieces for the `value=`/`ref=` shorthand-attribute case
//! (`<property value=.../>`, `<property ref=.../>`, and `<constructor-arg>`'s
//! identical pair) are also here ([`value_lit_from_attr`],
//! [`ref_from_attr`]) — U6/U7 (SB-04/05) call these directly rather than
//! reimplementing the same attribute-to-`InjectValue` conversion. This unit
//! deliberately does **not** bundle them into one "resolve an owner
//! element's injected value" function: deciding what counts as a conflict
//! between the two (`ConflictingValueAndRef`) is SB-04/05's own test-owned
//! edge case (build plan's SB-04 row), not this one's.
//!
//! Every type/function here is `pub(crate)` — a seam not visible from an
//! external integration-test binary, the exact situation `tests/u1_events.rs`
//! and `tests/u2_encoding.rs` already document for `events`/`encoding`. The
//! real U5a test suite — per-variant snapshots, the inner-bean *structural*
//! snapshot, and the depth/`DEPTH_LIMIT` proptests — lives in this file's
//! own `#[cfg(test)] mod tests`; `tests/u5a_inject_value.rs` carries the
//! same pointer-plus-smoke-test shape those two files already established.
//!
//! U6 (`crate::property`) is now the first real production call site for
//! this module's entry points (`parse_inject_value_child`,
//! `value_lit_from_attr`, `ref_from_attr`) — U7 (`ConstructorArg`, SB-05)
//! wires the same three for its own shorthand-attribute pair once it lands.

use crate::dispatch::{
    element_text_segments, find_attr, is_beans_ns, merge_text_segments, resolve_qname,
    spanned_attr, NsScope,
};
use crate::events::{scan_braced_expr, XmlAttr, XmlElement};
use crate::model::{
    BeanRef, ByteSpan, ClassRef, DiagCode, Diagnostic, InjectValue, RefKind, Spanned, ValueLit,
};

// ---------------------------------------------------------------------
// The single ValueLit builder.
// ---------------------------------------------------------------------

/// Builds a [`ValueLit`] from already-resolved pieces — the one
/// construction site every literal value in this crate goes through (see
/// this module's own doc comment). `span` is the literal's own extent (the
/// `<value>` element's full span, or just `text.span` for a `value=`
/// shorthand attribute, which has no separate wrapping element to span).
/// `placeholders`/`spel_refs` come from [`extract_placeholders_and_spel_refs`]
/// run over `text` — P9's (SB-13) augmentation of this one builder, per
/// this module's own doc comment. `text` is used as its own sole
/// extraction segment here, which is exact for this function's only
/// caller shapes (a `value=` shorthand attribute's one contiguous
/// `Spanned<String>`, or a test literal built the same way) — an element
/// with more than one raw text run (comment- or CDATA-adjacent text
/// inside `<value>`/`<prop>`) must go through
/// [`build_value_lit_from_segments`] instead, since `text` alone can no
/// longer carry per-run span boundaries once merged (see
/// `dispatch::element_text_segments`'s doc comment).
pub(crate) fn build_value_lit(
    text: Spanned<String>,
    span: ByteSpan,
    value_type: Option<Spanned<ClassRef>>,
) -> ValueLit {
    let (placeholders, spel_refs) = extract_placeholders_and_spel_refs(std::slice::from_ref(&text));
    ValueLit {
        span,
        text,
        value_type,
        placeholders,
        spel_refs,
    }
}

/// Same as [`build_value_lit`], but for a caller that has an element's
/// **unmerged** raw text segments on hand (`dispatch::element_text_segments`)
/// — `${}`/`#{}` extraction scans each segment separately (each one's own
/// span is exact; see that function's doc comment for why merging first
/// would corrupt the offsets), while `text` (typically
/// `dispatch::merge_text_segments(segments, ...)`) still becomes the
/// `ValueLit`'s own display text exactly as before.
pub(crate) fn build_value_lit_from_segments(
    segments: &[Spanned<String>],
    text: Spanned<String>,
    span: ByteSpan,
    value_type: Option<Spanned<ClassRef>>,
) -> ValueLit {
    let (placeholders, spel_refs) = extract_placeholders_and_spel_refs(segments);
    ValueLit {
        span,
        text,
        value_type,
        placeholders,
        spel_refs,
    }
}

// ---------------------------------------------------------------------
// P9 (SB-13): `${}`/`#{}` extraction.
// ---------------------------------------------------------------------

/// Extracts `${prop}` placeholder keys and `#{beanRef}` SpEL
/// bean-reference candidates from `segments` — [`build_value_lit`]/
/// [`build_value_lit_from_segments`]'s one extraction pass, run uniformly
/// over every literal value this crate produces (this module's own doc
/// comment).
///
/// M0b landed the minimal scope (build plan P9 row / spec SB-13): "at
/// minimum extract `#{beanRef}` bean-reference candidates and `${prop}`
/// keys; nested `${a.${b}}` and heavy harvesting can be M1." This is that
/// M1 harvesting pass — see [`extract_from_slice`] for the exact rule.
///
/// Each entry of `segments` is scanned **independently** — `depth`/
/// `quote_aware` both reset per segment, exactly as `events::build_tree`'s
/// own `check_unterminated_placeholders` scans each raw text/CDATA run on
/// its own rather than a merged string (`events.rs`, the `Event::Text`/
/// `Event::CData` arms). This is required for invariant #4 (every
/// `Spanned`'s span slices back to its own decoded text): a comment or a
/// CDATA delimiter sitting between two text runs is real source bytes
/// that never made it into any segment's `value`, so scanning a
/// concatenation of `segments` would compute offsets into a string the
/// source itself doesn't contiguously contain at those positions. It also
/// keeps this pass in agreement with that same diagnostic layer: an
/// opener in one segment with no closing brace in *that* segment is
/// exactly what `check_unterminated_placeholders` already flags
/// `UnterminatedPlaceholder` for (e.g. `${a<!-- -->}`, where the comment
/// splits `${a` and `}` into two separate text runs) — scanning per
/// segment here means this pass likewise finds no closing brace and
/// harvests nothing for it, rather than silently stitching the two runs
/// back together and harvesting a placeholder U1 already called
/// unterminated.
fn extract_placeholders_and_spel_refs(
    segments: &[Spanned<String>],
) -> (Vec<Spanned<String>>, Vec<Spanned<String>>) {
    let mut placeholders = Vec::new();
    let mut spel_refs = Vec::new();
    for text in coalesce_contiguous_segments(segments) {
        extract_from_slice(
            &text.value,
            text.span.start,
            0,
            false,
            &mut placeholders,
            &mut spel_refs,
        );
    }
    (placeholders, spel_refs)
}

/// Merges consecutive entries of `segments` that are **byte-contiguous**
/// (`prev.span.end == next.span.start`) into one — the entity-split
/// harvesting regression fix (M1, coalesced with `events::build_tree`'s own
/// identical fix on its `check_unterminated_placeholders` call sites): an
/// entity reference (`&amp;`) arrives as its own raw text run between two
/// ordinary text runs, splitting one logical `${}`/`#{}` expression
/// (`#{flagA &amp;&amp; flagB}`) across more than one `segments` entry even
/// though every byte between them is still present, unresolved, in
/// `source`. Scanning each of those runs independently (as this function
/// did before this fix) meant the opener never met its own closer, so
/// nothing was ever harvested — a regression from M1c's own heavier
/// harvesting, since M0b's original single-segment call site never hit it.
///
/// This is exactly the same contiguity test this module's own doc comment
/// already explains for *why* segments must be scanned separately in the
/// first place (a comment or a CDATA delimiter sitting between two runs is
/// real source bytes no run's `value` contains) — merging only ever
/// happens when *no* such gap exists, so the merged entry's `value` is
/// still an exact `source` slice at its own (now wider) `span`, upholding
/// invariant #4 for every entry this function goes on to scan. Concatenating
/// each contiguous group's `value` strings (rather than re-slicing
/// `source`, which this module has no handle on) is safe for exactly the
/// same reason: each entry's own `value` already equals `source` at its own
/// span (per-entry invariant #4, `dispatch::element_text_segments`'s own
/// guarantee), so two byte-contiguous entries' concatenated values equal
/// `source` at the union span.
fn coalesce_contiguous_segments(segments: &[Spanned<String>]) -> Vec<Spanned<String>> {
    let mut merged: Vec<Spanned<String>> = Vec::with_capacity(segments.len());
    for text in segments {
        match merged.last_mut() {
            Some(prev) if prev.span.end == text.span.start => {
                prev.value.push_str(&text.value);
                prev.span.end = text.span.end;
            }
            _ => merged.push(text.clone()),
        }
    }
    merged
}

/// One `${}`/`#{}` scan-and-harvest pass over `slice`, recursing into each
/// match's own inner text so nested expressions are harvested too, not
/// just the outermost one. `base` is `slice`'s own absolute byte offset
/// into the original literal (for span math); `depth` counts how many
/// levels of nesting this call is already inside (0 = the literal's own
/// top-level text, as opposed to already being inside some outer `${}`/
/// `#{}`'s inner text) — capped at [`crate::DEPTH_LIMIT`], same rationale
/// and same constant [`crate::events::scan_braced_expr`]'s own nesting cap
/// reuses (a heap-bounded, not call-stack-bounded, recursion would still
/// be needed for truly pathological input, but `scan_braced_expr` already
/// gives up past that many nested openers, so anything this function could
/// still recurse into beyond the cap is vanishingly rare hostile input,
/// not a real fixture shape — capping here is simpler than also making
/// this call heap-iterative). `quote_aware` mirrors `scan_braced_expr`'s
/// own per-opener rule: `true` only when `slice` is itself the inner text
/// of a `#{...}` (SpEL) opener, `false` for the literal's own top-level
/// text and for a `${...}` opener's inner text — SpEL string-literal
/// quoting (`'`/`"`) is a `#{}` notion only.
///
/// **The exact harvesting rule** (M1, SB-13 "heavier expression
/// harvesting" build-plan row):
/// - Every top-level opener in `slice` still contributes its own entry
///   exactly as M0b did: a `${...}` contributes one `placeholders` entry
///   holding the *whole* inner text verbatim (so `${a.${b}}` still yields
///   an `"a.${b}"` entry — the outer key's raw form is never dropped or
///   rewritten); a `#{...}` contributes at most one `spel_refs` entry, the
///   leading identifier-shaped token at the front of the trimmed SpEL body
///   ([`leading_bean_ref_candidate`]) — unchanged M0b heuristic, kept
///   deliberately minimal (no expression parsing).
/// - **New in M1**: after recording that outer entry, this function
///   recurses into the *same* inner text (one level deeper) to harvest any
///   further `${}`/`#{}` sub-expressions nested inside it, *in addition
///   to* the outer entry — never instead of it. So `${a.${b}}` now yields
///   `["a.${b}", "b"]` (outer first, inner second, in scan order), and
///   `#{beanA.m(#{other})}` yields `spel_refs: ["beanA", "other"]`. This is
///   what makes "multiple `#{}` in one literal" collection complete even
///   when one is nested inside another's argument list, not just when
///   they're siblings (`#{a} #{b}` already collected both under M0b, since
///   those are two separate top-level openers the scan just walks past
///   each other — this recursion is only needed for the nested case).
/// - The recursive inner scan is **quote-aware exactly like
///   `scan_braced_expr`**: when recursing into a `#{...}` opener's own
///   inner text, `quote_aware` flips to `true`, so a SpEL string literal
///   inside it (`#{f('${x}')}`) does *not* spuriously harvest the `${x}`
///   sitting inside the quotes as if it were a real nested placeholder —
///   quoted text is exactly as inert to this harvesting pass as it is to
///   `scan_braced_expr`'s own brace matching. A `${...}` opener's inner
///   text keeps `quote_aware = false` when recursed into, same as the
///   top-level scan, since `${}` defaults never treat `'`/`"` specially
///   (`${admin.name:O'Reilly}`).
/// - An opener that never closes (`scan_braced_expr` returns `None`) stops
///   this slice's scan entirely: raw text is already kept verbatim and U1
///   already diagnosed `UnterminatedPlaceholder` on it; there's no
///   well-formed key to extract for it. In the ordinary case `None` means
///   the scan ran off the end of `slice` looking for a close, so nothing
///   afterward could close it either (same "nothing left in this run can
///   close it" reasoning `check_unterminated_placeholders` documents) —
///   but `scan_braced_expr` also returns `None` short of end-of-slice once
///   its own nested-opener count hits `DEPTH_LIMIT` (its own doc comment),
///   a deliberately bounded-effort give-up on pathologically deep nesting,
///   not proof no later well-formed opener exists in the rest of `slice`.
///   `check_unterminated_placeholders` makes the exact same simplifying
///   choice for that sub-case (stopping its own scan on the first `None`
///   too), so this stays layer-consistent rather than a P9-only gap; a
///   real fixture nesting `DEPTH_LIMIT`-deep is itself already hostile-
///   input territory (SB-16), not a shape either layer optimizes for.
/// - A literal bare `$` or `#` not immediately followed by `{` (no `${`/
///   `#{` sequence at all) never enters this scan in the first place —
///   same opener test the diagnostic scan uses.
#[allow(clippy::too_many_arguments)]
fn extract_from_slice(
    slice: &str,
    base: u32,
    depth: u32,
    quote_aware: bool,
    placeholders: &mut Vec<Spanned<String>>,
    spel_refs: &mut Vec<Spanned<String>>,
) {
    let bytes = slice.as_bytes();
    let mut i = 0usize;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if quote_aware && (b == b'\'' || b == b'"') {
            quote = Some(b);
            i += 1;
            continue;
        }
        let is_dollar = b == b'$';
        let is_hash = b == b'#';
        if !(is_dollar || is_hash) || i + 1 >= bytes.len() || bytes[i + 1] != b'{' {
            i += 1;
            continue;
        }
        let Some(next) = scan_braced_expr(bytes, i) else {
            break; // unterminated — already diagnosed by U1, nothing to extract
        };
        let inner_start = i + 2;
        let inner_end = next - 1; // exclude the matching closing '}'
        let inner = &slice[inner_start..inner_end];
        if is_dollar {
            if !inner.is_empty() {
                placeholders.push(Spanned {
                    value: inner.to_string(),
                    span: ByteSpan {
                        start: base + inner_start as u32,
                        end: base + inner_end as u32,
                    },
                });
            }
        } else if let Some((rel_start, candidate)) = leading_bean_ref_candidate(inner) {
            let candidate_start = inner_start + rel_start;
            let candidate_end = candidate_start + candidate.len();
            spel_refs.push(Spanned {
                value: candidate.to_string(),
                span: ByteSpan {
                    start: base + candidate_start as u32,
                    end: base + candidate_end as u32,
                },
            });
        }
        if depth < crate::DEPTH_LIMIT {
            extract_from_slice(
                inner,
                base + inner_start as u32,
                depth + 1,
                is_hash, // only a `#{}` opener's own inner text is quote-aware
                placeholders,
                spel_refs,
            );
        }
        i = next;
    }
}

/// The bean-reference candidate at the front of a `#{...}` SpEL body's
/// inner text — a leading identifier-shaped token (Unicode-aware: a
/// leading alphabetic/`_`/`$` char, then any run of alphanumeric/`_`/`$`
/// chars — this crate's proptest generators exercise Korean identifiers
/// elsewhere in the model, so bean names aren't assumed ASCII-only here
/// either) immediately at the start of the whitespace-trimmed expression.
/// `None` when the trimmed body is empty, or doesn't start with an
/// identifier char at all (a string literal, a number, an operator) — see
/// this function's one caller's doc comment for why that's "no candidate"
/// rather than a guess.
fn leading_bean_ref_candidate(inner: &str) -> Option<(usize, &str)> {
    let start = inner.find(|c: char| !c.is_whitespace())?;
    let rest = &inner[start..];
    let mut chars = rest.chars();
    let first = chars.next()?;
    if !(first.is_alphabetic() || first == '_' || first == '$') {
        return None;
    }
    let mut end = first.len_utf8();
    for c in rest[end..].chars() {
        if c.is_alphanumeric() || c == '_' || c == '$' {
            end += c.len_utf8();
        } else {
            break;
        }
    }
    Some((start, &rest[..end]))
}

/// `value="literal"` shorthand attribute → a [`ValueLit`] (no
/// `value_type` — that's only ever expressed via `<value type="...">`'s own
/// attribute, which the shorthand attribute form has no room for).
pub(crate) fn value_lit_from_attr(attr: &XmlAttr) -> ValueLit {
    let text = spanned_attr(attr);
    let span = text.span;
    build_value_lit(text, span, None)
}

/// `<value type="java.lang.Integer">42</value>` → a [`ValueLit`]. `text`
/// reads every direct text child (`dispatch::merge_text_segments` over
/// `dispatch::element_text_segments`, shared with `<description>`'s own
/// reading); `span` is the whole element's extent, not just the text span
/// — matches every other self-span node in this model. Goes through
/// [`build_value_lit_from_segments`] rather than [`build_value_lit`] so
/// P9 extraction sees each raw text/CDATA run's own span instead of the
/// merged `text`'s (that module's own doc comment on why: a `<value>`
/// with comment- or CDATA-adjacent text children is not byte-contiguous
/// with `text.value`).
fn value_lit_from_element(element: &XmlElement) -> ValueLit {
    let segments = element_text_segments(element);
    let text = merge_text_segments(&segments, element.span);
    let value_type = parse_type_attr(&element.attrs);
    build_value_lit_from_segments(&segments, text, element.span, value_type)
}

/// `type="..."` on a `<value>` child — the one `ClassRef`-bearing attribute
/// this unit itself produces (SB-12's cross-unit conformance check, I1,
/// verifies every `ClassRef` site including this one is populated by its
/// owning unit — this is U5a's). Empty-value handling mirrors
/// `bean::parse_class_ref`'s identical reasoning for `class=`: invariant #5
/// (`ClassRef.raw` never empty) is upheld by simply never constructing one
/// from an empty attribute value.
fn parse_type_attr(attrs: &[XmlAttr]) -> Option<Spanned<ClassRef>> {
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

// ---------------------------------------------------------------------
// ref= / <ref> / <idref>.
// ---------------------------------------------------------------------

/// `ref="beanName"` shorthand attribute → a `Spanned<BeanRef>` (always
/// `RefKind::Bean` — same container the attribute itself resolves within,
/// spec's "settled decisions"). `None` (plus `RefWithoutTarget`) for a
/// present-but-empty attribute, mirroring `bean::spanned_bean_ref`'s
/// identical empty-value policy for `parent=`/`factory-bean=` (invariant
/// #5: `BeanRef.raw` never empty).
pub(crate) fn ref_from_attr(
    attr: &XmlAttr,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Spanned<BeanRef>> {
    bean_ref_from_attr(attr, RefKind::Bean, diagnostics)
}

fn bean_ref_from_attr(
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

/// `<ref bean=.../local=.../parent=...>` → a `Spanned<BeanRef>`. Precedence
/// when more than one is present (not legal per the Spring XSD — this
/// crate has no schema view, spec's "settled decisions" — but must still resolve
/// *something* deterministically rather than picking arbitrarily): `bean`
/// first, then legacy `local`, then `parent` — the conventional attribute
/// documentation order. `None` (plus `RefWithoutTarget`) when none of the
/// three are present at all, or the one chosen is present-but-empty.
fn ref_from_element(
    element: &XmlElement,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Spanned<BeanRef>> {
    if let Some(attr) = find_attr(&element.attrs, "bean") {
        return bean_ref_from_attr(attr, RefKind::Bean, diagnostics);
    }
    if let Some(attr) = find_attr(&element.attrs, "local") {
        return bean_ref_from_attr(attr, RefKind::Local, diagnostics);
    }
    if let Some(attr) = find_attr(&element.attrs, "parent") {
        return bean_ref_from_attr(attr, RefKind::ParentContainer, diagnostics);
    }
    diagnostics.push(Diagnostic {
        code: DiagCode::RefWithoutTarget,
        span: Some(element.span),
        message: "<ref> has none of bean=/local=/parent=".to_string(),
    });
    None
}

/// `<idref bean=.../local=...>` → a `Spanned<BeanRef>`. No `parent=` here —
/// unlike `<ref>`, the Spring `beans.xsd` `IdRefType` only ever declares
/// `bean`/(legacy) `local`.
fn idref_from_element(
    element: &XmlElement,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Spanned<BeanRef>> {
    if let Some(attr) = find_attr(&element.attrs, "bean") {
        return bean_ref_from_attr(attr, RefKind::Bean, diagnostics);
    }
    if let Some(attr) = find_attr(&element.attrs, "local") {
        return bean_ref_from_attr(attr, RefKind::Local, diagnostics);
    }
    diagnostics.push(Diagnostic {
        code: DiagCode::RefWithoutTarget,
        span: Some(element.span),
        message: "<idref> has none of bean=/local=".to_string(),
    });
    None
}

// ---------------------------------------------------------------------
// The recursive value-child dispatcher — this unit's "recursion heart".
// ---------------------------------------------------------------------

/// Resolves one already-identified value-shaped child element — `<value>`,
/// `<ref>`, `<idref>`, `<null>`, an anonymous `<bean>`, or (U5b) a
/// collection element (`<list>`/`<set>`/`<array>`/`<map>`/`<props>`) — into
/// an `InjectValue`. `None` for a genuinely unrecognized shape — the caller
/// decides how to diagnose an absence; this function only ever resolves,
/// never opines on what "nothing recognized" means at the call site.
///
/// `depth` is the caller's running nesting depth (0 at the outermost
/// injection point); this function is the single place that checks it
/// against [`crate::DEPTH_LIMIT`] before recursing into `<bean>` — see this
/// module's own doc comment for why the check lives here rather than in
/// `bean::parse_bean` itself.
pub(crate) fn parse_inject_value_child(
    scope: &NsScope,
    diagnostics: &mut Vec<Diagnostic>,
    depth: u32,
    element: &XmlElement,
) -> Option<InjectValue> {
    let child_scope = NsScope::from_element(element, Some(scope));
    let (ns, local) = resolve_qname(&element.name, &child_scope);
    if !is_beans_ns(&ns) {
        return None;
    }
    match local.as_str() {
        "value" => Some(InjectValue::Value(value_lit_from_element(element))),
        "ref" => ref_from_element(element, diagnostics).map(InjectValue::Ref),
        "idref" => idref_from_element(element, diagnostics).map(InjectValue::Idref),
        "null" => Some(InjectValue::Null(element.span)),
        "bean" => Some(parse_inner_bean(scope, diagnostics, depth, element)),
        // U5b (SB-07): collections. Wired directly into `crate::collection`
        // — this is that module's own seam (U5a→U5b is a *serial*
        // extension of this exact match, not a parallel leaf pair), not
        // one of the frozen root-/bean-child dispatch matches the
        // leaf-conflict-avoidance contract protects. `scope` (not
        // `child_scope`) is passed through, same "callee re-derives its
        // own overlay" convention `parse_inner_bean` below follows.
        "list" | "set" | "array" | "map" | "props" => Some(
            crate::collection::parse_collection_value(scope, diagnostics, depth, element),
        ),
        _ => None,
    }
}

/// `<bean>` (anonymous inner bean) → `InjectValue::Inner`, guarded by
/// [`crate::DEPTH_LIMIT`]. At the limit: `NestingLimitExceeded` plus an
/// opaque `InjectValue::Null` downgrade (spec's own `NestingLimitExceeded`
/// doc comment: "the remaining subtree is treated as opaque rather than
/// risking a stack overflow") **instead of** calling `parse_bean` at all —
/// the guard's whole point is to bound the recursion *before* it happens,
/// not to let one more level through and stop after.
fn parse_inner_bean(
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
                "inner <bean> nesting exceeded {} levels; subtree treated as opaque",
                crate::DEPTH_LIMIT
            ),
        });
        return InjectValue::Null(element.span);
    }
    // `depth + 1`: this inner bean is one nesting level deeper than the
    // caller that reached it — `parse_bean` threads this value, unchanged,
    // into its own `<property>`/`<constructor-arg>` children's calls back
    // into `parse_inject_value_child`, which is what keeps this mutual
    // recursion actually bounded (see this module's own doc comment).
    let bean = crate::bean::parse_bean(scope, element, diagnostics, depth + 1);
    InjectValue::Inner(Box::new(bean))
}

// ---------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------
//
// These stay in-module (rather than an external `tests/u5a_inject_value.rs`)
// because every type/function this unit exports is `pub(crate)` — a seam
// not visible from an external integration-test binary, same rationale
// `src/events.rs`'s/`src/encoding.rs`'s own `#[cfg(test)] mod tests` doc
// comments document. `tests/u5a_inject_value.rs` carries a pointer here.

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

    // -------------------------------------------------------------
    // Snapshot per variant.
    // -------------------------------------------------------------

    #[test]
    fn sb06_value_child_snapshot() {
        let element = parse_fragment("<value type=\"java.lang.Integer\">42</value>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        insta::assert_json_snapshot!(result);
    }

    #[test]
    fn sb06_value_child_with_empty_type_attr_has_no_value_type() {
        // Present-but-empty `type=""` must fall back to `None`, upholding
        // invariant #5 (`ClassRef.raw` never empty) the same way
        // `bean::parse_class_ref` does for `class=` — this is the branch
        // that actually enforces it for this unit's own `ClassRef` site.
        let element = parse_fragment("<value type=\"\">42</value>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        match result {
            Some(InjectValue::Value(vl)) => {
                assert_eq!(vl.value_type, None);
                assert_eq!(vl.text.value, "42");
            }
            other => panic!("expected Value, got {other:?}"),
        }
    }

    #[test]
    fn sb06_value_child_without_type_snapshot() {
        let element = parse_fragment("<value>hello world</value>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        insta::assert_json_snapshot!(result);
    }

    #[test]
    fn sb06_ref_child_bean_attr_snapshot() {
        let element = parse_fragment("<ref bean=\"target\"/>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        insta::assert_json_snapshot!(result);
    }

    #[test]
    fn sb06_ref_child_local_attr_snapshot() {
        let element = parse_fragment("<ref local=\"target\"/>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        assert!(diagnostics.is_empty());
        insta::assert_json_snapshot!(result);
        match result {
            Some(InjectValue::Ref(r)) => assert_eq!(r.value.kind, RefKind::Local),
            other => panic!("expected Ref(Local), got {other:?}"),
        }
    }

    #[test]
    fn sb06_ref_child_parent_attr_snapshot() {
        let element = parse_fragment("<ref parent=\"target\"/>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        assert!(diagnostics.is_empty());
        insta::assert_json_snapshot!(result);
        match result {
            Some(InjectValue::Ref(r)) => assert_eq!(r.value.kind, RefKind::ParentContainer),
            other => panic!("expected Ref(ParentContainer), got {other:?}"),
        }
    }

    #[test]
    fn sb06_idref_child_bean_attr_snapshot() {
        let element = parse_fragment("<idref bean=\"target\"/>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        assert!(diagnostics.is_empty());
        insta::assert_json_snapshot!(result);
    }

    #[test]
    fn sb06_idref_child_local_attr_snapshot() {
        let element = parse_fragment("<idref local=\"target\"/>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        assert!(diagnostics.is_empty());
        insta::assert_json_snapshot!(result);
        match result {
            Some(InjectValue::Idref(r)) => assert_eq!(r.value.kind, RefKind::Local),
            other => panic!("expected Idref(Local), got {other:?}"),
        }
    }

    #[test]
    fn sb06_null_child_snapshot() {
        let element = parse_fragment("<null/>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        assert!(diagnostics.is_empty());
        insta::assert_json_snapshot!(result);
    }

    #[test]
    fn sb06_value_attr_shorthand_builds_value_lit() {
        let element = parse_fragment("<property name=\"x\" value=\"literalText\"/>");
        let attr = find_attr(&element.attrs, "value").expect("value attr present");
        let value_lit = value_lit_from_attr(attr);
        assert_eq!(value_lit.text.value, "literalText");
        assert_eq!(value_lit.value_type, None);
        assert!(value_lit.placeholders.is_empty());
        assert!(value_lit.spel_refs.is_empty());
        assert_eq!(value_lit.span, value_lit.text.span);
    }

    #[test]
    fn sb06_ref_attr_shorthand_builds_bean_ref() {
        let element = parse_fragment("<property name=\"x\" ref=\"target\"/>");
        let attr = find_attr(&element.attrs, "ref").expect("ref attr present");
        let mut diagnostics = no_diag();
        let bean_ref = ref_from_attr(attr, &mut diagnostics).expect("ref resolves");
        assert!(diagnostics.is_empty());
        assert_eq!(bean_ref.value.raw, "target");
        assert_eq!(bean_ref.value.kind, RefKind::Bean);
    }

    #[test]
    fn sb06_ref_attr_shorthand_empty_is_ref_without_target() {
        let element = parse_fragment("<property name=\"x\" ref=\"\"/>");
        let attr = find_attr(&element.attrs, "ref").expect("ref attr present");
        let mut diagnostics = no_diag();
        let bean_ref = ref_from_attr(attr, &mut diagnostics);
        assert_eq!(bean_ref, None);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == DiagCode::RefWithoutTarget));
    }

    // -------------------------------------------------------------
    // Inner-bean structural snapshot (build plan: "structure only" — content
    // completion is I6's job once U6/U7 wire property/ctor-arg). U6 has
    // now landed (`crate::property::parse_property`, wired behind
    // `bean::dispatch_bean_child`'s `"property"` arm), so the inner bean's
    // own `<property>` content is populated too — updated from this test's
    // original assertion (`bean.properties.is_empty()`), which pinned the
    // pre-U6 boundary this exact comment predicted would move.
    // -------------------------------------------------------------

    #[test]
    fn sb06_inner_bean_structural_snapshot() {
        let element = parse_fragment(concat!(
            "<bean id=\"innerBean\" class=\"com.example.Widget\">",
            "<property name=\"ignoredForNow\" value=\"1\"/>",
            "</bean>"
        ));
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:?}"
        );
        match &result {
            Some(InjectValue::Inner(bean)) => {
                assert_eq!(
                    bean.id.as_ref().map(|s| s.value.as_str()),
                    Some("innerBean")
                );
                assert_eq!(
                    bean.class.as_ref().map(|c| c.value.raw.as_str()),
                    Some("com.example.Widget")
                );
                assert_eq!(bean.properties.len(), 1);
                assert_eq!(bean.properties[0].name.value, "ignoredForNow");
            }
            other => panic!("expected Inner, got {other:?}"),
        }
        insta::assert_json_snapshot!(result);
    }

    // -------------------------------------------------------------
    // RefWithoutTarget: missing/empty target attributes.
    // -------------------------------------------------------------

    #[test]
    fn sb06_ref_child_with_no_target_attrs_is_ref_without_target() {
        let element = parse_fragment("<ref/>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        assert_eq!(result, None);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == DiagCode::RefWithoutTarget));
    }

    #[test]
    fn sb06_ref_child_with_empty_bean_attr_is_ref_without_target() {
        let element = parse_fragment("<ref bean=\"\"/>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        assert_eq!(result, None);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == DiagCode::RefWithoutTarget));
    }

    #[test]
    fn sb06_idref_child_with_no_target_attrs_is_ref_without_target() {
        let element = parse_fragment("<idref/>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        assert_eq!(result, None);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == DiagCode::RefWithoutTarget));
    }

    #[test]
    fn sb06_idref_child_ignores_parent_attr_unlike_ref() {
        // <idref> has no `parent=` in the Spring XSD (unlike <ref>) — an
        // element carrying only `parent=` must fall through to
        // RefWithoutTarget, not silently resolve as ParentContainer.
        let element = parse_fragment("<idref parent=\"target\"/>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        assert_eq!(result, None);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == DiagCode::RefWithoutTarget));
    }

    #[test]
    fn sb06_ref_child_bean_attr_takes_precedence_over_local_and_parent() {
        let element = parse_fragment("<ref bean=\"b\" local=\"l\" parent=\"p\"/>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        match result {
            Some(InjectValue::Ref(r)) => {
                assert_eq!(r.value.raw, "b");
                assert_eq!(r.value.kind, RefKind::Bean);
            }
            other => panic!("expected Ref(Bean) via precedence, got {other:?}"),
        }
    }

    #[test]
    fn sb06_idref_child_bean_attr_takes_precedence_over_local() {
        // `<idref>`'s own precedence table (`idref_from_element`'s doc
        // comment: `bean` first, then legacy `local`) has no dedicated test
        // pinning both attributes present at once — only `<ref>`'s
        // three-way precedence above is directly exercised.
        let element = parse_fragment("<idref bean=\"b\" local=\"l\"/>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        match result {
            Some(InjectValue::Idref(r)) => {
                assert_eq!(r.value.raw, "b");
                assert_eq!(r.value.kind, RefKind::Bean);
            }
            other => panic!("expected Idref(Bean) via precedence, got {other:?}"),
        }
    }

    // -------------------------------------------------------------
    // Unrecognized / foreign-namespace / reserved-collection shapes.
    // -------------------------------------------------------------

    #[test]
    fn sb06_unrecognized_beans_ns_child_returns_none_without_diagnostic() {
        let element = parse_fragment("<totally-made-up/>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        assert_eq!(result, None);
        assert!(
            diagnostics.is_empty(),
            "this unit doesn't opine on what an unrecognized shape means: {diagnostics:?}"
        );
    }

    #[test]
    fn sb06_foreign_namespace_child_returns_none() {
        let element = parse_fragment(concat!(
            "<aop:scoped-proxy ",
            "xmlns:aop=\"http://www.springframework.org/schema/aop\"/>"
        ));
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
        assert_eq!(result, None);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn sb06_collection_element_names_resolve_to_collection_via_u5b() {
        // U5b (SB-07) has landed: these five element names now resolve to
        // `InjectValue::Collection` through `crate::collection`, rather
        // than the `None` reservation this test originally pinned before
        // U5b existed. Full collection-shape coverage (items/entries/
        // merge/value-type/key-type/span) lives in
        // `collection::tests`/`tests/u5b_collection.rs` — this only pins
        // that this match arm actually routes there.
        for name in ["list", "set", "array", "map", "props"] {
            let element = parse_fragment(&format!("<{name}/>"));
            let mut diagnostics = no_diag();
            let result =
                parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
            assert!(
                matches!(result, Some(InjectValue::Collection(_))),
                "{name} must resolve to InjectValue::Collection, got {result:?}"
            );
            assert!(diagnostics.is_empty(), "{name} must not be diagnosed here");
        }
    }

    // -------------------------------------------------------------
    // proptest: depth 0..4 panic-free (build plan's own generator range).
    // -------------------------------------------------------------

    proptest::proptest! {
        #[test]
        fn sb06_proptest_inner_bean_depth_0_to_4_panic_free(
            depth in 0u32..4,
            id in "[a-zA-Z][a-zA-Z0-9_]{0,15}",
            class in "com\\.example\\.[A-Z][a-zA-Z0-9]{0,15}",
        ) {
            let source = format!("<bean id=\"{id}\" class=\"{class}\"/>");
            let element = parse_fragment(&source);
            let mut diagnostics = Vec::new();
            let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, depth, &element);
            proptest::prop_assert!(matches!(result, Some(InjectValue::Inner(_))));
            proptest::prop_assert!(!diagnostics.iter().any(|d| d.code == DiagCode::NestingLimitExceeded));
        }

        #[test]
        fn sb06_proptest_value_text_round_trips_without_entities(
            text in "[a-zA-Z0-9 ,.]{0,50}",
        ) {
            let source = format!("<value>{text}</value>");
            let element = parse_fragment(&source);
            let mut diagnostics = Vec::new();
            let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, 0, &element);
            match result {
                Some(InjectValue::Value(vl)) => proptest::prop_assert_eq!(vl.text.value, text),
                other => proptest::prop_assert!(false, "expected Value, got {:?}", other),
            }
        }

        #[test]
        fn sb06_proptest_arbitrary_depth_never_panics_and_downgrades_exactly_at_the_limit(
            depth in 0u32..2000,
        ) {
            let element = parse_fragment("<bean class=\"com.example.Widget\"/>");
            let mut diagnostics = Vec::new();
            let result = parse_inject_value_child(&NsScope::default(), &mut diagnostics, depth, &element);
            let downgraded = diagnostics.iter().any(|d| d.code == DiagCode::NestingLimitExceeded);
            if depth >= crate::DEPTH_LIMIT {
                proptest::prop_assert!(downgraded, "expected NestingLimitExceeded at depth {}", depth);
                proptest::prop_assert!(matches!(result, Some(InjectValue::Null(_))));
            } else {
                proptest::prop_assert!(!downgraded, "unexpected NestingLimitExceeded at depth {}", depth);
                proptest::prop_assert!(matches!(result, Some(InjectValue::Inner(_))));
            }
        }
    }

    // -------------------------------------------------------------
    // DEPTH_LIMIT downgrade — pinned boundary cases (not just the proptest
    // above), per this unit's explicit test-design requirement.
    // -------------------------------------------------------------

    #[test]
    fn sb06_depth_limit_downgrades_inner_bean_to_null_plus_diagnostic() {
        let element = parse_fragment("<bean id=\"deep\" class=\"com.example.Widget\"/>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(
            &NsScope::default(),
            &mut diagnostics,
            crate::DEPTH_LIMIT,
            &element,
        );
        assert_eq!(result, Some(InjectValue::Null(element.span)));
        assert!(diagnostics
            .iter()
            .any(|d| d.code == DiagCode::NestingLimitExceeded));
    }

    #[test]
    fn sb06_depth_one_below_limit_still_recurses_into_a_real_inner_bean() {
        let element = parse_fragment("<bean id=\"stillOk\" class=\"com.example.Widget\"/>");
        let mut diagnostics = no_diag();
        let result = parse_inject_value_child(
            &NsScope::default(),
            &mut diagnostics,
            crate::DEPTH_LIMIT - 1,
            &element,
        );
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code == DiagCode::NestingLimitExceeded),
            "one level below the limit must not downgrade: {diagnostics:?}"
        );
        match result {
            Some(InjectValue::Inner(bean)) => {
                assert_eq!(bean.id.as_ref().map(|s| s.value.as_str()), Some("stillOk"))
            }
            other => panic!("expected Inner, got {other:?}"),
        }
    }

    #[test]
    fn sb06_depth_limit_downgrade_does_not_call_parse_bean_at_all() {
        // The downgrade must happen *before* recursing, not one level past
        // it — a malformed inner bean (missing class/parent, which would
        // normally raise BeanWithoutClassOrParent) must not leak that
        // diagnostic through when it's never actually parsed.
        let element = parse_fragment("<bean id=\"neverParsed\"/>");
        let mut diagnostics = no_diag();
        let _ = parse_inject_value_child(
            &NsScope::default(),
            &mut diagnostics,
            crate::DEPTH_LIMIT,
            &element,
        );
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code == DiagCode::BeanWithoutClassOrParent),
            "parse_bean must never run once the depth limit is hit: {diagnostics:?}"
        );
        assert_eq!(
            diagnostics.len(),
            1,
            "exactly one diagnostic (NestingLimitExceeded), nothing from a real bean parse: {diagnostics:?}"
        );
    }

    #[test]
    fn sb06_build_value_lit_no_placeholder_or_spel_syntax_leaves_both_fields_empty() {
        let vl = build_value_lit(
            Spanned {
                value: "x".to_string(),
                span: ByteSpan { start: 0, end: 1 },
            },
            ByteSpan { start: 0, end: 1 },
            None,
        );
        assert!(vl.placeholders.is_empty());
        assert!(vl.spel_refs.is_empty());
    }

    // -------------------------------------------------------------
    // P9 (SB-13): `${}`/`#{}` extraction table — build plan's own case
    // list: "${prop} · #{beanA.m()} · partial expression · CDATA · literal $/# ·
    // <list> item-internal ${} · p-namespace attribute-internal ${}, centralized
    // lock · ${unterminated → UnterminatedPlaceholder+raw preserved". Every case here goes through
    // `build_value_lit` directly (the single builder P9 augments) — the
    // "flows through the central builder no matter the call site" claim
    // itself is exercised end-to-end in `tests/p9_spel_placeholder.rs`
    // instead (via `<list>`/p-namespace/`<prop>` fixtures through the
    // public `parse` API), not duplicated here.
    // -------------------------------------------------------------

    fn value_lit_for(text: &str) -> ValueLit {
        let element = parse_fragment(&format!("<value>{text}</value>"));
        value_lit_from_element(&element)
    }

    #[test]
    fn sb13_dollar_placeholder_key_extracted() {
        let vl = value_lit_for("${prop}");
        assert_eq!(
            vl.placeholders
                .iter()
                .map(|p| p.value.as_str())
                .collect::<Vec<_>>(),
            vec!["prop"]
        );
        assert!(vl.spel_refs.is_empty());
    }

    #[test]
    fn sb13_dollar_placeholder_span_covers_only_the_inner_key() {
        let source = "<value>${prop}</value>";
        let element = parse_fragment(source);
        let vl = value_lit_from_element(&element);
        let p = &vl.placeholders[0];
        assert_eq!(&source[p.span.start as usize..p.span.end as usize], "prop");
    }

    #[test]
    fn sb13_spel_bean_ref_candidate_extracted_from_method_call() {
        let vl = value_lit_for("#{beanA.m()}");
        assert!(vl.placeholders.is_empty());
        assert_eq!(
            vl.spel_refs
                .iter()
                .map(|s| s.value.as_str())
                .collect::<Vec<_>>(),
            vec!["beanA"]
        );
    }

    #[test]
    fn sb13_spel_bean_ref_candidate_span_covers_only_the_identifier() {
        let source = "<value>#{beanA.m()}</value>";
        let element = parse_fragment(source);
        let vl = value_lit_from_element(&element);
        let s = &vl.spel_refs[0];
        assert_eq!(&source[s.span.start as usize..s.span.end as usize], "beanA");
    }

    #[test]
    fn sb13_spel_bare_bean_ref_extracted() {
        let vl = value_lit_for("#{beanA}");
        assert_eq!(
            vl.spel_refs
                .iter()
                .map(|s| s.value.as_str())
                .collect::<Vec<_>>(),
            vec!["beanA"]
        );
    }

    #[test]
    fn sb13_spel_partial_expression_string_literal_yields_no_bean_ref_candidate() {
        // "partial expression" edge case: a SpEL body that isn't bean-reference-shaped
        // (starts with a string literal, not an identifier) must not guess
        // a wrong candidate.
        let vl = value_lit_for("#{'literal'}");
        assert!(vl.spel_refs.is_empty());
    }

    #[test]
    fn sb13_spel_partial_expression_arithmetic_yields_no_bean_ref_candidate() {
        let vl = value_lit_for("#{1 + 2}");
        assert!(vl.spel_refs.is_empty());
    }

    #[test]
    fn sb13_quote_aware_brace_literal_extracts_cleanly_without_crashing() {
        // The same quote-aware `#{'{'}`/`#{map['{']}` cases the U1 fix
        // covers must also extract cleanly here (not just avoid the
        // UnterminatedPlaceholder diagnostic).
        //
        // `#{'{'}`'s body starts with a string literal, not an identifier —
        // no bean-ref candidate. `#{map['{']}`'s body *does* start with the
        // identifier-shaped token `map` — this M0b heuristic's leading-
        // identifier rule extracts it as a candidate (an imperfect but
        // deliberately minimal "candidate", per this module's own doc
        // comment — resolving whether `map` is really a bean name or a
        // local variable is out of this unit's scope).
        let vl = value_lit_for("#{'{'}");
        assert!(vl.spel_refs.is_empty());

        let vl = value_lit_for("#{map['{']}");
        assert_eq!(
            vl.spel_refs
                .iter()
                .map(|s| s.value.as_str())
                .collect::<Vec<_>>(),
            vec!["map"]
        );
    }

    #[test]
    fn sb13_dollar_placeholder_with_apostrophe_default_extracted_whole() {
        // Quote-as-string-delimiter is a SpEL (`#{}`) notion only — inside a
        // `${prop:default}` property placeholder, `'`/`"` are ordinary
        // literal characters (Spring default values routinely contain an
        // apostrophe). Regression: treating the apostrophe as a quote
        // opener swallowed the real closing `}`, dropping the whole
        // placeholder key.
        let vl = value_lit_for("${admin.name:O'Reilly}");
        assert_eq!(
            vl.placeholders
                .iter()
                .map(|p| p.value.as_str())
                .collect::<Vec<_>>(),
            vec!["admin.name:O'Reilly"]
        );
        assert!(vl.spel_refs.is_empty());
    }

    #[test]
    fn sb13_dollar_placeholder_with_double_quote_default_extracted_whole() {
        let vl = value_lit_for("${msg:say \"hi\"}");
        assert_eq!(
            vl.placeholders
                .iter()
                .map(|p| p.value.as_str())
                .collect::<Vec<_>>(),
            vec!["msg:say \"hi\""]
        );
    }

    #[test]
    fn sb13_multiple_placeholders_and_spel_refs_in_one_value_all_extracted() {
        let vl = value_lit_for("${host}:${port} routed via #{routerBean.pick()}");
        assert_eq!(
            vl.placeholders
                .iter()
                .map(|p| p.value.as_str())
                .collect::<Vec<_>>(),
            vec!["host", "port"]
        );
        assert_eq!(
            vl.spel_refs
                .iter()
                .map(|s| s.value.as_str())
                .collect::<Vec<_>>(),
            vec!["routerBean"]
        );
    }

    #[test]
    fn sb13_nested_dollar_placeholder_collects_outer_and_inner_key() {
        // M1 ("heavy harvesting"): the outer key's raw text is never lost
        // (`"a.${b}"`, same as M0b), but the inner `${b}` is now also
        // harvested as its own entry — additive, not a replacement.
        let vl = value_lit_for("${a.${b}}");
        assert_eq!(
            vl.placeholders
                .iter()
                .map(|p| p.value.as_str())
                .collect::<Vec<_>>(),
            vec!["a.${b}", "b"]
        );
    }

    #[test]
    fn sb13_nested_dollar_placeholder_inner_span_covers_only_inner_key() {
        let source = "<value>${a.${b}}</value>";
        let element = parse_fragment(source);
        let vl = value_lit_from_element(&element);
        let inner = &vl.placeholders[1];
        assert_eq!(inner.value, "b");
        assert_eq!(
            &source[inner.span.start as usize..inner.span.end as usize],
            "b"
        );
    }

    #[test]
    fn sb13_triple_nested_dollar_placeholder_collects_every_level() {
        let vl = value_lit_for("${a.${b.${c}}}");
        assert_eq!(
            vl.placeholders
                .iter()
                .map(|p| p.value.as_str())
                .collect::<Vec<_>>(),
            vec!["a.${b.${c}}", "b.${c}", "c"]
        );
    }

    #[test]
    fn sb13_spel_nested_call_argument_extracts_outer_and_inner_bean_ref() {
        // "richer forms" (build plan P9 row): a SpEL method call whose own
        // argument is itself a `#{...}` — both the outer leading identifier
        // and the nested one are bean-reference candidates.
        let vl = value_lit_for("#{beanA.m(#{other})}");
        assert_eq!(
            vl.spel_refs
                .iter()
                .map(|s| s.value.as_str())
                .collect::<Vec<_>>(),
            vec!["beanA", "other"]
        );
    }

    #[test]
    fn sb13_multiple_sibling_spel_refs_in_one_literal_all_extracted() {
        // Two separate top-level `#{}` expressions in one literal (not
        // nested inside each other) — both collected, build plan's own
        // `#{a} #{b}` case.
        let vl = value_lit_for("#{a} #{b}");
        assert_eq!(
            vl.spel_refs
                .iter()
                .map(|s| s.value.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    #[test]
    fn sb13_spel_string_literal_containing_dollar_brace_not_harvested_as_nested_placeholder() {
        // Carry-over correctness check for the new recursive harvesting:
        // a `${x}`-shaped sequence sitting *inside* a `#{}` SpEL string
        // literal is inert text to SpEL, not a real nested placeholder —
        // the quote-aware recursion must not invent a spurious "x" entry.
        let vl = value_lit_for("#{f('${x}')}");
        assert!(vl.placeholders.is_empty());
        assert_eq!(
            vl.spel_refs
                .iter()
                .map(|s| s.value.as_str())
                .collect::<Vec<_>>(),
            vec!["f"]
        );
    }

    #[test]
    fn sb13_dollar_default_wrapping_quote_protected_hash_brace_extracted_whole() {
        // M1 carry-over fix (see `events::scan_braced_expr`'s own doc
        // comment): a `${}` default wrapping a `#{'}'}` SpEL sub-expression
        // (a string literal whose value is a literal `}`) must close at the
        // true final `}`, not at the first brace-looking byte inside the
        // nested SpEL string literal.
        let vl = value_lit_for("${x:#{'}'}}");
        assert_eq!(
            vl.placeholders
                .iter()
                .map(|p| p.value.as_str())
                .collect::<Vec<_>>(),
            vec!["x:#{'}'}"]
        );
        // The nested `#{'}'}` body is a string literal, not
        // bean-reference-shaped — no spurious candidate from the recursive
        // harvest either.
        assert!(vl.spel_refs.is_empty());
    }

    #[test]
    fn sb13_cdata_internal_placeholder_and_spel_ref_extracted() {
        let element = parse_fragment("<value><![CDATA[${prop} and #{beanA}]]></value>");
        let vl = value_lit_from_element(&element);
        assert_eq!(
            vl.placeholders
                .iter()
                .map(|p| p.value.as_str())
                .collect::<Vec<_>>(),
            vec!["prop"]
        );
        assert_eq!(
            vl.spel_refs
                .iter()
                .map(|s| s.value.as_str())
                .collect::<Vec<_>>(),
            vec!["beanA"]
        );
    }

    #[test]
    fn sb13_cdata_internal_literal_dollar_and_hash_without_brace_not_extracted() {
        // Negative CDATA case: a bare `$`/`#` with no `{` right after it is
        // still just literal text inside a CDATA section, same as outside
        // one.
        let element =
            parse_fragment("<value><![CDATA[costs $100 and uses #comment, no braces]]></value>");
        let vl = value_lit_from_element(&element);
        assert!(vl.placeholders.is_empty());
        assert!(vl.spel_refs.is_empty());
    }

    #[test]
    fn sb13_literal_dollar_and_hash_without_brace_not_extracted() {
        let vl = value_lit_for("costs $100 and uses #comment style, no braces here");
        assert!(vl.placeholders.is_empty());
        assert!(vl.spel_refs.is_empty());
    }

    #[test]
    fn sb13_literal_dollar_and_hash_with_space_before_brace_not_extracted() {
        // `$ {notAPlaceholder}` / `# {notASpelExpr}` — the opener must be
        // the exact two-byte `${`/`#{` sequence, not "somewhere nearby".
        let vl = value_lit_for("$ {notAPlaceholder} # {notASpelExpr}");
        assert!(vl.placeholders.is_empty());
        assert!(vl.spel_refs.is_empty());
    }

    #[test]
    fn sb13_unterminated_dollar_placeholder_extracts_nothing() {
        // Raw text is kept verbatim (U1's own recovery rule 6), but with no
        // matching close there's no well-formed key to extract.
        let vl = value_lit_for("${unterminated");
        assert!(vl.placeholders.is_empty());
        assert!(vl.spel_refs.is_empty());
        assert_eq!(vl.text.value, "${unterminated");
    }

    #[test]
    fn sb13_unterminated_hash_placeholder_extracts_nothing() {
        let vl = value_lit_for("#{unterminated");
        assert!(vl.placeholders.is_empty());
        assert!(vl.spel_refs.is_empty());
        assert_eq!(vl.text.value, "#{unterminated");
    }

    #[test]
    fn sb13_value_attr_shorthand_also_extracts_via_the_central_builder() {
        // `value_lit_from_attr` (the `value=` shorthand path) is a
        // different call site than `<value>` element text, but both funnel
        // through `build_value_lit` — pin that this path gets extraction
        // too, not just the element-child path every other case above uses.
        let element = parse_fragment("<property name=\"x\" value=\"${prop}\"/>");
        let attr = find_attr(&element.attrs, "value").expect("value attr present");
        let vl = value_lit_from_attr(attr);
        assert_eq!(
            vl.placeholders
                .iter()
                .map(|p| p.value.as_str())
                .collect::<Vec<_>>(),
            vec!["prop"]
        );
    }

    // -------------------------------------------------------------
    // Entity-split harvesting regression (M1c fix): an entity reference
    // (`&amp;`) arrives as its own raw text run, splitting one logical
    // `${}`/`#{}` expression across more than one `element_text_segments`
    // entry even though every byte between the runs is still present in the
    // source. `coalesce_contiguous_segments` must fold byte-contiguous runs
    // back together before `extract_from_slice` ever scans them.
    // -------------------------------------------------------------

    fn segs(pieces: &[(&str, u32)]) -> Vec<Spanned<String>> {
        pieces
            .iter()
            .map(|(s, start)| Spanned {
                value: s.to_string(),
                span: ByteSpan {
                    start: *start,
                    end: *start + s.len() as u32,
                },
            })
            .collect()
    }

    #[test]
    fn sb13_entity_split_segments_coalesce_before_scanning() {
        // "#{flagA " (0..8) + "&amp;" (8..13) + "&amp;" (13..18) + " flagB}" (18..25)
        // — exactly the shape `element_text_segments` produces for
        // `#{flagA &amp;&amp; flagB}` element text: four byte-contiguous
        // runs, none of which contains a matched `#{...}` on its own.
        let segments = segs(&[
            ("#{flagA ", 0),
            ("&amp;", 8),
            ("&amp;", 13),
            (" flagB}", 18),
        ]);
        let (placeholders, spel_refs) = extract_placeholders_and_spel_refs(&segments);
        assert!(placeholders.is_empty());
        assert_eq!(
            spel_refs
                .iter()
                .map(|s| s.value.as_str())
                .collect::<Vec<_>>(),
            vec!["flagA"],
            "coalesced entity-split segments must still harvest one spel_ref"
        );
    }

    #[test]
    fn sb13_non_contiguous_segments_are_not_coalesced() {
        // A gap between spans (e.g. a comment sitting between two text
        // runs, `dispatch::element_text_segments`'s own doc comment) must
        // NOT be bridged — only genuinely byte-contiguous runs coalesce.
        // "${a" (0..3) then a gap (comment at 3..12) then "}" (12..13).
        let segments = segs(&[("${a", 0)]);
        let mut segments = segments;
        segments.push(Spanned {
            value: "}".to_string(),
            span: ByteSpan { start: 12, end: 13 },
        });
        let (placeholders, _spel_refs) = extract_placeholders_and_spel_refs(&segments);
        assert!(
            placeholders.is_empty(),
            "non-contiguous runs must not be stitched into a false placeholder: {placeholders:?}"
        );
    }

    #[test]
    fn sb13_korean_bean_ref_candidate_extracted() {
        // This crate's own proptest generators exercise Korean identifiers
        // elsewhere (spec: "Korean identifiers") — a SpEL bean name isn't assumed
        // ASCII-only here either.
        let vl = value_lit_for("#{한글빈.메서드()}");
        assert_eq!(
            vl.spel_refs
                .iter()
                .map(|s| s.value.as_str())
                .collect::<Vec<_>>(),
            vec!["한글빈"]
        );
    }
}
