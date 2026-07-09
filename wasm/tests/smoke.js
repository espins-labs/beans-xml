// Smoke test for the beans-xml-wasm bindings (nodejs target).
//
// Not part of `cargo test` -- this checks the built wasm-pack output, so
// it must run *after* `wasm-pack build wasm --target nodejs` (or simply
// `./wasm/build.sh`):
//
//   ./wasm/build.sh
//   node wasm/tests/smoke.js
//
// Parses fixtures/core/minimal_bean.xml and asserts the known bean id and
// class both appear in the returned JSON, plus is_beans_doc()/version()
// and the Uint8Array-only input contract (batis-xml sibling pattern).

const fs = require("fs");
const path = require("path");
const vm = require("node:vm");
const wasm = require("../pkg/beans_xml_wasm.js");

const fixturePath = path.join(
  __dirname,
  "..",
  "..",
  "fixtures",
  "core",
  "minimal_bean.xml",
);
const bytes = fs.readFileSync(fixturePath);

const json = wasm.parse(new Uint8Array(bytes));
const result = JSON.parse(json);

assert(
  json.includes("widgetService"),
  "expected bean id 'widgetService' in the JSON output",
);
assert(
  json.includes("com.example.WidgetService"),
  "expected class 'com.example.WidgetService' in the JSON output",
);
assert(
  result.beans.beans[0].id.value === "widgetService",
  "expected the parsed bean id to round-trip through JSON.parse",
);
assert(
  typeof result.beans.beans[0].span === "object" &&
    typeof result.beans.beans[0].span.start === "number",
  "expected the bean's span field to be present",
);

const isBeansDoc = wasm.is_beans_doc(new Uint8Array(bytes));
assert(
  isBeansDoc === true,
  `expected is_beans_doc() to return true, got ${JSON.stringify(isBeansDoc)}`,
);
assert(
  isBeansDoc === (result.beans !== null),
  "expected is_beans_doc() to agree with parse()'s beans field (invariant #7)",
);

console.log(`wasm.version() = ${wasm.version()}`);
console.log(`wasm.is_beans_doc() = ${isBeansDoc}`);
console.log(`JSON output size: ${json.length} bytes`);
console.log(
  "PASS: bean id, span, class, and is_beans_doc() all present/correct",
);

// A16 (batis-xml cold code review, ported): parse()/is_beans_doc() must
// not silently coerce a wrong-typed input -- a string would become a
// zero-filled buffer (misleading "no root element found", not an error
// about the real mistake), a number "succeeds" with meaningless output,
// and null/undefined throws an internal TypeError from deep inside the
// generated glue code. All three must throw one clear, explicit
// TypeError instead.

assertThrowsTypeError(
  () => wasm.parse("<beans></beans>"),
  "parse",
  "a string",
);
assertThrowsTypeError(() => wasm.parse(42), "parse", "a number");
assertThrowsTypeError(() => wasm.parse(null), "parse", "null");
assertThrowsTypeError(
  () => wasm.is_beans_doc("<beans></beans>"),
  "is_beans_doc",
  "a string",
);
assertThrowsTypeError(() => wasm.is_beans_doc(42), "is_beans_doc", "a number");
assertThrowsTypeError(() => wasm.is_beans_doc(null), "is_beans_doc", "null");

console.log(
  "PASS: parse()/is_beans_doc() reject string/number/null input with a clear TypeError",
);

// B39 (batis-xml cold code review, ported): `instanceof Uint8Array` is
// realm-bound -- a genuine Uint8Array constructed via node:vm's separate
// context (a stand-in for a different iframe/Worker realm) has a
// *different* Uint8Array constructor identity, so `instanceof` alone
// would wrongly reject it. parse()/is_beans_doc() must accept it via the
// duck-typed fallback.
const otherRealmUint8Array = vm.runInContext(
  "Uint8Array",
  vm.createContext({}),
);
const crossRealmBytes = new otherRealmUint8Array(bytes);
assert(
  !(crossRealmBytes instanceof Uint8Array),
  "test setup: this array must actually be from a different realm",
);
const crossRealmResult = JSON.parse(wasm.parse(crossRealmBytes));
assert(
  crossRealmResult.beans.beans[0].id.value === "widgetService",
  "expected a cross-realm Uint8Array to parse the same as a same-realm one",
);
assert(
  wasm.is_beans_doc(crossRealmBytes) === true,
  "expected is_beans_doc() to also accept a cross-realm Uint8Array",
);
console.log("PASS: parse()/is_beans_doc() accept a cross-realm Uint8Array");

