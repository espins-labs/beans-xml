//! Unit **U0** — output model tests.
//!
//! Per the internal build plan's U0 test design:
//! (b) bidirectional round trip per `InjectValue`/`Collection`/`DiagCode`
//!     variant (`assert_eq!(from_json(to_json(x)), x)`),
//! (c) unknown-`kind` JSON deserializes to `Unrecognized` and stays there
//!     through a further serialize/deserialize cycle (invariant #6),
//! (d) `schema/beans-xml.v1.json` snapshot stub (gated on the optional
//!     `schema` feature — not part of the default `cargo test` gate),
//! (e) a default-gate field-name shape lock for the two `#[serde(rename)]`
//!     fields (`Bean.abstract_`, `Qualifier.type_`) that (b)/(c)'s pure
//!     round trips cannot detect a regression in — see that section's own
//!     doc comment for why.
//!
//! Test names are prefixed `sb_u0_` per this crate's `AGENTS.md` naming
//! convention (unit id prefix for traceability).

use beans_xml::*;

fn span(start: u32, end: u32) -> ByteSpan {
    ByteSpan { start, end }
}

fn spanned<T>(value: T, start: u32, end: u32) -> Spanned<T> {
    Spanned {
        value,
        span: span(start, end),
    }
}

fn sample_bean_ref(raw: &str, kind: RefKind) -> Spanned<BeanRef> {
    spanned(
        BeanRef {
            raw: raw.to_string(),
            kind,
        },
        0,
        raw.len() as u32,
    )
}

fn sample_value_lit(text: &str) -> ValueLit {
    ValueLit {
        span: span(0, text.len() as u32),
        text: spanned(text.to_string(), 0, text.len() as u32),
        value_type: None,
        placeholders: vec![],
        spel_refs: vec![],
    }
}

fn sample_bean() -> Bean {
    Bean {
        span: span(0, 20),
        id: Some(spanned("target".to_string(), 0, 6)),
        names: vec![],
        class: Some(spanned(
            ClassRef {
                raw: "com.example.Widget".to_string(),
            },
            0,
            19,
        )),
        parent: None,
        scope: None,
        abstract_: false,
        lazy_init: None,
        primary: false,
        autowire: None,
        autowire_candidate: None,
        depends_on: vec![],
        factory_bean: None,
        factory_method: None,
        init_method: None,
        destroy_method: None,
        properties: vec![],
        constructor_args: vec![],
        lookup_methods: vec![],
        replaced_methods: vec![],
        qualifiers: vec![],
        decorators: vec![],
        description: None,
        meta: vec![],
    }
}

/// Serialize `value`, deserialize it back, and assert the result is
/// unchanged — the bidirectional round trip this unit's tests are built
/// around (build plan U0 test (b)).
fn round_trip<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let json = serde_json::to_string(value).expect("serializes");
    let back: T = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(&back, value, "round trip changed value; json was: {json}");
}

// ---------------------------------------------------------------------
// InjectValue: bidirectional round trip per variant.
// ---------------------------------------------------------------------

#[test]
fn sb_u0_inject_value_ref_round_trips() {
    round_trip(&InjectValue::Ref(sample_bean_ref("target", RefKind::Bean)));
}

#[test]
fn sb_u0_inject_value_idref_round_trips() {
    round_trip(&InjectValue::Idref(sample_bean_ref(
        "target",
        RefKind::Local,
    )));
}

#[test]
fn sb_u0_inject_value_value_round_trips() {
    round_trip(&InjectValue::Value(sample_value_lit("hello")));
}

#[test]
fn sb_u0_inject_value_inner_round_trips() {
    round_trip(&InjectValue::Inner(Box::new(sample_bean())));
}

#[test]
fn sb_u0_inject_value_collection_round_trips() {
    let coll = Collection::List {
        items: vec![],
        value_type: None,
        merge: Some(false),
    };
    round_trip(&InjectValue::Collection(spanned(coll, 0, 10)));
}

