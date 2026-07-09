//! `beans-xml` — a lenient parser for **Spring Framework bean XML configuration**
//! (the `<beans>` schema): bean definitions, dependency wiring, `<import>`,
//! component-scan, profiles. **Spring beans — not EJB / JavaBeans / CDI.**
//!
//! Design: the internal design spec (maintained privately, not in this repo).
//! Build order + dispatch contract: the internal build plan (same). This file
//! is the **frozen public API surface**; units fill the parser behind it.
//!
//! Contract (batis-xml sibling): `parse`/`parse_bytes` never return `Err` — every
//! anomaly becomes a [`Diagnostic`]. Spans are **decoded-UTF-8** offsets.

#![forbid(unsafe_code)]

pub mod model;
pub use model::*;

/// Unit U1 — event/recovery layer (quick-xml event wrapper with spans,
/// tree-builder skeleton, recovery rules). `pub(crate)` seam: not part of
/// the published API surface — `parse`/`parse_bytes`/`is_beans_doc` below
/// (U3's job, per the build plan) are the public entry points wired
/// behind it. Only this module's own `#[cfg(test)]` mod tests it directly
/// (a seam not visible from an external integration-test binary); the
/// public functions below exercise it through the real pipeline.
mod events;

/// Unit U2 — encoding detection (BOM/UTF-8/declared-label/EUC-KR/lossy
/// decode chain, WHATWG label output). `pub(crate)` seam: same situation
/// as `events` above — `parse_bytes` below (U3's job) wires this module's
/// `decode` together with `events::build_tree` into the real pipeline.
mod encoding;

/// Unit U3 — root detection, `BeansFile` header (SB-01), and the frozen
/// root-child dispatch skeleton every leaf unit (P1/P3/P4/P5/P7/P10)
/// fills a handler function of. `pub(crate)` seam — see that module's own
/// doc comment for the full contract; `parse`/`parse_bytes`/`is_beans_doc`
/// below are its public entry points.
mod dispatch;

/// Unit U4 — `<bean>` core attributes (SB-02) and the frozen bean-child
/// dispatch skeleton every leaf unit (P2/P6/P8) fills a handler function
/// of. `pub(crate)` seam — see that module's own doc comment for the full
/// contract; wired behind `dispatch::dispatch_root_child`'s `"bean"` arm.
mod bean;

/// Unit U5a — `InjectValue` core (SB-06): value/ref/idref/inner/null (no
/// collections — see `collection` (U5b) directly below for those).
/// `pub(crate)` seam — not itself called from `parse`/`parse_bytes`
/// directly; U6/U7 (`<property>`/`<constructor-arg>`) wire it up. See that
/// module's own doc comment for the full contract.
mod inject_value;

/// Unit U5b — collections (SB-07): `<list>`/`<set>`/`<array>`/`<map>`/
/// `<props>` -> `Collection`, wired directly into
/// `inject_value::parse_inject_value_child`'s own match arm (a serial
/// continuation of U5a on the same spine, not a parallel leaf — see that
/// module's own doc comment). `pub(crate)` seam — see this module's own
/// doc comment for the full contract (item/key/value reuse of U5a,
/// self-recursion for nested collections, `DEPTH_LIMIT` bookkeeping).
mod collection;

/// Unit U6 — `<property>` (SB-04): wraps `InjectValue` (U5a) with `name` +
/// `<meta>` into a `Property`, wired behind `bean::dispatch_bean_child`'s
/// `"property"` arm. `pub(crate)` seam — see that module's own doc comment
/// for the full contract (value=/ref= precedence, `ConflictingValueAndRef`).
mod property;

/// Unit U7 — `<constructor-arg>` (SB-05): wraps `InjectValue` (U5a) with
/// `index`/`type`/`name` + `<meta>` into a `ConstructorArg`, wired behind
/// `bean::dispatch_bean_child`'s `"constructor-arg"` arm. `pub(crate)` seam —
/// see that module's own doc comment for the full contract (symmetric with
/// U6's `property`, plus index/type resolution).
mod constructor_arg;