// A detached backing buffer (e.g. already transferred to a Worker via
// postMessage) must give a friendly, specific message -- not whatever raw
// exception the JS engine's Uint8Array constructor happens to throw.
if (typeof ArrayBuffer.prototype.transfer === "function") {
  const detachedSource = new otherRealmUint8Array(bytes);
  detachedSource.buffer.transfer();
  try {
    wasm.parse(detachedSource);
    assert(false, "expected parse(detached) to throw");
  } catch (err) {
    assert(
      err instanceof TypeError,
      `expected a TypeError for a detached buffer, got ${err}`,
    );
    assert(
      err.message.includes("detached"),
      `expected the detached-buffer message to say so plainly, got: ${err.message}`,
    );
    assert(
      err.message.includes("Pass a live Uint8Array/Buffer instead"),
      `expected the crate's specific actionable wording, not the raw engine error, got: ${err.message}`,
    );
  }
  console.log("PASS: parse() gives a friendly message for a detached buffer");
} else {
  console.log(
    "SKIP: ArrayBuffer.prototype.transfer unavailable in this Node version",
  );
}

// B42 (batis-xml cold code review, ported): the *same-realm* fast path (a
// genuine `instanceof Uint8Array` that passes the dyn_ref check directly)
// must give the identical friendly message as the cross-realm path, not
// the raw engine "detached ArrayBuffer" exception.
if (typeof ArrayBuffer.prototype.transfer === "function") {
  const sameRealmDetached = new Uint8Array(bytes);
  sameRealmDetached.buffer.transfer();
  assert(
    sameRealmDetached instanceof Uint8Array,
    "test setup: this array must be a genuine same-realm Uint8Array",
  );
  try {
    wasm.parse(sameRealmDetached);
    assert(false, "expected parse(same-realm detached) to throw");
  } catch (err) {
    assert(
      err instanceof TypeError,
      `expected a TypeError for a same-realm detached buffer, got ${err}`,
    );
    assert(
      err.message.includes("detached"),
      `expected the same-realm detached-buffer message to say so plainly, got: ${err.message}`,
    );
    assert(
      err.message.includes("Pass a live Uint8Array/Buffer instead"),
      `expected the crate's specific actionable wording (same as the cross-realm case), not the raw engine error, got: ${err.message}`,
    );
  }
  console.log(
    "PASS: parse() gives the same friendly message for a SAME-realm detached buffer",
  );
} else {
  console.log(
    "SKIP: ArrayBuffer.prototype.transfer unavailable in this Node version",
  );
}

// A plain Array must still be rejected (no BYTES_PER_ELEMENT at all) --
// the duck-typing fallback must not loosen A16's original guarantee.
assertThrowsTypeError(() => wasm.parse([1, 2, 3]), "parse", "a plain array");
console.log("PASS: parse() still rejects a plain Array");

// A plain object that merely *claims* a byte-array shape (byteLength +
// BYTES_PER_ELEMENT, but no real indexed contents) must not be silently
// accepted as empty input -- it isn't a real TypedArray, so the
// `Reflect.construct` copy the duck-typed path takes actually yields zero
// bytes regardless of the claimed byteLength (its `length` property, the
// array-like constructor's real source of truth, is absent). Without a
// byteLength cross-check, `parse()` would return the same "no root
// element found" result as truly empty input instead of rejecting the
// mistaken input outright.
assertThrowsTypeError(
  () => wasm.parse({ byteLength: 8, BYTES_PER_ELEMENT: 1 }),
  "parse",
  "a fake byte-shaped plain object",
);
console.log(
  "PASS: parse() rejects a plain object that only claims a byte-array shape",
);

// A genuinely-empty same-realm Uint8Array (byteLength === 0, ambiguous
// with "detached" by that signal alone) must parse as empty input --
// diagnostics only, never a thrown error (B48 ported).
{
  const emptySameRealm = new Uint8Array(0);
  assert(
    emptySameRealm instanceof Uint8Array,
    "test setup: this array must be a genuine same-realm Uint8Array",
  );
  const emptyResult = JSON.parse(wasm.parse(emptySameRealm));
  assert(
    emptyResult.beans === null,
    `expected an empty same-realm Uint8Array to parse as empty input (no root element), got: ${JSON.stringify(emptyResult)}`,
  );
  assert(
    Array.isArray(emptyResult.diagnostics) && emptyResult.diagnostics.length > 0,
    "expected parse() to return normally (not throw) for an empty same-realm Uint8Array, with a diagnostic explaining the empty input",
  );
  assert(
    wasm.is_beans_doc(emptySameRealm) === false,
    "expected is_beans_doc() to return false for empty input, not throw",
  );
  console.log(
    "PASS: parse()/is_beans_doc() accept a genuinely-empty same-realm Uint8Array as empty input, not an error",
  );
}

function assert(cond, message) {
  if (!cond) {
    console.error(`FAIL: ${message}`);
    process.exit(1);
  }
}

function assertThrowsTypeError(fn, fnName, inputDescription) {
  try {
    fn();
  } catch (err) {
    assert(
      err instanceof TypeError,
      `expected ${fnName}(${inputDescription}) to throw a TypeError, got ${err}`,
    );
    assert(
      err.message.includes(fnName) && err.message.includes("Uint8Array"),
      `expected ${fnName}(${inputDescription})'s error message to name the function and expected type, got: ${err.message}`,
    );
    return;
  }
  assert(false, `expected ${fnName}(${inputDescription}) to throw, but it returned normally`);
}
