//! WebAssembly bindings for `beans-xml`. Minimal surface, JSON-string
//! boundary: consumers get the whole output model (schema v1, see
//! `../schema/beans-xml.v1.json`) as a JSON string rather than a
//! marshalled JS object -- simplest, schema-faithful, no per-field glue
//! code to keep in sync as the model grows.
//!
//! Bytes-only: `parse` takes a `Uint8Array`/`Buffer` and runs the core
//! crate's `parse_bytes` (encoding detection included). The core crate's
//! string-based `parse(&str)` is deliberately **not** exposed here --
//! Rust-native callers use it directly; wasm callers always go through
//! bytes so encoding detection stays a single code path (same choice as
//! the batis-xml sibling crate).

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// A16 (batis-xml cold code review, ported): `&[u8]`'s wasm-bindgen
/// marshalling coerces *any* JS value into bytes rather than validating
/// it's really a `Uint8Array`/`Buffer` first -- a JS string would silently
/// become a zero-filled buffer (parsing it then fails with a misleading
/// "no root element found", not an error pointing at the real mistake), a
/// number "succeeds" with equally meaningless output, and `null`/
/// `undefined` throws an internal `TypeError` from deep inside the
/// generated glue code instead of a clear message. Accepting `&JsValue`
/// and validating with a real `instanceof Uint8Array` check (via
/// `JsCast::dyn_ref`, which also accepts Node's `Buffer` -- a `Uint8Array`
/// subclass) turns all of these into one explicit, actionable `TypeError`.
fn require_bytes(input: &JsValue, fn_name: &str) -> Result<Vec<u8>, JsValue> {
    // B42 (batis-xml cold code review, ported): fast-pathing same-realm
    // `Uint8Array`/`Buffer` inputs straight through `arr.to_vec()` calls
    // `%TypedArray%.prototype.set` on the underlying `ArrayBuffer` -- for
    // a *detached* buffer (e.g. already transferred to a Worker via
    // `postMessage`) that throws the raw, unhelpful "%TypedArray%.prototype.set
    // on a detached ArrayBuffer" engine error rather than a friendly,
    // actionable message. Both the same-realm and cross-realm duck-typed
    // paths below go through the same fallible copy + friendly-message
    // mapping on failure -- one code path for "read the bytes out of a
    // byte-shaped input", not two with divergent error shapes.
    //
    // B48 (ported): a same-realm `Uint8Array` with `byte_length() > 0` is
    // guaranteed live -- a *detached* view's `byteLength` reads back as
    // `0` per spec (detaching zeroes the view's length rather than
    // leaving stale values), so a non-zero `byteLength` here proves the
    // buffer has not been detached and `.to_vec()` cannot trap. Take that
    // single-copy path directly rather than paying the extra
    // `Reflect.construct` copy below for the common case. A `byteLength`
    // of `0` is ambiguous between "genuinely empty" and "detached", so it
    // still routes through the fallible `copy_bytes_from` path, which
    // tells the two apart by whether the `Reflect.construct` copy throws.
    if let Some(arr) = input.dyn_ref::<js_sys::Uint8Array>() {
        if arr.byte_length() > 0 {
            return Ok(arr.to_vec());
        }
    }

    let claimed_byte_length = if input.dyn_ref::<js_sys::Uint8Array>().is_some() {
        // A real (possibly cross-realm) `Uint8Array`: no independently
        // claimed length to cross-check against, `copy_bytes_from` is
        // trusted directly (same as before this fix).
        None
    } else if let Some(len) = duck_typed_byte_length(input) {
        Some(len)
    } else {
        return wrong_type_err(fn_name, input);
    };

    let bytes = copy_bytes_from(input).map_err(|_| -> JsValue {
        js_sys::TypeError::new(&format!(
            "{fn_name}() was given a byte array whose contents could not be read -- \
             its underlying buffer is likely detached (e.g. already transferred to \
             a Worker via postMessage). Pass a live Uint8Array/Buffer instead."
        ))
        .into()
    })?;

    // Duck-typed inputs only: a plain object that merely *claims* a
    // `byteLength`/`BYTES_PER_ELEMENT` shape isn't a real TypedArray, so
    // `Reflect.construct(Uint8Array, [obj])` falls back to the
    // array-like-object constructor path, which reads the object's
    // `length` property (absent here) rather than its `byteLength` --
    // silently yielding an empty copy instead of the claimed byte count.
    // Cross-checking the actual copy's length against what the object
    // claimed turns that silent truncation into the same explicit
    // TypeError a wrong-typed input already gets, instead of a
    // misleading "no root element found" from parsing bytes that were
    // never really there.
    if let Some(claimed) = claimed_byte_length {
        if bytes.len() as f64 != claimed {
            return wrong_type_err(fn_name, input);
        }
    }

    Ok(bytes)
}

fn wrong_type_err<T>(fn_name: &str, input: &JsValue) -> Result<T, JsValue> {
    Err(js_sys::TypeError::new(&format!(
        "{fn_name}() expects the raw file bytes as a Uint8Array/Buffer -- got {}. \
         Do not pass a decoded string (see README: feed raw bytes, not a \
         host-pre-decoded string) -- read the file as bytes instead.",
        describe_js_value(input)
    ))
    .into())
}