/// Unit P7 — `NamespacedElement` + allowlisted ref harvest (SB-02c): the
/// shared builder behind both `dispatch::parse_namespaced` (root-child
/// catch-all) and `bean::parse_decorator` (bean-child catch-all), plus the
/// frozen `NS_REF_ALLOWLIST` table. `pub(crate)` seam — see that module's
/// own doc comment for the full contract.
mod namespaced;

/// Maximum input size; larger input is rejected before decoding.
pub const MAX_INPUT_BYTES: usize = 10 * 1024 * 1024;
/// Maximum nesting depth for beans / collections.
pub const DEPTH_LIMIT: u32 = 256;

/// Parse a decoded `<beans>` XML string. Never returns `Err`.
///
/// Always reports `encoding: Some("UTF-8")` — the one encoding a Rust
/// `&str` can ever be (see `ParseResult::encoding`'s own doc comment).
pub fn parse(source: &str) -> ParseResult {
    let tree = events::build_tree(source);
    let mut diagnostics = tree.diagnostics;
    let beans = parse_root(tree.root.as_ref(), &mut diagnostics);
    ParseResult {
        beans,
        encoding: Some("UTF-8".to_string()),
        diagnostics,
    }
}

/// Parse raw bytes (encoding detected). Never returns `Err`.
pub fn parse_bytes(bytes: &[u8]) -> ParseResult {
    if bytes.len() > MAX_INPUT_BYTES {
        return ParseResult {
            beans: None,
            encoding: None,
            diagnostics: vec![Diagnostic {
                code: DiagCode::OversizeInput,
                span: None,
                message: "input exceeds MAX_INPUT_BYTES".to_string(),
            }],
        };
    }
    let (source, mut diagnostics, encoding) = encoding::decode(bytes);
    let tree = events::build_tree(&source);
    diagnostics.extend(tree.diagnostics);
    let beans = parse_root(tree.root.as_ref(), &mut diagnostics);
    ParseResult {
        beans,
        encoding: Some(encoding.to_string()),
        diagnostics,
    }
}

/// Cheap pre-check: is the document root `<beans>`? Applies the same
/// `MAX_INPUT_BYTES` cap before decoding, and shares `dispatch::is_beans_root`
/// (a private helper — not part of the public API surface) with the real
/// parse pipeline (both `parse_root` below and this function call the
/// exact same check) so invariant #7 — `is_beans_doc(b) ==
/// parse_bytes(b).beans.is_some()` — holds by construction. Deliberately
/// stops at the raw event tree (no header/dispatch work) rather than
/// calling `parse_bytes` itself, so it stays cheap even once later units
/// make full parsing progressively heavier.
pub fn is_beans_doc(bytes: &[u8]) -> bool {
    if bytes.len() > MAX_INPUT_BYTES {
        return false;
    }
    let (source, _diagnostics, _encoding) = encoding::decode(bytes);
    let tree = events::build_tree(&source);
    tree.root.as_ref().is_some_and(dispatch::is_beans_root)
}

/// Crate version string.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Shared by `parse`/`parse_bytes`: resolves the tree's root element (if
/// any) into a `BeansFile`, or `None` plus a `NotBeansRoot` diagnostic —
/// whether because no root element was found at all, or because the root
/// found isn't `<beans>` (see [`dispatch::is_beans_root`]).
fn parse_root(
    root: Option<&events::XmlElement>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<BeansFile> {
    match root {
        None => {
            diagnostics.push(Diagnostic {
                code: DiagCode::NotBeansRoot,
                span: None,
                message: "no root element found".to_string(),
            });
            None
        }
        Some(root) if !dispatch::is_beans_root(root) => {
            diagnostics.push(Diagnostic {
                code: DiagCode::NotBeansRoot,
                span: Some(root.span),
                message: format!("document root <{}> is not <beans>", root.name),
            });
            None
        }
        Some(root) => {
            let scope = dispatch::NsScope::from_element(root, None);
            Some(dispatch::parse_beans_body(&scope, root, diagnostics, 0).into_beans_file())
        }
    }
}
