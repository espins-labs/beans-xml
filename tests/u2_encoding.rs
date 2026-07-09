//! Unit **U2** — encoding detection tests.
//!
//! `decode` and its helpers (`src/encoding.rs`) are `pub(crate)` — a seam
//! not visible from this external integration-test binary, the same
//! situation `tests/u1_events.rs` documents for `build_tree`
//! (`src/events.rs`'s own `#[cfg(test)] mod tests` doc comment). The real
//! U2 test suite — the UTF-8/EUC-KR/CP949/UTF-16/UTF-8-BOM table crossed
//! with {declared-matches, declared-mismatch (reality wins),
//! no-declaration}, plus the undecodable/lossy-fallback case — lives
//! there: `src/encoding.rs`'s own `#[cfg(test)]` module.
//!
//! This file exists so the unit still has a `tests/<unit>.rs` file per
//! this crate's `AGENTS.md` naming convention, and to pin from *outside*
//! the crate the one piece of this unit's contract that's already wired
//! into the public API today: `parse_bytes`'s oversize rejection
//! (`MAX_INPUT_BYTES`) happens **before** any decode is attempted, so
//! `ParseResult::encoding` is `None` for oversize input specifically
//! (never a decoded label, lossy or otherwise) — this holds regardless of
//! whether `parse_bytes` has been wired to `encoding::decode` yet (that
//! wiring, like `events::build_tree`'s, is U3's job).

#[test]
fn sb17_oversize_input_rejected_before_decode_with_encoding_none() {
    let big = vec![b' '; beans_xml::MAX_INPUT_BYTES + 1];
    let result = beans_xml::parse_bytes(&big);
    assert_eq!(result.encoding, None);
    assert!(result.beans.is_none());
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(
        result.diagnostics[0].code,
        beans_xml::DiagCode::OversizeInput
    );
}

#[test]
fn sb17_exactly_max_input_bytes_is_not_rejected_as_oversize() {
    // Boundary check on the cap itself: `MAX_INPUT_BYTES` bytes exactly is
    // within budget, only input strictly greater than it is oversize (see
    // `lib.rs::parse_bytes`'s own `>` comparison).
    let at_limit = vec![b' '; beans_xml::MAX_INPUT_BYTES];
    let result = beans_xml::parse_bytes(&at_limit);
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|d| d.code == beans_xml::DiagCode::OversizeInput),
        "input at exactly MAX_INPUT_BYTES must not be flagged oversize: {:?}",
        result.diagnostics
    );
}
