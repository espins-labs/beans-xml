//! Output model — the serde serialization of these types IS the published
//! schema (`schema/beans-xml.v1.json`, pinned by a snapshot test/example).
//!
//! Spec: the internal design spec's "Public API surface" section. Build order +
//! the dispatch contract that keeps parallel leaf units conflict-free: the
//! internal build plan. This file is **unit U0** — the
//! frozen data contract every other unit builds behind. Per this crate's
//! `AGENTS.md`: do not change a field or enum variant's name/shape without
//! calling it out explicitly; adding a new `DiagCode` variant is the one
//! pre-approved additive exception.
//!
//! Sibling crate `batis-xml` uses the same shape of contract (`Err`-free
//! parser, `Diagnostic`-only anomalies, `Spanned`/`ByteSpan`) — see that
//! crate's `model.rs` for the shared vocabulary this one reuses. One
//! deliberate difference: **`ByteSpan` here is decoded-UTF-8 offsets**
//! (see its own doc comment) — batis-xml's are raw input bytes. This was a
//! corrected contract during beans-xml's own design cold review (a stale
//! "raw bytes" draft would have been wrong for EUC-KR/CP949 input, where
//! decoding changes character byte-widths).

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------
// ByteSpan / Spanned — the span vocabulary every located value shares.
// ---------------------------------------------------------------------

/// Half-open `[start, end)` offset into the **decoded UTF-8** text this
/// crate produced — *not* the caller's original raw input bytes.
///
/// For UTF-8 input this is identical to raw byte offsets (decoding is a
/// no-op). For any other source encoding (EUC-KR, CP949, UTF-16, ...; see
/// [`ParseResult::encoding`]) decoding changes character byte-widths, so a
/// span here is only valid against the **re-encoded UTF-8** text, not the
/// original file's bytes. To slice the original bytes, decode them the
/// same way this crate did (using the `encoding` label as the anchor) and
/// slice the redecoded string, not the raw input.
///
/// A leading byte-order mark is never part of this text at either entry
/// point — `parse_bytes` consumes it during decoding, and `parse` treats
/// its `&str` input as already BOM-free (a caller handing in a string that
/// still carries one is on its own for that byte, same as batis-xml).
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteSpan {
    pub start: u32,
    pub end: u32,
}

/// A value plus the span of the source text it was read from.
///
/// Used for every field whose owning struct/enum has no `span` field of
/// its own. Types that *do* carry an own `span: ByteSpan` field (`Bean`,
/// `ValueLit`, `MapEntry`, `PropEntry`, `NamespacedElement`, `Property`,
/// `ConstructorArg`, `Qualifier`, ...) are never additionally wrapped in
/// `Spanned<T>` at their use sites — invariant #2 (see the spec) calls
/// this "self-span": exactly one span per located node, never two.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spanned<T> {
    pub value: T,
    pub span: ByteSpan,
}

// ---------------------------------------------------------------------
// Top-level parse result.
// ---------------------------------------------------------------------

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParseResult {
    /// `None` if the document root is not `<beans>` (see
    /// `DiagCode::NotBeansRoot`). Invariant #7: `is_beans_doc(b) ==
    /// parse_bytes(b).beans.is_some()`.
    pub beans: Option<BeansFile>,
    /// Detected encoding as a WHATWG label (`encoding_rs`'s own values,
    /// e.g. `"UTF-8"`, `"EUC-KR"`). `None` only when no decode was
    /// attempted at all — input rejected by the raw-byte `MAX_INPUT_BYTES`
    /// cap before `parse_bytes` ever decodes it. `parse` (already-decoded
    /// `&str` input) always reports `"UTF-8"`, the one encoding a Rust
    /// `&str` can ever be.
    pub encoding: Option<String>,
    /// Parsing never fails — every anomaly accumulates here instead.
    pub diagnostics: Vec<Diagnostic>,
}

