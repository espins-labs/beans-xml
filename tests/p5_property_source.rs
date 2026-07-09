//! Unit **P5** — `context:property-placeholder` / `util:properties`
//! (SB-11). Test design per the internal build plan's P5 row: a table
//! covering multiple `location` values and `classpath:`-prefixed paths,
//! plus the spec's named edge case — a bean-declared
//! `PropertyPlaceholderConfigurer` stays on the plain `Bean` path (no
//! special-casing).
//!
//! `dispatch::parse_property_source` is `pub(crate)` — not visible from
//! this external integration-test binary — so every test here goes through
//! the public API (`beans_xml::parse`) only, the same convention
//! `tests/p3_import.rs`/`tests/p4_component_scan.rs` established.

use beans_xml::PropertySource;

const CONTEXT_NS: &str = r#"xmlns:context="http://www.springframework.org/schema/context""#;
const UTIL_NS: &str = r#"xmlns:util="http://www.springframework.org/schema/util""#;

fn parse_ok(source: &str) -> beans_xml::BeansFile {
    beans_xml::parse(source).beans.expect("beans root")
}

/// Parses `source` (expected to declare exactly one top-level property
/// source element) and returns that one `PropertySource` (span dropped —
/// callers that care about span assert on the outer `Spanned` directly).
fn only_source(source: &str) -> PropertySource {
    let beans = parse_ok(source);
    assert_eq!(
        beans.property_sources.len(),
        1,
        "expected exactly one property source element"
    );
    beans.property_sources.into_iter().next().unwrap().value
}

// ---------------------------------------------------------------------
// context:property-placeholder -> PropertySource::Placeholder.
// ---------------------------------------------------------------------

#[test]
fn sb11_single_location_placeholder() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:property-placeholder location="classpath:app.properties"/></beans>"#
    );
    match only_source(&source) {
        PropertySource::Placeholder { locations } => {
            assert_eq!(locations.len(), 1);
            assert_eq!(locations[0].value, "classpath:app.properties");
        }
        other => panic!("expected Placeholder, got {other:?}"),
    }
}

#[test]
fn sb11_multiple_locations_comma_separated() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:property-placeholder location="classpath:a.properties,classpath:b.properties"/></beans>"#
    );
    match only_source(&source) {
        PropertySource::Placeholder { locations } => {
            assert_eq!(locations.len(), 2);
            assert_eq!(locations[0].value, "classpath:a.properties");
            assert_eq!(locations[1].value, "classpath:b.properties");
        }
        other => panic!("expected Placeholder, got {other:?}"),
    }
}

#[test]
fn sb11_multiple_locations_with_surrounding_whitespace_trimmed() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:property-placeholder location="classpath:a.properties, classpath:b.properties , classpath:c.properties"/></beans>"#
    );
    match only_source(&source) {
        PropertySource::Placeholder { locations } => {
            assert_eq!(locations.len(), 3);
            assert_eq!(locations[0].value, "classpath:a.properties");
            assert_eq!(locations[1].value, "classpath:b.properties");
            assert_eq!(locations[2].value, "classpath:c.properties");
        }
        other => panic!("expected Placeholder, got {other:?}"),
    }
}

#[test]
fn sb11_location_span_slices_to_its_own_token() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:property-placeholder location="classpath:a.properties,classpath:b.properties"/></beans>"#
    );
    match only_source(&source) {
        PropertySource::Placeholder { locations } => {
            let second = &locations[1];
            let slice = &source[second.span.start as usize..second.span.end as usize];
            assert_eq!(slice, "classpath:b.properties");
        }
        other => panic!("expected Placeholder, got {other:?}"),
    }
}

