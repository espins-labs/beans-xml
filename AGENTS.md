# beans-xml — agent working agreement

A lenient parser for **Spring Framework bean XML** (`<beans>` schema). Any coding
agent (Claude Code, Cursor, Codex, …) reads this file. Pure doc — no tool-specific
machinery lives in this repo (workspace rule).

**The internal design spec is the single source of truth** for purpose/non-goals,
the output model, invariants, and kill criteria; the internal build plan is the
single source of truth for the unit decomposition (U0…, P1…, I1…) and per-unit
test design (both maintained privately, not in this repo). The **public** contract,
for anyone outside this working agreement, is the published JSON schema
(`schema/beans-xml.v1.json`, once emitted) plus the conformance fixtures in
`fixtures/` — that's what ports/consumers conform to. The dispatch contract that
keeps parallel leaf units conflict-free is summarized below so it stays
self-contained now that the build plan isn't in-repo.

## Method (non-negotiable)

1. **Freeze the contracts first.** `src/model.rs` (the **data** contract, unit U0) and
   the **dispatch contract** (U3 root-child match, U4 `parse_bean` skeleton, handler
   signature, `NS_REF_ALLOWLIST`) are frozen before the parallel leaf wave. Do not
   change model fields without explicit approval — one exception: **adding `DiagCode`
   variants is allowed** (additive), but call it out in your status report so the
   internal design spec stays in sync.
2. **Test-first, unit order** per the internal build plan: U0 → (U1 ∥ U2) → U3 → U4 → U5a →
   (U6 ∥ U7) → leaf wave (P*) → integration (I*). One unit = one order block = tests
   first, minimal impl, refactor, **one commit per unit**.
3. **Test naming**: prefix with the unit/SB id (e.g. `u4_bean_core_*`, `sb02_*`) for
   traceability. Each unit owns its own `tests/<unit>.rs` + insta snapshot subdir; a
   leaf fills only its own module handler fn — **zero lines of the shared dispatch
   match** (dispatch contract), so parallel implementers never collide.
4. **No panics** on public paths; no `unwrap`/`expect` outside tests. Every anomaly →
   a `Diagnostic`. `parse`/`parse_bytes` never return `Err`.
5. **Spans are decoded-UTF-8 offsets** (not raw input bytes) — non-UTF-8 input is
   re-sliced by consumers via `ParseResult::encoding`. (This is the corrected contract;
   it differs from a stale one-liner — see the internal design spec's ByteSpan doc.)
6. **References are raw** — record `ref`/`parent`/SpEL `#{}` target names verbatim;
   resolution (cross-file / component-scan) is the consumer's job.
7. **English only** in code, docs, commit messages.

## Leak safety (publication)

Fixtures are **synthetic `com.example.*` only** — never derive from any company /
private corpus (class names, bean ids, packages, cron values, hosts). A private
benchmark corpus is read only via the `BEANSXML_PRIVATE_CORPUS` env var and is
excluded from the published package (`Cargo.toml` `include` allowlist). **Before any
publish, run a leak scan** over the working tree + git history + packed tarball
(company vocabulary deny-list, injected — never committed). See the internal design
spec's fixture section.

## Gates (run before claiming done)

```
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test
cargo check --target wasm32-unknown-unknown   # pure-Rust deps only
```

## Git safety

Never run destructive git commands (`checkout --`, `restore`, `reset --hard`) while
uncommitted unique work exists — make a WIP commit first.