#[test]
fn sb_u0_inject_value_null_round_trips() {
    round_trip(&InjectValue::Null(span(0, 6)));
}

#[test]
fn sb_u0_inject_value_unrecognized_round_trips() {
    round_trip(&InjectValue::Unrecognized);
}

// ---------------------------------------------------------------------
// Collection: bidirectional round trip per variant.
// ---------------------------------------------------------------------

#[test]
fn sb_u0_collection_list_round_trips() {
    round_trip(&Collection::List {
        items: vec![InjectValue::Null(span(0, 4))],
        value_type: Some(spanned(
            ClassRef {
                raw: "java.lang.String".into(),
            },
            0,
            10,
        )),
        merge: Some(true),
    });
}

#[test]
fn sb_u0_collection_set_round_trips() {
    round_trip(&Collection::Set {
        items: vec![],
        value_type: None,
        merge: None,
    });
}

#[test]
fn sb_u0_collection_array_round_trips() {
    round_trip(&Collection::Array {
        items: vec![],
        value_type: None,
        merge: None,
    });
}

#[test]
fn sb_u0_collection_map_round_trips() {
    let entry = MapEntry {
        span: span(0, 10),
        key: InjectValue::Value(sample_value_lit("k")),
        value: InjectValue::Value(sample_value_lit("v")),
        value_type: None,
    };
    round_trip(&Collection::Map {
        entries: vec![entry],
        key_type: Some(spanned(
            ClassRef {
                raw: "java.lang.String".into(),
            },
            0,
            10,
        )),
        value_type: None,
        merge: None,
    });
}

#[test]
fn sb_u0_collection_props_round_trips() {
    let entry = PropEntry {
        span: span(0, 10),
        key: spanned("k".to_string(), 0, 1),
        value: sample_value_lit("v"),
    };
    round_trip(&Collection::Props {
        entries: vec![entry],
        merge: None,
    });
}

#[test]
fn sb_u0_collection_unrecognized_round_trips() {
    round_trip(&Collection::Unrecognized);
}

// ---------------------------------------------------------------------
// DiagCode: bidirectional round trip per variant.
// ---------------------------------------------------------------------

#[test]
fn sb_u0_diag_code_all_variants_round_trip() {
    let variants = [
        DiagCode::EncodingUndetectable,
        DiagCode::EncodingMismatch,
        DiagCode::UnclosedTag,
        DiagCode::UnexpectedCloseTag,
        DiagCode::DuplicateAttribute,
        DiagCode::NotBeansRoot,
        DiagCode::DuplicateBeanId,
        DiagCode::BeanWithoutClassOrParent,
        DiagCode::ConflictingValueAndRef,
        DiagCode::RefWithoutTarget,
        DiagCode::UnknownElement,
        DiagCode::InvalidEntity,
        DiagCode::UnterminatedPlaceholder,
        DiagCode::NestingLimitExceeded,
        DiagCode::OversizeInput,
        DiagCode::Other,
    ];
    for code in variants {
        let diag = Diagnostic {
            code,
            span: Some(span(1, 2)),
            message: "x".to_string(),
        };
        round_trip(&diag);
    }
}

// ---------------------------------------------------------------------
// Invariant #6: an unknown `kind` JSON payload absorbs into
// `Unrecognized` and stays there through a further round trip — the
// adjacently-tagged + `#[serde(other)]` forward-compat mechanism.
// ---------------------------------------------------------------------

#[test]
fn sb_u0_inject_value_unknown_kind_deserializes_to_unrecognized_and_stays_stable() {
    let json = r#"{"kind":"some_future_variant","content":{"anything":"goes"}}"#;
    let v: InjectValue = serde_json::from_str(json).expect("deserializes, doesn't fail");
    assert_eq!(v, InjectValue::Unrecognized);

    // Re-serialize, re-deserialize: still Unrecognized (stability, not
    // just a one-shot absorb).
    let re_json = serde_json::to_string(&v).expect("serializes");
    let re_v: InjectValue = serde_json::from_str(&re_json).expect("deserializes again");
    assert_eq!(re_v, InjectValue::Unrecognized);
}

