//! Unit **I2** — span invariants (SB-15), strengthened for M0b per
//! the internal build plan's I2 row. M0a already exercised
//! per-unit *bounds* proptests (each unit's own `#[cfg(test)]` module —
//! e.g. `inject_value`/`collection`'s own depth proptests double as bounds
//! checks); this file is the M0b addition the build plan calls out
//! specifically:
//!
//! - **Invariant #4, non-UTF-8 branch**: a generated document is
//!   re-encoded to EUC-KR / a declared-CP949-label / UTF-16LE+BOM /
//!   UTF-16BE+BOM, parsed via [`beans_xml::parse_bytes`], and every span of
//!   interest is **re-sliced by a consumer re-deriving the same decode
//!   from `ParseResult::encoding`** (never from anything internal to this
//!   crate) — proving the `encoding` label really is the re-slicing anchor
//!   the spec's `ByteSpan` doc comment promises, not just an informational
//!   string.
//! - **`NamespacedElement`/`<map><entry>`/`<props><prop>` self-span**: none
//!   of these three are `Spanned<T>`-wrapped (model contract: "self-span
//!   node ... not Spanned-wrapped = no double span") — pinning that their own
//!   `span` field slices to exactly their own element markup, and that the
//!   enclosing parent's span contains it (invariant #2, narrow slice).
//! - **CRLF**: `\r\n` inside element text and between elements is carried
//!   through raw (this crate does no XML line-end normalization — spans
//!   are byte offsets into the decoded string, entities/CDATA unresolved,
//!   per invariant #4's own "entities/CDATA unresolved" clause, and CRLF gets the
//!   same "raw, unresolved" treatment) and still slices correctly.
//! - **Invariant #3, determinism**: `parse`/`parse_bytes` produce a
//!   structurally identical `ParseResult` (which derives `PartialEq`) when
//!   run twice over the same bytes — arbitrary bytes, arbitrary unicode,
//!   and a generated well-formed document.
//!
//! Every type/function here is public (`beans_xml::parse`/`parse_bytes`,
//! the `model` re-exports) — this is a pure end-to-end integration test,
//! same convention every other `tests/i*.rs`/`tests/p*.rs` file follows.

use beans_xml::{ByteSpan, Collection, DiagCode, InjectValue};

fn slice(s: &str, span: ByteSpan) -> &str {
    &s[span.start as usize..span.end as usize]
}

/// A fixed, always-Hangul-bearing marker element, unconditionally present
/// in every generated document — guarantees at least one non-ASCII byte
/// regardless of what the proptest generator happens to pick for
/// `bean_id`/`class_suffix`/`prop_name`/`prop_value` (all four are also
/// allowed to be pure-ASCII shapes, e.g. `bean_id = "0"`). Without this, an
/// all-ASCII generated case would be **valid UTF-8 too** — `src/encoding.rs`'s
/// decode chain tries strict UTF-8 *before* the EUC-KR heuristic, so a
/// pure-ASCII document re-encoded "as EUC-KR" decodes right back out via
/// the UTF-8 branch (reality is genuinely ambiguous for ASCII-only bytes),
/// making `result.encoding` report `"UTF-8"` rather than the `"EUC-KR"`
/// this test means to exercise. This marker forces the non-ASCII branch
/// deterministically while leaving the four fields under test free to
/// range over the full ASCII+Hangul generator.
const HANGUL_MARKER: &str = "<description>빈설정</description>";

fn build_doc(bean_id: &str, class_suffix: &str, prop_name: &str, prop_value: &str) -> String {
    format!(
        "<beans>{HANGUL_MARKER}<bean id=\"{bean_id}\" class=\"com.example.{class_suffix}\">\
         <property name=\"{prop_name}\" value=\"{prop_value}\"/></bean></beans>"
    )
}

// ---------------------------------------------------------------------
// Re-decode helpers — a consumer, working only from raw bytes plus
// `ParseResult::encoding`, has to reproduce the exact same decode this
// crate performed internally (`src/encoding.rs`'s own "Span coarsening
// for re-encoded content" doc section). These mirror that module's
// decode chain for exactly the three non-UTF-8 families this test drives
// (EUC-KR/CP949 share one `encoding_rs::EUC_KR` codec; UTF-16LE/BE strip
// their own 2-byte BOM before decoding the remainder) — not a generic
// by-label dispatcher, since each proptest case below already knows which
// family it produced.
// ---------------------------------------------------------------------

fn decode_euc_kr(bytes: &[u8]) -> String {
    let (decoded, _, had_errors) = encoding_rs::EUC_KR.decode(bytes);
    assert!(
        !had_errors,
        "EUC-KR decode must not error on bytes this same test just encoded"
    );
    decoded.into_owned()
}

