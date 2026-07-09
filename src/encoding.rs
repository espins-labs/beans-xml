//! Unit **U2** — encoding detection: a global, label-driven decode chain.
//!
//! Spec: the internal design spec's "settled decisions" ("same as batis
//! MM-14's chain, verbatim") and SB-17's own row. Build order: the internal
//! build plan's U2 row — depends on U0 only, parallel with U1. This module mirrors the
//! sibling crate `batis-xml`'s `src/encoding.rs` (same chain, same
//! `encoding_rs`/WHATWG-label idioms) verbatim in shape; only the doc
//! comments' example fixtures are beans-xml-flavored (`<beans>` instead of
//! `<mapper>`).
//!
//! `pub(crate)` — like `src/events.rs`'s `build_tree`, this has no caller
//! yet. Wiring [`decode`]'s output into the public `parse`/`parse_bytes`
//! (combining it with `events::build_tree` and the root-child dispatch that
//! decides what `beans` actually contains) is **U3's job**, not this
//! unit's — see that module's own doc comment for the identical rationale.
//! Only this module's own `#[cfg(test)]` mod exercises [`decode`] so far;
//! `tests/u2_encoding.rs` carries a pointer here plus whatever is still
//! observable through the current public stub.
//!
//! Chain:
//! 1. **BOM sniff**: a UTF-8 BOM is skipped; a UTF-16 LE/BE BOM selects the
//!    matching `encoding_rs` decoder directly. A UTF-16 document has no
//!    ASCII-safe way to expose its own `<?xml ... encoding="...">`
//!    declaration (the prolog isn't ASCII bytes at all), so the BOM is the
//!    only signal available for it — no declared-vs-actual comparison is
//!    even attempted in that branch.
//! 2. **UTF-8 attempt** (strict — a single invalid byte falls through).
//! 3. **Declared label, trust-but-verify**: the XML declaration's
//!    `encoding` value is resolved via [`resolve_label`] (WHATWG's full
//!    label registry via `encoding_rs::Encoding::for_label`, covering
//!    Shift_JIS/EUC-JP, GBK/GB18030/Big5, Windows-125x/KOI8/ISO-8859-*,
//!    UTF-16, ... plus a small supplemental table for Windows/legacy
//!    Korean code-page names WHATWG doesn't register — see that function).
//!    If it resolves and decodes without errors (`had_errors == false`),
//!    it's used directly. An unrecognized label or a failing decode both
//!    fall through to the next step — reality wins either way.
//! 4. **EUC-KR heuristic fallback**: undeclared legacy Korean government/
//!    enterprise `<beans>` configs (spec's "encoding fixtures" note) are the
//!    one family this crate special-cases without a declaration at all. A
//!    heuristic, not a capability limit — disambiguating an undeclared
//!    Shift_JIS/GBK/etc. document is chardet-tier work and out of scope.
//! 5. **Lossy + `EncodingUndetectable`** if nothing above worked.
//!
//! When the declared encoding disagrees with whichever step actually
//! succeeded, **reality wins** plus an `EncodingMismatch` diagnostic —
//! [`declared_mismatch_diagnostic`] does this generically (resolve the
//! declared label, compare identity with the actual encoding used) rather
//! than a hardcoded per-family table, so it covers every alias
//! automatically.
//!
//! ## Span coarsening for re-encoded content
//!
//! Every `ByteSpan` this crate produces is computed by walking the
//! *decoded* `String` this module returns (see `ByteSpan`'s own doc
//! comment in `src/model.rs`) — for UTF-8 input (unchanged byte-for-byte
//! by decoding) those offsets are exactly the original bytes. For any
//! document decoded from a non-UTF-8 encoding (EUC-KR, CP949, UTF-16, ...),
//! decoding to UTF-8 changes character byte-widths, so spans are offsets
//! into the *re-encoded* UTF-8 string, not the original raw bytes — the
//! `encoding` label this module returns is the re-slicing anchor.

use crate::model::{DiagCode, Diagnostic};

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const UTF16LE_BOM: &[u8] = &[0xFF, 0xFE];
const UTF16BE_BOM: &[u8] = &[0xFE, 0xFF];