#[test]
fn sb_u0_collection_unknown_kind_deserializes_to_unrecognized_and_stays_stable() {
    let json = r#"{"kind":"some_future_collection","content":{"anything":"goes"}}"#;
    let c: Collection = serde_json::from_str(json).expect("deserializes, doesn't fail");
    assert_eq!(c, Collection::Unrecognized);

    let re_json = serde_json::to_string(&c).expect("serializes");
    let re_c: Collection = serde_json::from_str(&re_json).expect("deserializes again");
    assert_eq!(re_c, Collection::Unrecognized);
}

#[test]
fn sb_u0_diag_code_unknown_string_deserializes_to_other() {
    let json = r#"{"code":"some_future_diag_code","span":null,"message":"x"}"#;
    let d: Diagnostic = serde_json::from_str(json).expect("deserializes, doesn't fail");
    assert_eq!(d.code, DiagCode::Other);
}

// A known `kind`/`code` must still resolve to its real variant, not fall
// through the `serde(other)` escape hatch by accident.

#[test]
fn sb_u0_inject_value_known_kind_deserializes_normally_not_via_fallback() {
    let json = r#"{"kind":"null","content":{"start":0,"end":4}}"#;
    let v: InjectValue = serde_json::from_str(json).expect("deserializes");
    assert_eq!(v, InjectValue::Null(span(0, 4)));
}

#[test]
fn sb_u0_collection_known_kind_deserializes_normally_not_via_fallback() {
    let json = r#"{"kind":"set","content":{"items":[],"value_type":null,"merge":null}}"#;
    let c: Collection = serde_json::from_str(json).expect("deserializes");
    assert_eq!(
        c,
        Collection::Set {
            items: vec![],
            value_type: None,
            merge: None
        }
    );
}

#[test]
fn sb_u0_diag_code_known_string_deserializes_normally_not_via_fallback() {
    let json = r#"{"code":"unclosed_tag","span":null,"message":"x"}"#;
    let d: Diagnostic = serde_json::from_str(json).expect("deserializes");
    assert_eq!(d.code, DiagCode::UnclosedTag);
}

// ---------------------------------------------------------------------
// Whole-document sanity: a hand-built BeansFile with a bit of everything
// still round trips (catches a field wired to the wrong JSON shape that
// per-type tests above wouldn't).
// ---------------------------------------------------------------------

#[test]
fn sb_u0_parse_result_with_populated_beans_file_round_trips() {
    let bean = sample_bean();
    let beans_file = BeansFile {
        span: span(0, 100),
        profile: Some(spanned("dev,test".to_string(), 8, 16)),
        description: None,
        default_lazy_init: Some(true),
        default_autowire: None,
        default_init_method: None,
        default_destroy_method: Some(spanned(String::new(), 0, 0)),
        default_merge: None,
        default_autowire_candidates: None,
        imports: vec![spanned(
            Import {
                resource: spanned("classpath:other.xml".to_string(), 0, 19),
                kind: ImportKind::Classpath,
            },
            0,
            19,
        )],
        aliases: vec![spanned(
            Alias {
                name: spanned("target".to_string(), 0, 6),
                alias: spanned("t".to_string(), 10, 11),
            },
            0,
            11,
        )],
        beans: vec![bean],
        component_scans: vec![],
        property_sources: vec![spanned(
            PropertySource::Properties {
                id: Some(spanned("props".to_string(), 0, 5)),
                location: Some(spanned("classpath:app.properties".to_string(), 10, 34)),
            },
            0,
            34,
        )],
        namespaced: vec![NamespacedElement {
            ns: "http://www.springframework.org/schema/jee".to_string(),
            local: "jndi-lookup".to_string(),
            span: span(0, 30),
            id: Some(spanned("dataSource".to_string(), 5, 15)),
            attrs: vec![],
            refs: vec![],
        }],
        nested_profiles: vec![],
    };
    let result = ParseResult {
        beans: Some(beans_file),
        encoding: Some("UTF-8".to_string()),
        diagnostics: vec![Diagnostic {
            code: DiagCode::DuplicateBeanId,
            span: Some(span(0, 6)),
            message: "duplicate bean id 'target'".to_string(),
        }],
    };
    round_trip(&result);
}