// ---------------------------------------------------------------------
// <beans> document.
// ---------------------------------------------------------------------

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeansFile {
    /// Opening `<beans>` tag start → subtree end.
    pub span: ByteSpan,
    /// `<beans profile="dev,test">` — raw comma-separated text, unsplit
    /// (splitting/negation (`!prod`) semantics are a consumer concern).
    pub profile: Option<Spanned<String>>,
    pub description: Option<Spanned<String>>,
    // default-* attributes: inherited by a bean that doesn't declare its
    // own value. For `default_init_method`/`default_destroy_method`,
    // `Some("")` (explicit empty string — suppresses inheritance) and
    // `None` (attribute absent) are kept distinct on purpose: this
    // preserves the original document verbatim; deriving the *effective*
    // lifecycle method per bean is a consumer's job, not this parser's.
    pub default_lazy_init: Option<bool>,
    pub default_autowire: Option<Spanned<String>>,
    pub default_init_method: Option<Spanned<String>>,
    pub default_destroy_method: Option<Spanned<String>>,
    pub default_merge: Option<bool>,
    pub default_autowire_candidates: Option<Spanned<String>>,
    pub imports: Vec<Spanned<Import>>,
    pub aliases: Vec<Spanned<Alias>>,
    /// Top-level `<bean>` elements only — an inner (anonymous) `<bean>`
    /// lives inside its owning `InjectValue::Inner`, not here.
    pub beans: Vec<Bean>,
    pub component_scans: Vec<Spanned<ComponentScan>>,
    /// `context:property-placeholder` and `util:properties`.
    pub property_sources: Vec<Spanned<PropertySource>>,
    /// Non-first-class namespaces (`aop:*`, `tx:*`, `jee:*`, `util:list`,
    /// ...) preserved bare — see `NamespacedElement`'s own doc comment for
    /// the v0.1 namespace policy.
    pub namespaced: Vec<NamespacedElement>,
    /// Nested `<beans profile="...">` blocks (Spring 3.1+). Recursive —
    /// parsed via the same root-child dispatch as the top-level document
    /// (build plan: "recursion unification", `parse_beans_body` is re-entered, never
    /// reimplemented), so every field above applies equally inside here.
    pub nested_profiles: Vec<BeansFile>,
}

// ---------------------------------------------------------------------
// <bean> definition.
// ---------------------------------------------------------------------

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bean {
    /// Opening tag start → subtree end (or the element's own extent for a
    /// self-closed `<bean/>`).
    pub span: ByteSpan,
    pub id: Option<Spanned<String>>,
    /// `name="a, b c"` alias tokens (comma/semicolon/whitespace-separated)
    /// — **`id` is never duplicated into this list**. The effective
    /// registered name is `id`, or (when absent) the first token here; the
    /// union of `id` and every name here is what a `BeanRef.raw` may
    /// match against.
    pub names: Vec<Spanned<String>>,
    /// Absent when the class is inherited from `parent`/`factory-bean`.
    pub class: Option<Spanned<ClassRef>>,
    /// Bean-to-bean inheritance edge (`RefKind::Bean`) — distinct from a
    /// `<ref parent="...">` inject value, which targets a *parent
    /// container*, not a parent bean definition (see `RefKind`).
    pub parent: Option<Spanned<BeanRef>>,
    /// Legacy DTD `singleton="true"/"false"` is normalized into this same
    /// field (as `"singleton"`/`"prototype"`), not kept as a separate flag.
    pub scope: Option<Spanned<String>>,
    #[serde(rename = "abstract")]
    pub abstract_: bool,
    pub lazy_init: Option<bool>,
    pub primary: bool,
    pub autowire: Option<Spanned<String>>,
    pub autowire_candidate: Option<bool>,
    /// Edges (`RefKind::Bean`).
    pub depends_on: Vec<Spanned<BeanRef>>,
    pub factory_bean: Option<Spanned<BeanRef>>,
    pub factory_method: Option<Spanned<String>>,
    pub init_method: Option<Spanned<String>>,
    pub destroy_method: Option<Spanned<String>>,
    /// `<property>` children plus `p:`-namespace attributes normalized
    /// into the same shape.
    pub properties: Vec<Property>,
    /// `<constructor-arg>` children plus `c:`-namespace attributes
    /// normalized into the same shape.
    pub constructor_args: Vec<ConstructorArg>,
    /// Method injection — each is itself a bean-to-bean edge.
    pub lookup_methods: Vec<Spanned<LookupMethod>>,
    pub replaced_methods: Vec<Spanned<ReplacedMethod>>,
    /// `<qualifier type= value=>` plus its nested `<attribute>` children.
    pub qualifiers: Vec<Qualifier>,
    /// Namespaced children directly inside `<bean>` (e.g.
    /// `<aop:scoped-proxy/>`), preserved bare — same policy as
    /// `BeansFile::namespaced`.
    pub decorators: Vec<NamespacedElement>,
    pub description: Option<Spanned<String>>,
    pub meta: Vec<MetaEntry>,
}

