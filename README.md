# beans-xml

[![crates.io](https://img.shields.io/crates/v/beans-xml.svg)](https://crates.io/crates/beans-xml)
[![docs.rs](https://img.shields.io/docsrs/beans-xml)](https://docs.rs/beans-xml)
[![npm](https://img.shields.io/npm/v/beans-xml.svg)](https://www.npmjs.com/package/beans-xml)
[![CI](https://github.com/espins-labs/beans-xml/actions/workflows/ci.yml/badge.svg)](https://github.com/espins-labs/beans-xml/actions/workflows/ci.yml)
[![license](https://img.shields.io/crates/l/beans-xml.svg)](#license)

A lenient parser for **Spring Framework bean XML configuration** — the `<beans>`
schema: bean definitions, dependency wiring (`ref` / `property` / `constructor-arg`),
`<import>`, `component-scan`, profiles, p/c-namespace, collections, method injection.

> **Spring beans — not EJB / JavaBeans / CDI.** This parses the Spring
> `<beans xmlns="http://www.springframework.org/schema/beans">` DSL, not Enterprise
> JavaBeans, `java.beans` JavaBeans, or CDI managed beans. Unofficial; not affiliated
> with or endorsed by Broadcom / the Spring project.

Distributed on **crates.io** (`beans-xml`) and **npm** (WASM). Sibling of
[`batis-xml`](https://github.com/espins-labs/batis-xml) — same lenient, never-panic,
byte-span, JSON-output contract.

## What it does

- Turns one `<beans>` XML file into a structured model: every bean (id/name/class/
  parent/scope/factory/…), its properties and constructor-args, references, imports,
  aliases, component-scans, profiles.
- **Never panics, never returns `Err`** — broken/legacy input yields a partial result
  plus diagnostics. Detects encoding (UTF-8 / EUC-KR / CP949 / UTF-16).
- **References are raw**: `ref="x"` is recorded as the name `x`; resolving it to an
  actual bean (across files / component-scan) is the consumer's job — a parser sees
  one file, and cross-file / annotation-declared beans aren't in it.

## Installation

**Rust** (crates.io):

```toml
[dependencies]
beans-xml = "0.1"
```

or `cargo add beans-xml`. Full API documentation is on
[docs.rs](https://docs.rs/beans-xml). The optional `schema` feature adds
`schemars`-generated JSON Schema types (the pinned schema itself ships in
`schema/beans-xml.v1.json` regardless).

**Node / TypeScript** (npm): the same parser compiled to WebAssembly, same
JSON output, published as [`beans-xml`](https://www.npmjs.com/package/beans-xml):

```bash
npm install beans-xml
```

Feed it raw file bytes (a `Buffer`/`Uint8Array`, never a pre-decoded
string) — the package README documents the byte-span and encoding caveats.

**Build from source**:

```bash
git clone https://github.com/espins-labs/beans-xml
cd beans-xml
cargo test                  # core crate (MSRV 1.79)
./wasm/build.sh             # wasm package -> wasm/pkg (needs wasm-pack + Node)
node wasm/tests/smoke.js
```

## Status

**Parser complete** — every unit in the internal build plan (spec: `SB-01` through
`SB-16`, maintained privately, not in this repo) is implemented: bean core, p/c-namespace,
collections (`list`/`set`/`array`/`map`/`props`, `merge`), `import`/`alias`,
`component-scan`, nested `<beans profile=...>`, method injection (lookup-method/
replaced-method), qualifier/meta/decorator, `${}`/`#{}` placeholder and SpEL
bean-reference harvesting, and namespaced allowlisted-ref elements (`aop:`/`tx:`/
`task:`/`jee:`/...).

- **500+ tests**, ~97% line coverage (`cargo llvm-cov`).
- A public conformance fixture corpus (`fixtures/`) — the crate's actual public
  contract alongside `schema/beans-xml.v1.json` — locks parser behavior against
  regressions (`tests/conformance.rs`).
- WASM build (`wasm/`, npm-published) alongside the crates.io Rust crate.
- Two runnable examples: `bean_list` (flat bean listing) and `dep_graph_dot`
  (bean-to-bean dependency graph as Graphviz DOT).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