// ---------------------------------------------------------------------
// Field-name shape lock (runs under the default `cargo test` gate, no
// features required).
//
// The round-trip tests above (`from_json(to_json(x)) == x`) cannot catch a
// dropped `#[serde(rename = ...)]`: `Serialize`/`Deserialize` are derived
// together, so if a refactor removed a rename from both sides in lockstep,
// every round-trip test above would still pass while the published JSON
// field name silently changed. The only test that pins the actual wire
// shape against the committed schema snapshot is
// `sb_u0_schema_matches_committed_snapshot` below, but it only runs under
// `--features schema`, which no CI job currently enables (see
// `.github/workflows/ci.yml`). These two tests are the default-gate
// stopgap: they assert the specific renamed field names this crate relies
// on (`abstract`, `type` — both Rust keywords) appear literally in the
// serialized JSON, so a dropped rename fails plain `cargo test`.
// ---------------------------------------------------------------------

#[test]
fn sb_u0_bean_abstract_field_serializes_as_reserved_word_not_suffixed() {
    let bean = sample_bean();
    let json = serde_json::to_string(&bean).expect("serializes");
    assert!(
        json.contains("\"abstract\":"),
        "Bean.abstract_ must serialize as the JSON field \"abstract\" (see \
         #[serde(rename = \"abstract\")] in src/model.rs); json was: {json}"
    );
    assert!(
        !json.contains("\"abstract_\""),
        "Bean.abstract_ leaked its Rust field name \"abstract_\" into JSON \
         instead of the renamed \"abstract\"; json was: {json}"
    );
}

#[test]
fn sb_u0_qualifier_type_field_serializes_as_reserved_word_not_suffixed() {
    let qualifier = Qualifier {
        span: span(0, 10),
        type_: Some(spanned(
            ClassRef {
                raw: "com.example.Q".to_string(),
            },
            0,
            10,
        )),
        value: None,
        attributes: vec![],
    };
    let json = serde_json::to_string(&qualifier).expect("serializes");
    assert!(
        json.contains("\"type\":"),
        "Qualifier.type_ must serialize as the JSON field \"type\" (see \
         #[serde(rename = \"type\")] in src/model.rs); json was: {json}"
    );
    assert!(
        !json.contains("\"type_\""),
        "Qualifier.type_ leaked its Rust field name \"type_\" into JSON \
         instead of the renamed \"type\"; json was: {json}"
    );
}

// ---------------------------------------------------------------------
// Schema snapshot stub (build plan U0 test (d)) — gated on the optional
// `schema` feature, so it doesn't run under the plain `cargo test` gate.
// ---------------------------------------------------------------------

#[cfg(feature = "schema")]
#[test]
fn sb_u0_schema_matches_committed_snapshot() {
    let schema = schemars::schema_for!(beans_xml::ParseResult);
    let generated = serde_json::to_string_pretty(&schema).expect("schema serializes");
    let committed = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/schema/beans-xml.v1.json"
    ))
    .expect("schema/beans-xml.v1.json must exist — see examples/gen_schema.rs");
    assert_eq!(
        generated.trim_end(),
        committed.trim_end(),
        "schema/beans-xml.v1.json is stale — regenerate with \
         `cargo run --example gen_schema --features schema > schema/beans-xml.v1.json`"
    );
}