// ---------------------------------------------------------------------
// Property / constructor-arg wiring.
// ---------------------------------------------------------------------

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Property {
    /// `<property>` element extent, or (for a `p:`-namespace attribute)
    /// the attribute's own span.
    pub span: ByteSpan,
    pub name: Spanned<String>,
    pub value: InjectValue,
    pub meta: Vec<MetaEntry>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstructorArg {
    /// `<constructor-arg>` element extent, or (for a `c:`-namespace
    /// attribute) the attribute's own span.
    pub span: ByteSpan,
    pub index: Option<u32>,
    pub type_ref: Option<Spanned<ClassRef>>,
    pub name: Option<Spanned<String>>,
    pub value: InjectValue,
    pub meta: Vec<MetaEntry>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LookupMethod {
    pub name: Option<Spanned<String>>,
    /// `None` when `bean=` is absent — recorded as a `RefWithoutTarget`
    /// diagnostic, with the element itself still preserved (no edge is
    /// emitted for it).
    pub bean: Option<Spanned<BeanRef>>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplacedMethod {
    pub name: Spanned<String>,
    pub replacer: Option<Spanned<BeanRef>>,
    /// `<arg-type match="...">` values, or (arg-type text-content ruling:
    /// Spring also accepts this form) the trimmed text-content of an
    /// `<arg-type>` that has no non-whitespace `match=` — `match=` wins
    /// only when it has non-whitespace text, same as real Spring's own
    /// `StringUtils.hasText` precedence (a present-but-empty or
    /// whitespace-only `match=` falls back to the text body instead of
    /// being recorded as-is). Either way this is a type-*match* pattern,
    /// not a fully-qualified class name, so these are plain strings rather
    /// than `ClassRef` (see `ClassRef`'s own doc comment on that
    /// exclusion).
    pub arg_types: Vec<Spanned<String>>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Qualifier {
    pub span: ByteSpan,
    #[serde(rename = "type")]
    pub type_: Option<Spanned<ClassRef>>,
    pub value: Option<Spanned<String>>,
    /// Nested `<attribute key= value=>` children.
    pub attributes: Vec<AttrPair>,
}

// ---------------------------------------------------------------------
// Injected values — the recursive core (build plan U5a/U5b).
// ---------------------------------------------------------------------

/// The value injected into a `<property>`, `<constructor-arg>`, collection
/// item, or map entry key/value.
///
/// `#[non_exhaustive]` + adjacently tagged (`{"kind": ..., "content":
/// ...}`): a future beans-xml version may add a variant (e.g. for a v0.2
/// first-class namespace promotion) without breaking a consumer parsing
/// *this* version's JSON — an unrecognized `kind` string deserializes to
/// `Unrecognized` instead of failing the whole document. `Serialize` is
/// still derived (it produces exactly this shape); `Deserialize` is
/// hand-rolled below — see that impl's doc comment for why derive's own
/// `#[serde(other)]` support doesn't actually give adjacently tagged
/// enums a working fallback once a real variant's `content` is involved.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "content", rename_all = "snake_case")]
#[non_exhaustive]
pub enum InjectValue {
    /// `ref="beanName"` attribute, or a `<ref bean=/local=/parent=>` child.
    Ref(Spanned<BeanRef>),
    /// `<idref bean=/local=>`.
    Idref(Spanned<BeanRef>),
    /// A literal — `value="..."` attribute or `<value>` child — including
    /// any `${}`/`#{}` expressions found inside it (collected, not
    /// evaluated; see `ValueLit`).
    Value(ValueLit),
    /// An inline anonymous `<bean>`.
    Inner(Box<Bean>),
    Collection(Spanned<Collection>),
    /// `<null/>`.
    Null(ByteSpan),
    /// Forward-compat deserialization fallback — never produced by this
    /// crate's own parser. Excluded from the published JSON Schema (not a
    /// shape this crate's own output can ever contain — see
    /// `Collection::Unrecognized`'s matching doc comment for the same
    /// `schemars(skip)` rationale, reused here).
    #[doc(hidden)]
    #[cfg_attr(feature = "schema", schemars(skip))]
    Unrecognized,
}

/// Manual `Deserialize` for `InjectValue` — required because derived
/// `#[serde(other)]` support for adjacently tagged enums turns out to only
/// cover the case where the fallback's `content` is *absent or null*: a
/// payload from a genuinely new future variant (which will always carry a
/// real `content` value, since every existing variant does) hard-errors
/// derive's generated code instead of falling back to `Unrecognized` —
/// confirmed directly against this crate's own `serde` version, and
/// exercised by this crate's own test suite
/// (`sb_u0_inject_value_unknown_kind_deserializes_to_unrecognized_and_stays_stable`).
/// This is the adjacently tagged counterpart of the problem batis-xml's
/// `SqlText` solves for externally tagged enums (see that type's own
/// `Deserialize` doc comment) — different tagging shape, same underlying
/// need: a forward-compat fallback that actually works once real payload
/// data is involved, not just for a bare unit-variant tag.
///
/// Buffers `content` as a `serde_json::Value` before dispatching on
/// `kind` — `serde_json` is already a mandatory dependency of this crate
/// (its WASM boundary's own output is a JSON string, per the spec), and
/// `Value`'s own `Deserialize` impl works against any self-describing
/// `serde` `Deserializer`, not just `serde_json`'s own. Once `kind` is
/// unrecognized, the buffered `content` is simply discarded rather than
/// being fed to a variant that has nowhere to put it.
impl<'de> Deserialize<'de> for InjectValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            kind: String,
            #[serde(default)]
            content: serde_json::Value,
        }
        let Repr { kind, content } = Repr::deserialize(deserializer)?;
        Ok(match kind.as_str() {
            "ref" => {
                InjectValue::Ref(serde_json::from_value(content).map_err(serde::de::Error::custom)?)
            }
            "idref" => InjectValue::Idref(
                serde_json::from_value(content).map_err(serde::de::Error::custom)?,
            ),
            "value" => InjectValue::Value(
                serde_json::from_value(content).map_err(serde::de::Error::custom)?,
            ),
            "inner" => InjectValue::Inner(
                serde_json::from_value(content).map_err(serde::de::Error::custom)?,
            ),
            "collection" => InjectValue::Collection(
                serde_json::from_value(content).map_err(serde::de::Error::custom)?,
            ),
            "null" => InjectValue::Null(
                serde_json::from_value(content).map_err(serde::de::Error::custom)?,
            ),
            // Forward-compat: any other `kind` string (a future variant
            // this build doesn't know about yet) absorbs into
            // `Unrecognized`, discarding the buffered `content` rather
            // than failing the whole document.
            _ => InjectValue::Unrecognized,
        })
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueLit {
    pub span: ByteSpan,
    pub text: Spanned<String>,
    /// `<value type="java.lang.Integer">`.
    pub value_type: Option<Spanned<ClassRef>>,
    /// `${prop.key}` placeholder keys found inside `text` (collected, not
    /// evaluated).
    pub placeholders: Vec<Spanned<String>>,
    /// `#{beanName...}` SpEL bean-reference *candidates* found inside
    /// `text` — the single collection path for SpEL bean references (see
    /// the spec's "references are raw only" decision); not a full SpEL evaluator.
    pub spel_refs: Vec<Spanned<String>>,
}

/// `#[non_exhaustive]` + adjacently tagged: same forward-compat rationale
/// as `InjectValue` (its sibling in the spec's "enum policy"), so a future
/// collection kind never breaks a consumer reading this version's JSON.
/// `Serialize` is derived; `Deserialize` is hand-rolled below (see
/// `InjectValue`'s manual `Deserialize` impl for why).
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "content", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Collection {
    List {
        items: Vec<InjectValue>,
        value_type: Option<Spanned<ClassRef>>,
        merge: Option<bool>,
    },
    Set {
        items: Vec<InjectValue>,
        value_type: Option<Spanned<ClassRef>>,
        merge: Option<bool>,
    },
    Array {
        items: Vec<InjectValue>,
        value_type: Option<Spanned<ClassRef>>,
        merge: Option<bool>,
    },
    Map {
        entries: Vec<MapEntry>,
        /// `<map key-type="...">` — the per-entry `value-type` (when
        /// present on an individual `<entry>`) lands on `MapEntry` instead.
        key_type: Option<Spanned<ClassRef>>,
        value_type: Option<Spanned<ClassRef>>,
        merge: Option<bool>,
    },
    Props {
        entries: Vec<PropEntry>,
        merge: Option<bool>,
    },
    /// Forward-compat deserialization fallback — never produced by this
    /// crate's own parser. Excluded from the published JSON Schema
    /// (`schemars(skip)`): not a shape this crate's own output can ever
    /// contain, so it has no business in the *published* schema (same
    /// reasoning batis-xml's `SqlText::Unrecognized` documents).
    #[doc(hidden)]
    #[cfg_attr(feature = "schema", schemars(skip))]
    Unrecognized,
}

/// Manual `Deserialize` for `Collection` — see `InjectValue`'s own manual
/// `Deserialize` impl for the full rationale (identical problem: derived
/// `#[serde(other)]` on an adjacently tagged enum doesn't actually give a
/// working fallback once a real variant's `content` is involved). Each
/// struct-like variant's `content` is buffered as `serde_json::Value` and
/// then deserialized into a local field-matching helper struct — `List`/
/// `Set`/`Array` share one shape (`ListLike`) since their fields are
/// identical.
impl<'de> Deserialize<'de> for Collection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            kind: String,
            #[serde(default)]
            content: serde_json::Value,
        }
        #[derive(Deserialize)]
        struct ListLike {
            items: Vec<InjectValue>,
            #[serde(default)]
            value_type: Option<Spanned<ClassRef>>,
            #[serde(default)]
            merge: Option<bool>,
        }
        #[derive(Deserialize)]
        struct MapContent {
            entries: Vec<MapEntry>,
            #[serde(default)]
            key_type: Option<Spanned<ClassRef>>,
            #[serde(default)]
            value_type: Option<Spanned<ClassRef>>,
            #[serde(default)]
            merge: Option<bool>,
        }
        #[derive(Deserialize)]
        struct PropsContent {
            entries: Vec<PropEntry>,
            #[serde(default)]
            merge: Option<bool>,
        }

        let Repr { kind, content } = Repr::deserialize(deserializer)?;
        Ok(match kind.as_str() {
            "list" => {
                let c: ListLike =
                    serde_json::from_value(content).map_err(serde::de::Error::custom)?;
                Collection::List {
                    items: c.items,
                    value_type: c.value_type,
                    merge: c.merge,
                }
            }
            "set" => {
                let c: ListLike =
                    serde_json::from_value(content).map_err(serde::de::Error::custom)?;
                Collection::Set {
                    items: c.items,
                    value_type: c.value_type,
                    merge: c.merge,
                }
            }
            "array" => {
                let c: ListLike =
                    serde_json::from_value(content).map_err(serde::de::Error::custom)?;
                Collection::Array {
                    items: c.items,
                    value_type: c.value_type,
                    merge: c.merge,
                }
            }
            "map" => {
                let c: MapContent =
                    serde_json::from_value(content).map_err(serde::de::Error::custom)?;
                Collection::Map {
                    entries: c.entries,
                    key_type: c.key_type,
                    value_type: c.value_type,
                    merge: c.merge,
                }
            }
            "props" => {
                let c: PropsContent =
                    serde_json::from_value(content).map_err(serde::de::Error::custom)?;
                Collection::Props {
                    entries: c.entries,
                    merge: c.merge,
                }
            }
            // Forward-compat: any other `kind` string absorbs into
            // `Unrecognized`, discarding the buffered `content`.
            _ => Collection::Unrecognized,
        })
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapEntry {
    /// `<entry>` element extent.
    pub span: ByteSpan,
    /// From `key="..."`/`key-ref="..."` attribute or a `<key>` child.
    pub key: InjectValue,
    /// From `value="..."`/`value-ref="..."` attribute or a value child.
    pub value: InjectValue,
    /// Per-entry `<entry value-type="...">` — the map-wide `key-type`
    /// lives on `Collection::Map` instead.
    pub value_type: Option<Spanned<ClassRef>>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropEntry {
    /// `<prop key="...">` element extent.
    pub span: ByteSpan,
    pub key: Spanned<String>,
    pub value: ValueLit,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaEntry {
    pub key: Spanned<String>,
    pub value: Spanned<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttrPair {
    pub key: Spanned<String>,
    pub value: Spanned<String>,
}

// ---------------------------------------------------------------------
// References.
// ---------------------------------------------------------------------

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeanRef {
    /// The target name verbatim, exactly as written — resolution (which
    /// bean this actually points at, whether it exists at all) is always
    /// the consumer's job; this parser has no cross-file view. Invariant
    /// #5: never empty (an empty attribute value is reported as a
    /// diagnostic instead of producing an empty `raw`).
    pub raw: String,
    pub kind: RefKind,
}

/// Closed set — an exhaustive `match` is a consumer feature, unlike
/// `InjectValue`/`Collection`/`DiagCode`. There is no third kind of
/// bean-reference target; a genuinely new one would be a v2 concern.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    /// `ref=`/`bean=` attribute, or `parent=`/`depends-on=`/`factory-bean=`
    /// — all resolve within the *same* bean container.
    Bean,
    /// `<ref local="x">` — legacy same-file-only reference.
    Local,
    /// `<ref parent="x">` — targets a *parent container* (an ancestor
    /// `BeanFactory`/`ApplicationContext`), unrelated to a bean
    /// definition's own `parent=` inheritance attribute (which is
    /// `RefKind::Bean`).
    ParentContainer,
}

// ---------------------------------------------------------------------
// Alias / import / component-scan / property-source.
// ---------------------------------------------------------------------

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alias {
    pub name: Spanned<String>,
    pub alias: Spanned<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Import {
    pub resource: Spanned<String>,
    pub kind: ImportKind,
}

/// Closed set — see `RefKind`'s doc comment for the same rationale.
/// `Url` covers every URL scheme (including `file:`); `Other` is the
/// total fallback for a resource string matching none of the recognized
/// shapes (not a forward-compat mechanism, unlike `DiagCode::Other`).
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportKind {
    /// `classpath:...`.
    Classpath,
    /// `classpath*:...`.
    ClasspathStar,
    /// A bare relative path.
    Relative,
    Url,
    Other,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentScan {
    pub base_packages: Vec<Spanned<String>>,
    pub use_default_filters: Option<bool>,
    pub include_filters: Vec<ScanFilter>,
    pub exclude_filters: Vec<ScanFilter>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanFilter {
    pub filter_type: Spanned<String>,
    /// The filter pattern/expression — kept as a plain string (not
    /// `ClassRef`) since it may be an annotation FQN, a regex, or an
    /// AspectJ pointcut depending on `filter_type`, not always a class
    /// name (see `ClassRef`'s exclusion list).
    pub expression: Spanned<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertySource {
    /// `context:property-placeholder`.
    Placeholder { locations: Vec<Spanned<String>> },
    /// `util:properties` — `id` (when present) registers a bean, so it is
    /// a valid `ref=` target the same way any other bean id is.
    Properties {
        id: Option<Spanned<String>>,
        location: Option<Spanned<String>>,
    },
}

// ---------------------------------------------------------------------
// Namespaced (non-first-class) elements.
// ---------------------------------------------------------------------

/// A preserved-but-not-deeply-parsed namespaced element — v0.1's policy
/// for every namespace outside the first-class set (`beans` in full,
/// `context:component-scan`/`context:property-placeholder`,
/// `util:properties`). Covers `util:constant`/`util:list`/..., `aop:*`,
/// `tx:*`, `jee:*`, `jms:*`, `lang:*`, `cache:*`, `task:*`, `mvc:*`, and
/// any `<bean>`-internal decorator (`aop:scoped-proxy`, ...).
///
/// `id` matters beyond bookkeeping: an id-bearing namespaced element
/// (`jee:jndi-lookup`, `util:list`, ...) registers a bean the same way a
/// `<bean id=...>` does — without it, a `ref="dataSource"` pointing at a
/// JNDI-backed datasource would otherwise look like a dangling reference.
/// `refs` is populated by recursing into the subtree and collecting only
/// the attributes on the fixed `NS_REF_ALLOWLIST` table (aop
/// `aspect@ref`/`advisor@advice-ref`, tx `advice@transaction-manager`,
/// task `scheduled@ref`/`@scheduler`, jee JNDI references, ...) — this is
/// a bean-to-bean edge collection policy, not a general-purpose attribute
/// walk, and it deliberately excludes some attributes (e.g. aop
/// `pointcut-ref` — a pointcut is not itself a bean).
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamespacedElement {
    /// The namespace URI (or prefix, if the URI can't be resolved from the
    /// document's own declarations) — raw, unresolved.
    pub ns: String,
    /// The local (unprefixed) element name.
    pub local: String,
    /// Element extent (opening tag start → subtree end).
    pub span: ByteSpan,
    pub id: Option<Spanned<String>>,
    pub attrs: Vec<AttrPair>,
    pub refs: Vec<Spanned<BeanRef>>,
}

// ---------------------------------------------------------------------
// Class references.
// ---------------------------------------------------------------------

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassRef {
    /// The fully-qualified class name verbatim — alias resolution and
    /// generic/array-suffix parsing are a consumer's job. Invariant #5:
    /// never empty. Deliberately **not** used for `scan-filter`
    /// expressions or `<arg-type match=...>` patterns, which are type
    /// *patterns*, not necessarily FQNs — those stay plain `String`.
    pub raw: String,
}

// ---------------------------------------------------------------------
// Diagnostics.
// ---------------------------------------------------------------------

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: DiagCode,
    pub span: Option<ByteSpan>,
    pub message: String,
}

/// `#[non_exhaustive]` + `#[serde(other)]`: additions are pre-approved
/// (see this crate's `AGENTS.md`) and never break a consumer holding an
/// older build's `match` (a wildcard arm is compiler-enforced) or reading
/// a newer version's JSON (an unrecognized code string lands on `Other`
/// instead of failing deserialization). Never produced by this crate's
/// own parser — `Other` exists purely as the deserialization side of this
/// contract.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DiagCode {
    EncodingUndetectable,
    EncodingMismatch,
    UnclosedTag,
    UnexpectedCloseTag,
    /// Recovery rule: first value wins, duplicate is reported here.
    DuplicateAttribute,
    /// Document root is not `<beans>`.
    NotBeansRoot,
    /// Scoped to a single `<beans>` block — a `<beans profile="a">` and a
    /// sibling `<beans profile="b">` declaring the same id is a profile
    /// override, not a duplicate.
    DuplicateBeanId,
    /// No `class`, `parent`, or `factory-bean` at all — except an
    /// `abstract="true"` template bean, which is a legitimate exception.
    BeanWithoutClassOrParent,
    /// Both `value=` and `ref=` (or their equivalents) present on the same
    /// injection point.
    ConflictingValueAndRef,
    /// A reference-shaped element/attribute with no target name (e.g.
    /// `<lookup-method>` with no `bean=`).
    RefWithoutTarget,
    /// An unrecognized element inside the first-class `beans` namespace
    /// itself (not the same as an out-of-scope *namespace*, which is
    /// preserved via `NamespacedElement` instead of flagged here).
    UnknownElement,
    /// An entity reference could not be resolved; the raw text is kept
    /// as-is for that segment.
    InvalidEntity,
    /// A `${`/`#{` placeholder never found its closing delimiter; the raw
    /// text is kept as-is.
    UnterminatedPlaceholder,
    /// Recursion depth exceeded [`crate::DEPTH_LIMIT`] nesting levels
    /// (nested beans/collections) — the remaining subtree is treated as
    /// opaque rather than risking a stack overflow.
    NestingLimitExceeded,
    /// Input exceeded [`crate::MAX_INPUT_BYTES`], reported before any
    /// decoding is attempted.
    OversizeInput,
    /// Forward-compat deserialization fallback. See this enum's own doc
    /// comment.
    #[serde(other)]
    Other,
}

// ---------------------------------------------------------------------
// Parser seam: shared context accumulators (build plan "dispatch contract").
//
// Not part of the published output schema (no Serialize/Deserialize,
// no JsonSchema) -- these are `pub(crate)` builder types the dispatch
// units (U3's root-child match, U4's `parse_bean`) and every leaf
// handler function share. The build plan's stated rationale: a handler
// pushes into a pre-declared `Vec` field on `&mut ctx` rather than
// building one giant struct literal at assembly time (an "assembly hot
// spot" that would force every leaf handler to touch the same
// construction site). Freezing the shape here -- before the leaf wave
// fans out -- is what lets leaf units (P1, P3, P4, P5, P7, P10 against
// `BeansFileCtx`; P2, P6, P8 against `BeanCtx`) fill in independent
// handler functions without colliding on a shared struct literal.
// ---------------------------------------------------------------------

/// Accumulator for one `<beans>` body (top-level document or a nested
/// `<beans profile="...">` block) while its children are dispatched.
/// `into_beans_file` performs the final, infallible move into `BeansFile`
/// once every child has been visited.
///
/// Wired up by U3's root-child dispatch (`src/dispatch.rs`'s
/// `parse_beans_body`).
#[derive(Debug, Default)]
pub(crate) struct BeansFileCtx {
    pub span: ByteSpan,
    pub profile: Option<Spanned<String>>,
    pub description: Option<Spanned<String>>,
    pub default_lazy_init: Option<bool>,
    pub default_autowire: Option<Spanned<String>>,
    pub default_init_method: Option<Spanned<String>>,
    pub default_destroy_method: Option<Spanned<String>>,
    pub default_merge: Option<bool>,
    pub default_autowire_candidates: Option<Spanned<String>>,
    pub imports: Vec<Spanned<Import>>,
    pub aliases: Vec<Spanned<Alias>>,
    pub beans: Vec<Bean>,
    pub component_scans: Vec<Spanned<ComponentScan>>,
    pub property_sources: Vec<Spanned<PropertySource>>,
    pub namespaced: Vec<NamespacedElement>,
    pub nested_profiles: Vec<BeansFile>,
}

impl BeansFileCtx {
    /// Final, infallible assembly — moves every accumulated field into a
    /// `BeansFile`. Never fails: by construction every field here has the
    /// exact shape `BeansFile` expects.
    pub(crate) fn into_beans_file(self) -> BeansFile {
        BeansFile {
            span: self.span,
            profile: self.profile,
            description: self.description,
            default_lazy_init: self.default_lazy_init,
            default_autowire: self.default_autowire,
            default_init_method: self.default_init_method,
            default_destroy_method: self.default_destroy_method,
            default_merge: self.default_merge,
            default_autowire_candidates: self.default_autowire_candidates,
            imports: self.imports,
            aliases: self.aliases,
            beans: self.beans,
            component_scans: self.component_scans,
            property_sources: self.property_sources,
            namespaced: self.namespaced,
            nested_profiles: self.nested_profiles,
        }
    }
}

/// Accumulator for one `<bean>` element (top-level or an `InjectValue::Inner`
/// anonymous bean — both go through the same shared `parse_bean`, build
/// plan "recursion unification") while its attributes/children are dispatched.
///
/// Wired up by U4's bean-child dispatch (`src/bean.rs`'s `parse_bean`).
#[derive(Debug, Default)]
pub(crate) struct BeanCtx {
    pub span: ByteSpan,
    pub id: Option<Spanned<String>>,
    pub names: Vec<Spanned<String>>,
    pub class: Option<Spanned<ClassRef>>,
    pub parent: Option<Spanned<BeanRef>>,
    pub scope: Option<Spanned<String>>,
    pub abstract_: bool,
    pub lazy_init: Option<bool>,
    pub primary: bool,
    pub autowire: Option<Spanned<String>>,
    pub autowire_candidate: Option<bool>,
    pub depends_on: Vec<Spanned<BeanRef>>,
    pub factory_bean: Option<Spanned<BeanRef>>,
    pub factory_method: Option<Spanned<String>>,
    pub init_method: Option<Spanned<String>>,
    pub destroy_method: Option<Spanned<String>>,
    pub properties: Vec<Property>,
    pub constructor_args: Vec<ConstructorArg>,
    pub lookup_methods: Vec<Spanned<LookupMethod>>,
    pub replaced_methods: Vec<Spanned<ReplacedMethod>>,
    pub qualifiers: Vec<Qualifier>,
    pub decorators: Vec<NamespacedElement>,
    pub description: Option<Spanned<String>>,
    pub meta: Vec<MetaEntry>,
}

impl BeanCtx {
    /// Final, infallible assembly — see `BeansFileCtx::into_beans_file`.
    pub(crate) fn into_bean(self) -> Bean {
        Bean {
            span: self.span,
            id: self.id,
            names: self.names,
            class: self.class,
            parent: self.parent,
            scope: self.scope,
            abstract_: self.abstract_,
            lazy_init: self.lazy_init,
            primary: self.primary,
            autowire: self.autowire,
            autowire_candidate: self.autowire_candidate,
            depends_on: self.depends_on,
            factory_bean: self.factory_bean,
            factory_method: self.factory_method,
            init_method: self.init_method,
            destroy_method: self.destroy_method,
            properties: self.properties,
            constructor_args: self.constructor_args,
            lookup_methods: self.lookup_methods,
            replaced_methods: self.replaced_methods,
            qualifiers: self.qualifiers,
            decorators: self.decorators,
            description: self.description,
            meta: self.meta,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These stay in-module (rather than the unit's own `tests/u0_model.rs`)
    // because they exercise `pub(crate)` seam types that aren't visible
    // from an external integration-test binary.

    #[test]
    fn ctx_seam_round_trips_a_minimal_bean_and_beans_file() {
        let mut bean_ctx = BeanCtx {
            span: ByteSpan { start: 0, end: 10 },
            ..Default::default()
        };
        bean_ctx.id = Some(Spanned {
            value: "myBean".to_string(),
            span: ByteSpan { start: 1, end: 7 },
        });
        let bean = bean_ctx.into_bean();
        assert_eq!(bean.id.as_ref().map(|s| s.value.as_str()), Some("myBean"));
        assert!(!bean.abstract_);

        let mut file_ctx = BeansFileCtx::default();
        file_ctx.beans.push(bean);
        let file = file_ctx.into_beans_file();
        assert_eq!(file.beans.len(), 1);
        assert_eq!(file.beans[0].id.as_ref().unwrap().value, "myBean");
    }
}