/// Copies bytes out of a byte-shaped input (same-realm `Uint8Array`/
/// `Buffer`, or a cross-realm duck-typed equivalent) via a fresh,
/// same-realm `Uint8Array` built through `Reflect.construct` -- see
/// [`construct_uint8_array_from`]. Used for *both* realms (B42): a
/// detached backing `ArrayBuffer` makes the `new Uint8Array(x)` call
/// itself throw, surfaced here as `Err` rather than trapping the wasm
/// instance, regardless of which realm `input` was constructed in.
fn copy_bytes_from(input: &JsValue) -> Result<Vec<u8>, JsValue> {
    construct_uint8_array_from(input).map(|arr| arr.to_vec())
}

/// Duck-types "is this byte-shaped like a Uint8Array/Buffer" without
/// relying on `instanceof` (see [`require_bytes`]'s realm note above).
/// Returns the object's claimed `byteLength` (as a JS number) so the
/// caller can cross-check it against what `copy_bytes_from` actually
/// copies -- a plain object merely duck-typed to look byte-shaped (no
/// real `length`-indexed contents) copies as empty regardless of the
/// `byteLength` it claims, and that mismatch is `require_bytes`'s only
/// way to tell the two apart (see its cross-check).
fn duck_typed_byte_length(input: &JsValue) -> Option<f64> {
    if !input.is_object() {
        return None;
    }
    let byte_length = js_sys::Reflect::get(input, &JsValue::from_str("byteLength"))
        .ok()
        .and_then(|v| v.as_f64())?;
    let bytes_per_element = js_sys::Reflect::get(input, &JsValue::from_str("BYTES_PER_ELEMENT"))
        .ok()
        .and_then(|v| v.as_f64())?;
    (bytes_per_element == 1.0).then_some(byte_length)
}

/// Builds a fresh, same-realm `Uint8Array` by calling the global
/// `Uint8Array` constructor via `Reflect.construct` (rather than
/// `js_sys::Uint8Array::new`, which isn't `catch`-enabled and would trap
/// the whole wasm instance instead of surfacing a `Result` if the
/// constructor throws -- see the detached-ArrayBuffer case above).
fn construct_uint8_array_from(input: &JsValue) -> Result<js_sys::Uint8Array, JsValue> {
    let global = js_sys::global();
    let ctor = js_sys::Reflect::get(&global, &JsValue::from_str("Uint8Array"))?;
    let ctor: js_sys::Function = ctor.unchecked_into();
    let args = js_sys::Array::of1(input);
    let value = js_sys::Reflect::construct(&ctor, &args)?;
    Ok(value.unchecked_into())
}

/// A short, human-readable description of a `JsValue`'s type for the error
/// message above -- not a full `typeof`, just enough to name the mistake.
fn describe_js_value(v: &JsValue) -> &'static str {
    if v.is_null() {
        "null"
    } else if v.is_undefined() {
        "undefined"
    } else if v.as_string().is_some() {
        "a string"
    } else if v.as_f64().is_some() {
        "a number"
    } else if v.as_bool().is_some() {
        "a boolean"
    } else if v.is_instance_of::<js_sys::Array>() {
        "a plain Array (not a Uint8Array)"
    } else {
        "an unsupported value"
    }
}

/// Parses `<beans>` XML bytes and returns the `ParseResult` (schema v1) as
/// a JSON string. Never panics: encoding/parse failures already surface as
/// diagnostics inside the JSON per the core crate's contract (`parse`/
/// `parse_bytes` never return `Err`), and the (practically unreachable,
/// since `ParseResult` has no non-string map keys) serialization failure
/// case falls back to the JSON literal `null` rather than trapping the
/// wasm instance. Throws a `TypeError` (rejecting the call, not silently
/// coercing) if `input` isn't a `Uint8Array`/`Buffer` -- see
/// [`require_bytes`].
#[wasm_bindgen]
pub fn parse(
    #[wasm_bindgen(unchecked_param_type = "Uint8Array")] input: &JsValue,
) -> Result<String, JsValue> {
    let bytes = require_bytes(input, "parse")?;
    let result = beans_xml::parse_bytes(&bytes);
    Ok(serde_json::to_string(&result).unwrap_or_else(|_| "null".to_string()))
}

/// Cheap pre-check: is the document root `<beans>`? Guaranteed to agree
/// with `parse(bytes)`'s `beans` field being non-null (the core crate's
/// invariant #7: `is_beans_doc(b) == parse_bytes(b).beans.is_some()`).
/// Same `Uint8Array`/`Buffer` input validation as `parse` (A16).
#[wasm_bindgen(js_name = is_beans_doc)]
pub fn is_beans_doc(
    #[wasm_bindgen(unchecked_param_type = "Uint8Array")] input: &JsValue,
) -> Result<bool, JsValue> {
    let bytes = require_bytes(input, "is_beans_doc")?;
    Ok(beans_xml::is_beans_doc(&bytes))
}

/// This crate's version, from `Cargo.toml`.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