/// Decodes raw input bytes into a UTF-8 `String` plus whatever encoding
/// diagnostics the chain accumulated, and the WHATWG name of the decoder
/// actually used ([`encoding_rs::Encoding::name`]'s own value, e.g.
/// `"UTF-8"`, `"EUC-KR"`, `"UTF-16LE"`) — surfaced publicly as
/// `ParseResult::encoding` so a consumer working with the original input
/// bytes knows which decoder to reproduce this crate's byte offsets with.
/// The lossy fallback (step ⑤) reports `"UTF-8"`: the output is
/// `String::from_utf8_lossy`'s result, and a WHATWG `TextDecoder("utf-8")`
/// (non-fatal mode, the default) applies the same U+FFFD replacement for
/// invalid sequences, so it reproduces this exact output on the original
/// bytes.
///
/// Never panics (invariant #1) and never fails outright — every input,
/// including arbitrary/hostile bytes, produces *some* decoded string (the
/// lossy fallback is total), with `EncodingUndetectable` reported instead
/// of an `Err` when nothing more confident worked. The caller
/// (`MAX_INPUT_BYTES`-capped) is expected to reject oversize input
/// *before* ever calling this — that check lives in `lib.rs::parse_bytes`,
/// not here, since this module has no opinion on input size.
pub(crate) fn decode(bytes: &[u8]) -> (String, Vec<Diagnostic>, &'static str) {
    // ① BOM sniff. UTF-16 LE/BE BOMs are checked first (2-byte prefixes,
    // and a UTF-16 document's declaration can't be read as ASCII anyway,
    // so there's nothing else to check against here).
    if let Some(rest) = bytes.strip_prefix(UTF16LE_BOM) {
        let (decoded, _, had_errors) = encoding_rs::UTF_16LE.decode(rest);
        if !had_errors {
            return (
                decoded.into_owned(),
                Vec::new(),
                encoding_rs::UTF_16LE.name(),
            );
        }
    } else if let Some(rest) = bytes.strip_prefix(UTF16BE_BOM) {
        let (decoded, _, had_errors) = encoding_rs::UTF_16BE.decode(rest);
        if !had_errors {
            return (
                decoded.into_owned(),
                Vec::new(),
                encoding_rs::UTF_16BE.name(),
            );
        }
    }

    // A UTF-8 BOM is stripped before anything else below, so the
    // declaration scan doesn't fail to find "<?xml..." right after it
    // (str::trim_start only strips whitespace, not U+FEFF) and the
    // returned string never carries a leading U+FEFF either (spec's
    // ByteSpan doc: "A leading byte-order mark is never part of this text
    // at either entry point").
    let content = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    let declared = detect_declared_encoding(content);

    // ② UTF-8 attempt.
    if let Ok(s) = std::str::from_utf8(content) {
        let diagnostics = declared_mismatch_diagnostic(declared.as_deref(), encoding_rs::UTF_8)
            .into_iter()
            .collect();
        return (s.to_string(), diagnostics, encoding_rs::UTF_8.name());
    }

    // ③ Declared label, trust-but-verify.
    if let Some(label) = &declared {
        if let Some(enc) = resolve_label(label) {
            let (decoded, _, had_errors) = enc.decode(content);
            if !had_errors {
                let diagnostics = declared_mismatch_diagnostic(declared.as_deref(), enc)
                    .into_iter()
                    .collect();
                return (decoded.into_owned(), diagnostics, enc.name());
            }
        }
    }

    // ④ EUC-KR heuristic fallback.
    let (decoded, _, had_errors) = encoding_rs::EUC_KR.decode(content);
    if !had_errors {
        let diagnostics = declared_mismatch_diagnostic(declared.as_deref(), encoding_rs::EUC_KR)
            .into_iter()
            .collect();
        return (
            decoded.into_owned(),
            diagnostics,
            encoding_rs::EUC_KR.name(),
        );
    }

    // ⑤ Lossy + EncodingUndetectable.
    (
        String::from_utf8_lossy(content).into_owned(),
        vec![Diagnostic {
            code: DiagCode::EncodingUndetectable,
            span: None,
            message: "could not confidently detect an encoding (tried UTF-8, the declared label, \
                 EUC-KR/CP949 heuristic); decoded lossily"
                .to_string(),
        }],
        encoding_rs::UTF_8.name(),
    )
}

