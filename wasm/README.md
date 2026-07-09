# beans-xml

WebAssembly bindings for [`beans-xml`](https://github.com/espins-labs/beans-xml)
— a lenient parser for Spring Framework `<beans>` XML configuration.

```js
const beansXml = require("beans-xml");

const bytes = fs.readFileSync("applicationContext.xml"); // Buffer, NOT a decoded string
const result = JSON.parse(beansXml.parse(bytes)); // schema v1, see schema/beans-xml.v1.json
const isBeansDoc = beansXml.is_beans_doc(bytes); // cheap root-element pre-check
```

**Node.js target only — no browser/bundler build yet.** This package is
built with `wasm-pack --target nodejs` (CommonJS, loads the `.wasm` via
`fs.readFileSync` at require time). It will not work as-is in a browser
or with a bundler expecting `--target web`/`--target bundler` output
(`fetch`-based instantiation, ESM). That's a separate build target to
add later, not a difference in the Rust source.

## Two things that will bite you

**(a) Feed raw bytes — never a host-pre-decoded string.** Always pass the
file's original `Buffer`/`Uint8Array` to `parse`/`is_beans_doc`, not a
string you already decoded (e.g. `fs.readFileSync(path, "utf-8")` then
re-encoded). `beans-xml` detects the encoding itself (BOM sniff, UTF-8,
XML declaration `encoding=` label, an EUC-KR heuristic for
declaration-less legacy files, then a lossy fallback) — `result.encoding`
reports which of these won. Feeding it bytes that already went through a
host UTF-8 decoder defeats all of that, since a genuinely non-UTF-8 file
would already have been mangled (replacement characters) before
`beans-xml` ever sees it.

**(b) Spans are byte offsets into the UTF-8 text `beans-xml` itself
decoded — never JS string indices, and never the *original* file's raw
bytes for anything but a UTF-8 source.** Every `ByteSpan { start, end }`
in the JSON indexes into the UTF-8 bytes of the *decoded* text, while a JS
string is indexed by UTF-16 code units — these diverge the moment a
multi-byte character (e.g. Korean identifiers) appears before the offset
you care about. `result.encoding` (the WHATWG name `TextDecoder` accepts
directly) is what makes this reproducible:

```js
// bytes is the same Buffer/Uint8Array you fed to parse()
const decodedText = new TextDecoder(result.encoding).decode(bytes);
const utf8Bytes = new TextEncoder().encode(decodedText); // byte-identical to beans-xml's own internal String
const text = new TextDecoder("utf-8").decode(
  utf8Bytes.subarray(span.start, span.end)
);
```

If the input was plain UTF-8, `bytes` and `utf8Bytes` are already
byte-identical (decoding then re-encoding UTF-8 is a no-op), so slicing
`bytes` directly happens to work in that one case — but relying on that
silently breaks the moment a file turns out to be EUC-KR/CP949/UTF-16/
etc., which is exactly the failure mode `result.encoding` exists to
prevent. Always go through the `TextDecoder`/`TextEncoder` round trip
above regardless of what encoding you expect.

## References are raw

`ref="beanA"` / `bean="beanA"` / `local="beanA"` / `parent="beanA"` are
recorded as the raw name `beanA` in `BeanRef.raw` — resolving it to an
actual bean (across imported files, `component-scan`-declared beans, or
XML-vs-annotation config) is the consumer's job. A parser sees one file.