#[test]
fn sb11_missing_location_attr_is_empty_placeholder() {
    let source = format!(r#"<beans {CONTEXT_NS}><context:property-placeholder/></beans>"#);
    match only_source(&source) {
        PropertySource::Placeholder { locations } => {
            assert!(locations.is_empty());
        }
        other => panic!("expected Placeholder, got {other:?}"),
    }
}

#[test]
fn sb11_empty_location_attr_is_empty_placeholder() {
    let source =
        format!(r#"<beans {CONTEXT_NS}><context:property-placeholder location=""/></beans>"#);
    match only_source(&source) {
        PropertySource::Placeholder { locations } => {
            assert!(locations.is_empty());
        }
        other => panic!("expected Placeholder, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// util:properties -> PropertySource::Properties.
// ---------------------------------------------------------------------

#[test]
fn sb11_util_properties_with_id_and_location() {
    let source = format!(
        r#"<beans {UTIL_NS}><util:properties id="appProps" location="classpath:app.properties"/></beans>"#
    );
    match only_source(&source) {
        PropertySource::Properties { id, location } => {
            assert_eq!(id.map(|s| s.value), Some("appProps".to_string()));
            assert_eq!(
                location.map(|s| s.value),
                Some("classpath:app.properties".to_string())
            );
        }
        other => panic!("expected Properties, got {other:?}"),
    }
}

#[test]
fn sb11_util_properties_id_is_a_valid_ref_target_shape() {
    // spec: "id present -> valid ref= target" -- this unit only needs to
    // capture the id itself; resolving a ref= against it is a consumer's
    // job (same "raw capture, no resolution" policy as every BeanRef).
    let source = format!(
        r#"<beans {UTIL_NS}><util:properties id="dbProps" location="classpath:db.properties"/></beans>"#
    );
    match only_source(&source) {
        PropertySource::Properties { id, .. } => {
            assert_eq!(id.map(|s| s.value), Some("dbProps".to_string()));
        }
        other => panic!("expected Properties, got {other:?}"),
    }
}

#[test]
fn sb11_util_properties_without_id_has_none_id() {
    let source = format!(
        r#"<beans {UTIL_NS}><util:properties location="classpath:anon.properties"/></beans>"#
    );
    match only_source(&source) {
        PropertySource::Properties { id, location } => {
            assert_eq!(id, None);
            assert_eq!(
                location.map(|s| s.value),
                Some("classpath:anon.properties".to_string())
            );
        }
        other => panic!("expected Properties, got {other:?}"),
    }
}

#[test]
fn sb11_util_properties_without_location_has_none_location() {
    let source = format!(r#"<beans {UTIL_NS}><util:properties id="emptyProps"/></beans>"#);
    match only_source(&source) {
        PropertySource::Properties { id, location } => {
            assert_eq!(id.map(|s| s.value), Some("emptyProps".to_string()));
            assert_eq!(location, None);
        }
        other => panic!("expected Properties, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Spec-named edge case: declarative PropertyPlaceholderConfigurer stays a
// plain Bean, never special-cased into a PropertySource.
// ---------------------------------------------------------------------

#[test]
fn sb11_bean_declared_property_placeholder_configurer_is_a_plain_bean() {
    let source = concat!(
        "<beans>",
        r#"<bean id="propertyConfigurer" class="org.springframework.beans.factory.config.PropertyPlaceholderConfigurer">"#,
        r#"<property name="location" value="classpath:com/example/app.properties"/>"#,
        "</bean>",
        "</beans>",
    );
    let beans = parse_ok(source);
    assert!(
        beans.property_sources.is_empty(),
        "no NS-based property source element in this document -- must not synthesize one from the bean"
    );
    assert_eq!(beans.beans.len(), 1);
    let bean = &beans.beans[0];
    assert_eq!(
        bean.id.as_ref().map(|s| s.value.as_str()),
        Some("propertyConfigurer")
    );
    assert_eq!(
        bean.class.as_ref().map(|c| c.value.raw.as_str()),
        Some("org.springframework.beans.factory.config.PropertyPlaceholderConfigurer")
    );
    assert_eq!(bean.properties.len(), 1);
}

// ---------------------------------------------------------------------
// Span sanity + multiple property source elements.
// ---------------------------------------------------------------------

#[test]
fn sb11_placeholder_and_properties_both_collected_in_order() {
    let source = format!(
        concat!(
            "<beans {CONTEXT_NS} {UTIL_NS}>",
            r#"<context:property-placeholder location="classpath:a.properties"/>"#,
            r#"<util:properties id="p" location="classpath:b.properties"/>"#,
            "</beans>",
        ),
        CONTEXT_NS = CONTEXT_NS,
        UTIL_NS = UTIL_NS,
    );
    let beans = parse_ok(&source);
    assert_eq!(beans.property_sources.len(), 2);
    assert!(matches!(
        beans.property_sources[0].value,
        PropertySource::Placeholder { .. }
    ));
    assert!(matches!(
        beans.property_sources[1].value,
        PropertySource::Properties { .. }
    ));
}

#[test]
fn sb11_property_source_span_covers_the_element() {
    let source = format!(
        r#"<beans {CONTEXT_NS}><context:property-placeholder location="classpath:a.properties"/></beans>"#
    );
    let beans = parse_ok(&source);
    let spanned = &beans.property_sources[0];
    let slice = &source[spanned.span.start as usize..spanned.span.end as usize];
    assert_eq!(
        slice,
        r#"<context:property-placeholder location="classpath:a.properties"/>"#
    );
}