/// Resolves a declared encoding label to its `encoding_rs` encoding.
/// Tries the WHATWG-standard label registry first
/// (`encoding_rs::Encoding::for_label`) — this alone covers every standard
/// alias for every encoding `encoding_rs` implements (Shift_JIS, GB18030/
/// GBK, Big5, UTF-16, EUC-KR's own WHATWG-recognized aliases like
/// `windows-949`, ...; WHATWG's `"euc-kr"` label is itself already the
/// windows-949/UHC superset decoder, so a genuine CP949 document declared
/// as either name decodes identically — verified directly against
/// `encoding_rs` 0.8: encoding a CP949-only Hangul syllable outside the
/// older KS X 1001 2350-syllable table round-trips cleanly either way).
/// Falls back to a small supplemental table for Windows/legacy code-page
/// names for Korean text that WHATWG doesn't register at all (`CP949`,
/// `MS949`, `x-euc-kr`, `UHC`) but which are common in real
/// declared-encoding attributes from Windows-authored legacy Korean
/// government/enterprise tooling — all of them the same `encoding_rs::EUC_KR`
/// encoding in practice.
fn resolve_label(label: &str) -> Option<&'static encoding_rs::Encoding> {
    if let Some(enc) = encoding_rs::Encoding::for_label(label.as_bytes()) {
        return Some(enc);
    }
    let normalized: String = label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    match normalized.as_str() {
        "CP949" | "MS949" | "XEUCKR" | "UHC" => Some(encoding_rs::EUC_KR),
        _ => None,
    }
}

/// Compares the XML declaration's `encoding` (if any) against the `actual`
/// encoding reality settled on, returning a mismatch diagnostic when they
/// disagree (including when the declared label doesn't resolve to any
/// known encoding at all). Reality always wins — the caller has already
/// picked (and returns) the actually-successful decode regardless of what
/// this reports.
fn declared_mismatch_diagnostic(
    declared: Option<&str>,
    actual: &'static encoding_rs::Encoding,
) -> Option<Diagnostic> {
    let declared = declared?;
    if resolve_label(declared) == Some(actual) {
        return None;
    }
    Some(Diagnostic {
        code: DiagCode::EncodingMismatch,
        span: None,
        message: format!(
            "declared encoding '{declared}' does not match the actual encoding ({}); using {}",
            actual.name(),
            actual.name()
        ),
    })
}