fn utf16le_bom_bytes(s: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for unit in s.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

fn utf16be_bom_bytes(s: &str) -> Vec<u8> {
    let mut out = vec![0xFE, 0xFF];
    for unit in s.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

fn decode_utf16le_bom(bytes: &[u8]) -> String {
    let rest = bytes
        .strip_prefix([0xFF, 0xFE].as_slice())
        .expect("expected a UTF-16LE BOM prefix");
    let (decoded, _, had_errors) = encoding_rs::UTF_16LE.decode(rest);
    assert!(!had_errors, "UTF-16LE decode must not error");
    decoded.into_owned()
}

fn decode_utf16be_bom(bytes: &[u8]) -> String {
    let rest = bytes
        .strip_prefix([0xFE, 0xFF].as_slice())
        .expect("expected a UTF-16BE BOM prefix");
    let (decoded, _, had_errors) = encoding_rs::UTF_16BE.decode(rest);
    assert!(!had_errors, "UTF-16BE decode must not error");
    decoded.into_owned()
}

/// Asserts the four spans of interest in a one-bean/one-property document
/// (bean id, property name, property value text) re-slice — via
/// `reslice`, a consumer-side re-decode of the raw bytes anchored on
/// `ParseResult::encoding` — to exactly the literal text this test
/// generated, plus the parsed `.value` fields agree structurally. Shared
/// by every non-UTF-8 proptest case below so each one only has to build
/// its own bytes + call the matching re-decode helper.
fn assert_reslice_matches_generated_text(
    beans: &beans_xml::BeansFile,
    reslice: &str,
    bean_id: &str,
    prop_name: &str,
    prop_value: &str,
) {
    assert_eq!(beans.beans.len(), 1, "expected exactly one top-level bean");
    let bean = &beans.beans[0];

    let id = bean.id.as_ref().expect("bean id present");
    assert_eq!(id.value, bean_id);
    assert_eq!(slice(reslice, id.span), bean_id);

    assert_eq!(bean.properties.len(), 1, "expected exactly one property");
    let property = &bean.properties[0];
    assert_eq!(property.name.value, prop_name);
    assert_eq!(slice(reslice, property.name.span), prop_name);

    match &property.value {
        InjectValue::Value(value_lit) => {
            assert_eq!(value_lit.text.value, prop_value);
            assert_eq!(slice(reslice, value_lit.text.span), prop_value);
        }
        other => panic!("expected InjectValue::Value, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// proptest: invariant #4, non-UTF-8 branch — EUC-KR / declared-CP949 /
// UTF-16LE+BOM / UTF-16BE+BOM.
// ---------------------------------------------------------------------

proptest::proptest! {
    #[test]
    fn i2_invariant4_euc_kr_bytes_reslice_via_encoding_label(
        bean_id in "[a-zA-Z0-9가-힣]{1,12}",
        class_suffix in "[a-zA-Z0-9가-힣]{1,12}",
        prop_name in "[a-zA-Z0-9가-힣]{1,12}",
        prop_value in "[a-zA-Z0-9가-힣]{1,12}",
    ) {
        let source = build_doc(&bean_id, &class_suffix, &prop_name, &prop_value);
        let (encoded, _, had_errors) = encoding_rs::EUC_KR.encode(&source);
        // Guard, not an expected outcome: every character this generator
        // produces (ASCII + the full Hangul syllable block) is covered by
        // `encoding_rs::EUC_KR` per `src/encoding.rs`'s own doc comment
        // ("WHATWG's \"euc-kr\" label is itself already the windows-949/UHC
        // superset decoder"), so this should never actually skip a case —
        // `prop_assume!` just keeps the property honest if that ever stops
        // being true instead of failing on an untested encodability edge.
        proptest::prop_assume!(!had_errors);
        let bytes = encoded.into_owned();

        let result = beans_xml::parse_bytes(&bytes);
        proptest::prop_assert_eq!(result.encoding.as_deref(), Some("EUC-KR"));
        proptest::prop_assert!(
            !result.diagnostics.iter().any(|d| d.code == DiagCode::EncodingMismatch),
            "undeclared EUC-KR input must not report a mismatch: {:?}", result.diagnostics
        );
        let beans = result.beans.expect("beans root");
        let reslice = decode_euc_kr(&bytes);
        assert_reslice_matches_generated_text(&beans, &reslice, &bean_id, &prop_name, &prop_value);
    }

    #[test]
    fn i2_invariant4_declared_cp949_bytes_reslice_via_encoding_label(
        bean_id in "[a-zA-Z0-9가-힣]{1,12}",
        class_suffix in "[a-zA-Z0-9가-힣]{1,12}",
        prop_name in "[a-zA-Z0-9가-힣]{1,12}",
        prop_value in "[a-zA-Z0-9가-힣]{1,12}",
    ) {
        // `<?xml ... encoding="CP949"?>` — the ASCII-only prolog encodes
        // identically under EUC-KR (ASCII-compatible), so the whole
        // document can go through a single `encode` call.
        let source = format!(
            "<?xml version=\"1.0\" encoding=\"CP949\"?>{}",
            build_doc(&bean_id, &class_suffix, &prop_name, &prop_value)
        );
        let (encoded, _, had_errors) = encoding_rs::EUC_KR.encode(&source);
        proptest::prop_assume!(!had_errors);
        let bytes = encoded.into_owned();

        let result = beans_xml::parse_bytes(&bytes);
        // src/encoding.rs's own doc comment: "CP949 and EUC-KR share one
        // encoding_rs decoder" — the declared label resolves to the same
        // codec that actually decodes it, so this is both the reported
        // name and a no-mismatch case.
        proptest::prop_assert_eq!(result.encoding.as_deref(), Some("EUC-KR"));
        proptest::prop_assert!(
            !result.diagnostics.iter().any(|d| d.code == DiagCode::EncodingMismatch),
            "a declared CP949 label that resolves to the actual decode must not mismatch: {:?}",
            result.diagnostics
        );
        let beans = result.beans.expect("beans root");
        let reslice = decode_euc_kr(&bytes);
        assert_reslice_matches_generated_text(&beans, &reslice, &bean_id, &prop_name, &prop_value);
    }

    #[test]
    fn i2_invariant4_utf16le_bom_bytes_reslice_via_encoding_label(
        bean_id in "[a-zA-Z0-9가-힣]{1,12}",
        class_suffix in "[a-zA-Z0-9가-힣]{1,12}",
        prop_name in "[a-zA-Z0-9가-힣]{1,12}",
        prop_value in "[a-zA-Z0-9가-힣]{1,12}",
    ) {
        let source = build_doc(&bean_id, &class_suffix, &prop_name, &prop_value);
        let bytes = utf16le_bom_bytes(&source);

        let result = beans_xml::parse_bytes(&bytes);
        proptest::prop_assert_eq!(result.encoding.as_deref(), Some("UTF-16LE"));
        let beans = result.beans.expect("beans root");
        let reslice = decode_utf16le_bom(&bytes);
        assert_reslice_matches_generated_text(&beans, &reslice, &bean_id, &prop_name, &prop_value);
    }

    #[test]
    fn i2_invariant4_utf16be_bom_bytes_reslice_via_encoding_label(
        bean_id in "[a-zA-Z0-9가-힣]{1,12}",
        class_suffix in "[a-zA-Z0-9가-힣]{1,12}",
        prop_name in "[a-zA-Z0-9가-힣]{1,12}",
        prop_value in "[a-zA-Z0-9가-힣]{1,12}",
    ) {
        let source = build_doc(&bean_id, &class_suffix, &prop_name, &prop_value);
        let bytes = utf16be_bom_bytes(&source);

        let result = beans_xml::parse_bytes(&bytes);
        proptest::prop_assert_eq!(result.encoding.as_deref(), Some("UTF-16BE"));
        let beans = result.beans.expect("beans root");
        let reslice = decode_utf16be_bom(&bytes);
        assert_reslice_matches_generated_text(&beans, &reslice, &bean_id, &prop_name, &prop_value);
    }
}

// ---------------------------------------------------------------------
// proptest: invariant #3 — `parse`/`parse_bytes` are deterministic:
// parsing the same bytes twice produces a structurally identical model
// (build plan I2 row: "the same bytes parsed twice → identical whole-model structure"). `ParseResult`
// — and everything it transitively owns (`BeansFile`, every `Diagnostic`,
// `encoding`) — derives `PartialEq` (model contract, `src/model.rs`), so a
// single `assert_eq!` over the whole `ParseResult` from two independent
// parses of the same input is a direct, total check of "entire model
// structure identical", not just spans or a hand-picked subset of fields.
// ---------------------------------------------------------------------

proptest::proptest! {
    #[test]
    fn i2_invariant3_parse_bytes_is_deterministic_over_arbitrary_bytes(
        bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..2000)
    ) {
        let first = beans_xml::parse_bytes(&bytes);
        let second = beans_xml::parse_bytes(&bytes);
        proptest::prop_assert_eq!(first, second);
    }

    #[test]
    fn i2_invariant3_parse_is_deterministic_over_arbitrary_unicode(s in ".{0,1000}") {
        let first = beans_xml::parse(&s);
        let second = beans_xml::parse(&s);
        proptest::prop_assert_eq!(first, second);
    }

    /// Reuses this file's own feature-dense builder (bean + property +
    /// Hangul marker) so, unlike the two arbitrary-input cases above
    /// (which mostly exercise the diagnostics-only recovery path),
    /// determinism is also pinned over a well-formed document that
    /// actually populates `BeansFile`/`Bean`/`Property`/`InjectValue`.
    #[test]
    fn i2_invariant3_parse_is_deterministic_over_generated_valid_doc(
        bean_id in "[a-zA-Z0-9가-힣]{1,12}",
        class_suffix in "[a-zA-Z0-9가-힣]{1,12}",
        prop_name in "[a-zA-Z0-9가-힣]{1,12}",
        prop_value in "[a-zA-Z0-9가-힣]{1,12}",
    ) {
        let source = build_doc(&bean_id, &class_suffix, &prop_name, &prop_value);
        let first = beans_xml::parse(&source);
        let second = beans_xml::parse(&source);
        proptest::prop_assert_eq!(first, second);
    }
}

// ---------------------------------------------------------------------
// NamespacedElement / <map><entry> / <props><prop> self-span (model
// contract: none of these three are `Spanned<T>`-wrapped) + parent ⊇
// child (invariant #2, narrow slice around each self-span node).
// ---------------------------------------------------------------------

#[test]
fn i2_namespaced_element_self_span_and_parent_contains_it() {
    let source = concat!(
        "<beans xmlns:jee=\"http://www.springframework.org/schema/jee\">",
        r#"<jee:jndi-lookup id="ds" jndi-name="java:comp/env/jdbc/DS"/>"#,
        "</beans>"
    );
    let beans = beans_xml::parse(source).beans.expect("beans root");
    assert_eq!(beans.namespaced.len(), 1);
    let element = &beans.namespaced[0];
    assert_eq!(
        slice(source, element.span),
        r#"<jee:jndi-lookup id="ds" jndi-name="java:comp/env/jdbc/DS"/>"#
    );
    assert!(
        beans.span.start <= element.span.start && element.span.end <= beans.span.end,
        "BeansFile span must contain its NamespacedElement child: {:?} vs {:?}",
        beans.span,
        element.span
    );
}

#[test]
fn i2_map_entry_self_span_and_parent_contains_it() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.A\">",
        r#"<property name="m"><map><entry key="k1" value="v1"/></map></property>"#,
        "</bean></beans>"
    );
    let beans = beans_xml::parse(source).beans.expect("beans root");
    let InjectValue::Collection(collection) = &beans.beans[0].properties[0].value else {
        panic!("expected InjectValue::Collection");
    };
    let Collection::Map { entries, .. } = &collection.value else {
        panic!("expected Collection::Map");
    };
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(slice(source, entry.span), r#"<entry key="k1" value="v1"/>"#);
    assert!(
        collection.span.start <= entry.span.start && entry.span.end <= collection.span.end,
        "the <map>'s own span must contain its <entry> child: {:?} vs {:?}",
        collection.span,
        entry.span
    );
}

#[test]
fn i2_props_prop_entry_self_span_and_parent_contains_it() {
    let source = concat!(
        "<beans><bean id=\"a\" class=\"com.example.A\">",
        r#"<property name="p"><props><prop key="k1">v1</prop></props></property>"#,
        "</bean></beans>"
    );
    let beans = beans_xml::parse(source).beans.expect("beans root");
    let InjectValue::Collection(collection) = &beans.beans[0].properties[0].value else {
        panic!("expected InjectValue::Collection");
    };
    let Collection::Props { entries, .. } = &collection.value else {
        panic!("expected Collection::Props");
    };
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(slice(source, entry.span), r#"<prop key="k1">v1</prop>"#);
    assert!(
        collection.span.start <= entry.span.start && entry.span.end <= collection.span.end,
        "the <props>'s own span must contain its <prop> child: {:?} vs {:?}",
        collection.span,
        entry.span
    );
}

// ---------------------------------------------------------------------
// CRLF — raw, unresolved (no XML line-end normalization this crate
// performs), still slices correctly.
// ---------------------------------------------------------------------

#[test]
fn i2_crlf_inside_element_text_is_preserved_and_slices_correctly() {
    let source =
        "<beans><bean id=\"a\" class=\"com.example.A\"><description>line1\r\nline2</description></bean></beans>";
    let beans = beans_xml::parse(source).beans.expect("beans root");
    let description = beans.beans[0]
        .description
        .as_ref()
        .expect("description present");
    assert_eq!(description.value, "line1\r\nline2");
    assert_eq!(slice(source, description.span), "line1\r\nline2");
}

#[test]
fn i2_crlf_between_elements_does_not_break_bean_span() {
    let source = "<beans>\r\n  <bean id=\"a\" class=\"com.example.A\"/>\r\n</beans>";
    let beans = beans_xml::parse(source).beans.expect("beans root");
    assert_eq!(beans.beans.len(), 1);
    assert_eq!(
        slice(source, beans.beans[0].span),
        "<bean id=\"a\" class=\"com.example.A\"/>"
    );
}