/// Scans the first 1 KiB of raw bytes for `<?xml ... encoding="..." ?>`. A
/// plain byte-level scan, not a full XML parse — the prolog is always
/// ASCII-only per the XML spec, so `from_utf8_lossy` on this prefix is
/// safe and exact even when the rest of the document is in some other
/// ASCII-compatible encoding (whose ASCII-range bytes are identical to
/// ASCII/UTF-8 anyway). Not meaningful for UTF-16 input (handled entirely
/// by the BOM sniff before this is ever called).
///
/// The window is 1 KiB rather than the XML declaration's typical size
/// (`<?xml version="1.0" encoding="..."?>` is well under 100 bytes) to
/// tolerate padding before the terminator — e.g. an unusually long
/// `standalone`/extra pseudo-attribute, or excess whitespace — that would
/// otherwise push the real `?>` just past a tighter cutoff and silently
/// lose the declared label (M0b deferral: SB-17).
///
/// The `encoding=` search is bounded to the prolog itself (up to its own
/// terminator), not the whole window — otherwise a document whose first
/// element/attribute happens to be named `encoding=` within that window
/// (e.g. `<?xml version="1.0"?><bean encoding="..."/>`) would be misread
/// as if that were the XML declaration's own label. The terminator is
/// normally `?>`; a declaration malformed by dropping the `?` (closed
/// with a bare `>` instead) is still tolerated, so a declared `encoding=`
/// isn't lost just because the input is slightly malformed (same M0b
/// deferral). Both forms end in the same `>` byte, so bounding at the
/// *earliest* `>` in the window bounds the `encoding=` search identically
/// for either terminator — and, critically, never overshoots a bare-`>`-
/// closed declaration to a `?>` that belongs to something later in the
/// window (e.g. a processing instruction), which would misattribute that
/// later content's attributes as the declared encoding. A prolog with no
/// `>` at all within the window is treated as having no discoverable
/// declared encoding, same as any other unresolvable case.
fn detect_declared_encoding(bytes: &[u8]) -> Option<String> {
    const PROLOG_SCAN_WINDOW: usize = 1024;
    let head = &bytes[..bytes.len().min(PROLOG_SCAN_WINDOW)];
    let text = String::from_utf8_lossy(head);
    let trimmed = text.trim_start();
    if !trimmed.starts_with("<?xml") {
        return None;
    }
    let prolog_end = trimmed.find('>')?;
    let prolog = &trimmed[..prolog_end];
    let needle = "encoding=";
    let pos = prolog.find(needle)?;
    let rest = &prolog[pos + needle.len()..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

// ---------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------
//
// These stay in-module (rather than an external `tests/u2_encoding.rs`)
// because `decode` and its helpers are `pub(crate)` — a seam not visible
// from an external integration-test binary, same rationale
// `src/events.rs`'s own `#[cfg(test)] mod tests` documents for
// `build_tree`. `tests/u2_encoding.rs` carries a pointer here.
//
// Table (build plan U2 test design): UTF-8 / EUC-KR / CP949 / UTF-16 / a
// UTF-8-BOM-prefixed family, each crossed with {declared-matches,
// declared-mismatch (reality wins), no-declaration} wherever that
// distinction is architecturally meaningful for the family (UTF-16 is
// BOM-only — see the module doc comment's step ① note: no declared-label
// comparison is ever attempted once a BOM has selected the decoder, so
// that family's "declared" sub-case instead asserts the BOM overrides
// even a byte pattern that looks like a competing declaration).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DiagCode;

    fn euc_kr_bytes(s: &str) -> Vec<u8> {
        let (bytes, _, had_errors) = encoding_rs::EUC_KR.encode(s);
        assert!(!had_errors, "fixture text must be EUC-KR/CP949-encodable");
        bytes.into_owned()
    }

    fn utf16le_bytes(s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for unit in s.encode_utf16() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out
    }

    fn utf16be_bytes(s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for unit in s.encode_utf16() {
            out.extend_from_slice(&unit.to_be_bytes());
        }
        out
    }

    // -------------------------------------------------------------
    // UTF-8 family.
    // -------------------------------------------------------------

    #[test]
    fn u2_utf8_no_declaration_decodes_cleanly() {
        let bytes = r#"<beans><bean id="a"/></beans>"#.as_bytes();
        let (source, diagnostics, encoding) = decode(bytes);
        assert_eq!(source, r#"<beans><bean id="a"/></beans>"#);
        assert!(diagnostics.is_empty());
        assert_eq!(encoding, "UTF-8");
    }

    #[test]
    fn u2_utf8_declared_and_actual_agree_no_mismatch() {
        let bytes = br#"<?xml version="1.0" encoding="UTF-8"?><beans><bean id="a"/></beans>"#;
        let (_, diagnostics, encoding) = decode(bytes);
        assert!(diagnostics.is_empty());
        assert_eq!(encoding, "UTF-8");
    }

    #[test]
    fn u2_utf8_actual_but_declared_euckr_reality_wins_with_mismatch() {
        // Declared label lies (says EUC-KR) but the byte content is
        // genuinely valid UTF-8 — the strict UTF-8 attempt (step ②) wins
        // over the declared-label branch (step ③), same "reality wins"
        // contract regardless of which step it settles on.
        let bytes = "<?xml version=\"1.0\" encoding=\"EUC-KR\"?><beans><bean id=\"안녕\"/></beans>"
            .as_bytes();
        let (source, diagnostics, encoding) = decode(bytes);
        assert!(source.contains("안녕"));
        assert_eq!(encoding, "UTF-8");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, DiagCode::EncodingMismatch);
    }

    // -------------------------------------------------------------
    // EUC-KR family.
    // -------------------------------------------------------------

    #[test]
    fn u2_euckr_declared_and_actual_agree_no_mismatch() {
        let mut bytes = b"<?xml version=\"1.0\" encoding=\"EUC-KR\"?><beans><bean id=\"".to_vec();
        bytes.extend_from_slice(&euc_kr_bytes("그룹"));
        bytes.extend_from_slice(b"\"/></beans>");

        let (source, diagnostics, encoding) = decode(&bytes);
        assert!(source.contains("그룹"));
        assert!(diagnostics.is_empty());
        assert_eq!(encoding, "EUC-KR");
    }

    #[test]
    fn u2_euckr_declared_with_single_quotes_is_still_detected() {
        // `detect_declared_encoding`'s own quote-character check accepts
        // either `"` or `'` — the XML spec allows single-quoted attribute
        // values in the declaration (`<?xml version='1.0' ...?>`), and
        // every other declared-label test fixture in this module only ever
        // exercises the double-quoted form.
        let mut bytes = b"<?xml version='1.0' encoding='EUC-KR'?><beans><bean id=\"".to_vec();
        bytes.extend_from_slice(&euc_kr_bytes("그룹"));
        bytes.extend_from_slice(b"\"/></beans>");

        let (source, diagnostics, encoding) = decode(&bytes);
        assert!(source.contains("그룹"));
        assert!(diagnostics.is_empty());
        assert_eq!(encoding, "EUC-KR");
    }

    #[test]
    fn u2_euckr_actual_but_declared_utf8_reality_wins_with_mismatch() {
        let mut bytes = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><beans><bean id=\"".to_vec();
        bytes.extend_from_slice(&euc_kr_bytes("그룹"));
        bytes.extend_from_slice(b"\"/></beans>");

        let (source, diagnostics, encoding) = decode(&bytes);
        assert!(source.contains("그룹")); // reality (EUC-KR) wins, not the false UTF-8 claim
        assert_eq!(encoding, "EUC-KR");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, DiagCode::EncodingMismatch);
    }

    #[test]
    fn u2_euckr_no_declaration_falls_back_to_heuristic() {
        // Undeclared legacy Korean config (step ④) — no XML declaration at
        // all, so `declared` is `None` and no mismatch is possible.
        let mut bytes = b"<beans><bean id=\"".to_vec();
        bytes.extend_from_slice(&euc_kr_bytes("그룹"));
        bytes.extend_from_slice(b"\"/></beans>");

        let (source, diagnostics, encoding) = decode(&bytes);
        assert!(source.contains("그룹"));
        assert!(diagnostics.is_empty());
        assert_eq!(encoding, "EUC-KR");
    }

    // -------------------------------------------------------------
    // CP949 family (declared label resolves via the supplemental table;
    // the fixture text itself uses a Hangul syllable outside the older
    // KS X 1001 EUC-KR 2350-syllable table to exercise the CP949/
    // windows-949 extension range specifically).
    // -------------------------------------------------------------

    #[test]
    fn u2_cp949_declared_label_resolves_and_matches_no_mismatch() {
        let mut bytes = b"<?xml version=\"1.0\" encoding=\"CP949\"?><beans><bean id=\"".to_vec();
        bytes.extend_from_slice(&euc_kr_bytes("뷁"));
        bytes.extend_from_slice(b"\"/></beans>");

        let (source, diagnostics, encoding) = decode(&bytes);
        assert!(source.contains("뷁"));
        assert!(diagnostics.is_empty());
        assert_eq!(encoding, "EUC-KR"); // CP949 and EUC-KR share one encoding_rs decoder
    }

    #[test]
    fn u2_cp949_actual_but_declared_utf8_reality_wins_with_mismatch() {
        let mut bytes = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><beans><bean id=\"".to_vec();
        bytes.extend_from_slice(&euc_kr_bytes("뷁"));
        bytes.extend_from_slice(b"\"/></beans>");

        let (source, diagnostics, encoding) = decode(&bytes);
        assert!(source.contains("뷁"));
        assert_eq!(encoding, "EUC-KR");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, DiagCode::EncodingMismatch);
    }

    #[test]
    fn u2_cp949_no_declaration_falls_back_to_heuristic() {
        let mut bytes = b"<beans><bean id=\"".to_vec();
        bytes.extend_from_slice(&euc_kr_bytes("뷁"));
        bytes.extend_from_slice(b"\"/></beans>");

        let (source, diagnostics, encoding) = decode(&bytes);
        assert!(source.contains("뷁"));
        assert!(diagnostics.is_empty());
        assert_eq!(encoding, "EUC-KR");
    }

    // -------------------------------------------------------------
    // UTF-16 family (BOM-only — see module doc comment step ①).
    // -------------------------------------------------------------

    #[test]
    fn u2_utf16le_bom_no_declaration_decodes_cleanly() {
        let body = r#"<beans><bean id="a"/></beans>"#;
        let mut bytes = UTF16LE_BOM.to_vec();
        bytes.extend_from_slice(&utf16le_bytes(body));

        let (source, diagnostics, encoding) = decode(&bytes);
        assert_eq!(source, body);
        assert!(diagnostics.is_empty());
        assert_eq!(encoding, "UTF-16LE");
    }

    #[test]
    fn u2_utf16be_bom_no_declaration_decodes_cleanly() {
        let body = r#"<beans><bean id="a"/></beans>"#;
        let mut bytes = UTF16BE_BOM.to_vec();
        bytes.extend_from_slice(&utf16be_bytes(body));

        let (source, diagnostics, encoding) = decode(&bytes);
        assert_eq!(source, body);
        assert!(diagnostics.is_empty());
        assert_eq!(encoding, "UTF-16BE");
    }

    #[test]
    fn u2_utf16le_bom_wins_even_against_a_competing_byte_pattern() {
        // "Declared-mismatch" sub-case for the BOM-only UTF-16 family: the
        // decoded body itself contains what would look like a conflicting
        // `encoding="EUC-KR"` declaration if this were ever byte-scanned —
        // it never is, because the BOM branch returns before
        // `detect_declared_encoding` is even called. The BOM's own choice
        // always wins, unconditionally, for this family.
        let body = r#"<beans><!-- encoding="EUC-KR" --><bean id="a"/></beans>"#;
        let mut bytes = UTF16LE_BOM.to_vec();
        bytes.extend_from_slice(&utf16le_bytes(body));

        let (source, diagnostics, encoding) = decode(&bytes);
        assert_eq!(source, body);
        assert!(diagnostics.is_empty());
        assert_eq!(encoding, "UTF-16LE");
    }

    #[test]
    fn u2_utf16be_bom_wins_even_against_a_competing_byte_pattern() {
        // Same "declared-mismatch" sub-case as the LE test above, mirrored
        // for the big-endian BOM — the table's UTF-16LE/BE split (build
        // plan's I4 row) calls both out explicitly, not just one.
        let body = r#"<beans><!-- encoding="EUC-KR" --><bean id="a"/></beans>"#;
        let mut bytes = UTF16BE_BOM.to_vec();
        bytes.extend_from_slice(&utf16be_bytes(body));

        let (source, diagnostics, encoding) = decode(&bytes);
        assert_eq!(source, body);
        assert!(diagnostics.is_empty());
        assert_eq!(encoding, "UTF-16BE");
    }

    // -------------------------------------------------------------
    // UTF-8-BOM-prefixed family.
    // -------------------------------------------------------------

    #[test]
    fn u2_utf8_bom_no_declaration_stripped_and_absent_from_output() {
        let mut bytes = UTF8_BOM.to_vec();
        bytes.extend_from_slice(r#"<beans><bean id="a"/></beans>"#.as_bytes());

        let (source, diagnostics, encoding) = decode(&bytes);
        assert_eq!(source, r#"<beans><bean id="a"/></beans>"#);
        assert!(!source.contains('\u{FEFF}'));
        assert!(diagnostics.is_empty());
        assert_eq!(encoding, "UTF-8");
    }

    #[test]
    fn u2_utf8_bom_declared_and_actual_agree_no_mismatch() {
        let mut bytes = UTF8_BOM.to_vec();
        bytes.extend_from_slice(
            br#"<?xml version="1.0" encoding="UTF-8"?><beans><bean id="a"/></beans>"#,
        );

        let (source, diagnostics, encoding) = decode(&bytes);
        assert!(source.starts_with("<?xml"));
        assert!(diagnostics.is_empty());
        assert_eq!(encoding, "UTF-8");
    }

    #[test]
    fn u2_utf8_bom_before_declaration_does_not_defeat_mismatch_detection() {
        // A UTF-8 BOM directly followed by an XML declaration must not
        // make `detect_declared_encoding`'s "starts with <?xml" check fail
        // (`str::trim_start` doesn't strip U+FEFF) and silently lose the
        // declared label — the mismatch below must still fire.
        let mut bytes = UTF8_BOM.to_vec();
        bytes.extend_from_slice(
            br#"<?xml version="1.0" encoding="EUC-KR"?><beans><bean id="a"/></beans>"#,
        );

        let (source, diagnostics, encoding) = decode(&bytes);
        assert!(source.starts_with("<?xml"));
        assert!(!source.contains('\u{FEFF}'));
        assert_eq!(encoding, "UTF-8");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, DiagCode::EncodingMismatch);
    }

    // -------------------------------------------------------------
    // Undecodable / lossy fallback.
    // -------------------------------------------------------------

    #[test]
    fn u2_undecodable_bytes_decode_lossily_with_encoding_undetectable() {
        // Invalid in both UTF-8 and EUC-KR, and deliberately *not* starting
        // with a UTF-16 BOM (0xFF/0xFE-led sequences would instead enter
        // the BOM sniff in step ① — a `had_errors` UTF-16 decode there
        // doesn't reject the input, it simply falls through to steps
        // ②-④ same as any other bytes, so this fixture avoids that path
        // entirely rather than exercising it). 0x80 is a bare UTF-8
        // continuation byte with no lead byte (invalid UTF-8) and also not
        // a valid EUC-KR lead byte.
        let bytes: &[u8] = &[
            0x80, 0x81, 0x82, b'<', b'b', b'e', b'a', b'n', b's', b'/', b'>',
        ];
        let (_, diagnostics, encoding) = decode(bytes);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, DiagCode::EncodingUndetectable);
        assert_eq!(encoding, "UTF-8"); // lossy decode's own reported label
    }

    #[test]
    fn u2_unknown_declared_label_falls_through_to_next_step_with_mismatch() {
        let bytes = br#"<?xml version="1.0" encoding="totally-not-a-real-encoding"?><beans/>"#;
        let (source, diagnostics, encoding) = decode(bytes);
        assert!(source.contains("<beans"));
        assert_eq!(encoding, "UTF-8");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, DiagCode::EncodingMismatch);
    }

    #[test]
    fn u2_declared_scan_stops_at_prolog_end_no_false_mismatch_from_later_encoding_attr() {
        // The document is genuinely UTF-8 with no `encoding=` in its own
        // prolog; an unrelated `encoding` *attribute* on the root element
        // appears within the scan window. `detect_declared_encoding`
        // must stop scanning at the prolog's own terminator, not keep
        // scanning the whole window, or it would misread that
        // attribute's value as the declared document encoding and produce
        // a spurious `EncodingMismatch` (declared "utf-16" vs actual
        // UTF-8) even though nothing was actually declared.
        let bytes = br#"<?xml version="1.0"?><bean encoding="utf-16"/>"#;
        let (source, diagnostics, encoding) = decode(bytes);
        assert!(source.contains("encoding=\"utf-16\""));
        assert_eq!(encoding, "UTF-8");
        assert!(diagnostics.is_empty());
    }

    // -------------------------------------------------------------
    // SB-17 M0b deferral fix: the declared-encoding detection window.
    // Widened from 200 bytes to 1 KiB, and the `?>` terminator search
    // now tolerates a malformed declaration closed with a bare `>`.
    // -------------------------------------------------------------

    #[test]
    fn u2_declared_label_past_old_200_byte_window_is_still_detected() {
        // A long (but well-formed, `?>`-terminated) prolog whose closing
        // `?>` sits past the old 200-byte scan cutoff, comfortably inside
        // the new 1 KiB window. The declared label ("UTF-8") disagrees
        // with the actual content (genuine EUC-KR bytes), so the mismatch
        // firing at all is the direct evidence the label was found beyond
        // byte 200 — under the old bug, `detect_declared_encoding` would
        // return `None` here (its own 200-byte `head` slice never even
        // contains "?>"), silently dropping both the label and the
        // `EncodingMismatch` diagnostic despite reality still winning on
        // the decoded content.
        let padding = "x".repeat(220);
        let mut bytes =
            format!(r#"<?xml version="1.0" encoding="UTF-8" padding="{padding}"?>"#).into_bytes();
        assert!(
            bytes.len() > 200,
            "fixture must push '?>' past the old 200-byte cutoff"
        );
        assert!(
            bytes.len() < 1024,
            "fixture must still land inside the new 1 KiB window"
        );
        bytes.extend_from_slice(b"<beans><bean id=\"");
        bytes.extend_from_slice(&euc_kr_bytes("그룹"));
        bytes.extend_from_slice(b"\"/></beans>");

        let (source, diagnostics, encoding) = decode(&bytes);
        assert!(source.contains("그룹")); // reality (EUC-KR) wins
        assert_eq!(encoding, "EUC-KR");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, DiagCode::EncodingMismatch);
    }

    #[test]
    fn u2_declaration_closed_with_bare_gt_declared_label_still_detected_matching() {
        // Malformed declaration missing the closing `?` (`>` instead of
        // `?>`) — still a common real-world typo. The declared label
        // ("EUC-KR") is discovered via the bare-`>` fallback and matches
        // reality, so no mismatch.
        let mut bytes = b"<?xml version=\"1.0\" encoding=\"EUC-KR\"><beans><bean id=\"".to_vec();
        bytes.extend_from_slice(&euc_kr_bytes("그룹"));
        bytes.extend_from_slice(b"\"/></beans>");

        let (source, diagnostics, encoding) = decode(&bytes);
        assert!(source.contains("그룹"));
        assert!(diagnostics.is_empty());
        assert_eq!(encoding, "EUC-KR");
    }

    #[test]
    fn u2_declaration_closed_with_bare_gt_mismatch_still_detected() {
        // Same `>`-closed malformed declaration, but this time the
        // declared label ("UTF-8") disagrees with reality (genuine
        // EUC-KR bytes) — under the old bug (`find("?>")` only, no bare-
        // `>` fallback), this declaration would be invisible entirely
        // (no "?>" anywhere in the input), silently dropping the
        // `EncodingMismatch` diagnostic.
        let mut bytes = b"<?xml version=\"1.0\" encoding=\"UTF-8\"><beans><bean id=\"".to_vec();
        bytes.extend_from_slice(&euc_kr_bytes("그룹"));
        bytes.extend_from_slice(b"\"/></beans>");

        let (source, diagnostics, encoding) = decode(&bytes);
        assert!(source.contains("그룹")); // reality (EUC-KR) wins
        assert_eq!(encoding, "EUC-KR");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, DiagCode::EncodingMismatch);
    }

    #[test]
    fn u2_declaration_closed_with_bare_gt_stops_before_later_pi_terminator() {
        // Bare-`>`-closed declaration (no `encoding=` in it) followed
        // later in the scan window by a processing instruction whose own
        // `?>` terminator would, if the scan preferred `?>` over the
        // earliest `>`, extend the "prolog" slice past the declaration
        // and into that PI's content — misreading its `data` as if it
        // were `encoding=`. This is exactly the adversarial case the
        // earliest-terminator fix discriminates: the three "bare `>`"
        // tests above never exercise it because they have no `?` at all
        // after the declaration.
        let bytes = br#"<?xml version="1.0"><beans><bean encoding="utf-16"/></beans><?pi data?>"#;
        let (source, diagnostics, encoding) = decode(bytes);
        assert!(source.contains("encoding=\"utf-16\""));
        assert_eq!(encoding, "UTF-8");
        assert!(
            diagnostics.is_empty(),
            "bare-`>`-closed declaration must not misattribute later element/PI \
             content as the declared encoding: {diagnostics:?}"
        );
    }

    #[test]
    fn u2_prolog_terminator_beyond_1kib_window_still_treated_as_no_declaration() {
        // The widened window has its own edge: content past 1 KiB is
        // still out of scan range by design (this is a bounded byte-level
        // sniff, not a full parse) — such a document is treated the same
        // as one with no discoverable declared encoding, same as any
        // other unresolvable case, and no mismatch is fabricated.
        let padding = "x".repeat(1100);
        let mut bytes =
            format!(r#"<?xml version="1.0" encoding="UTF-8" padding="{padding}"?>"#).into_bytes();
        assert!(
            bytes.len() > 1024,
            "fixture must push '?>' past the widened 1 KiB window"
        );
        bytes.extend_from_slice(b"<beans><bean id=\"");
        bytes.extend_from_slice(&euc_kr_bytes("그룹"));
        bytes.extend_from_slice(b"\"/></beans>");

        let (source, diagnostics, encoding) = decode(&bytes);
        assert!(source.contains("그룹")); // heuristic fallback still decodes it
        assert_eq!(encoding, "EUC-KR");
        assert!(diagnostics.is_empty()); // no declared label found -> no mismatch possible
    }
}
