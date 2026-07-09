//! Unit **I3** — hostile-input resilience (SB-16), invariant #1 ("no panics/
//! infinite loops on any byte input (10MB · depth-256 hard constant)"), driven through the
//! **whole public pipeline** — `parse`/`parse_bytes`/`is_beans_doc` — not
//! just one layer. M0a's own panic-free proptests each stop at a single
//! `pub(crate)` seam (`events::build_tree` in `src/events.rs`'s own
//! `#[cfg(test)]`, `is_beans_doc` in `tests/u3_root_dispatch.rs`); this file
//! is the build plan's I3 row ("[all]") — it fuzzes malformed/truncated/
//! adversarial bytes all the way through dispatch → bean → property/
//! constructor-arg → inject_value → collection → namespaced, the layers
//! those per-unit generators never reach.
//!
//! Two proptest generators plus a hand-written hostile-fixture table (build
//! plan: "proptest invariant #1 (arbitrary bytes · truncation) + hostile fixtures").

use beans_xml::{DiagCode, DEPTH_LIMIT};

/// Drives every public entry point on the same bytes and asserts nothing
/// panics. Deliberately asserts nothing else — a panic already fails the
/// enclosing `#[test]`/proptest case on its own; this function's only job
/// is "exercise all three public entry points", not add assertions that
/// would narrow what a proptest shrinker converges on.
fn assert_pipeline_never_panics(bytes: &[u8]) {
    let _ = beans_xml::parse_bytes(bytes);
    let _ = beans_xml::is_beans_doc(bytes);
    // `parse(&str)` takes an already-decoded string — lossy-decode first,
    // same fallback `tests/u1_events.rs`'s own arbitrary-bytes proptest
    // uses, so this entry point gets exercised by the same corpus rather
    // than only ever seeing valid-UTF-8 generators.
    let lossy = String::from_utf8_lossy(bytes).into_owned();
    let _ = beans_xml::parse(&lossy);
}

// ---------------------------------------------------------------------
// proptest: invariant #1 — arbitrary bytes, arbitrary unicode, truncation
// of an otherwise-valid document, and depth/breadth-stressed "beans-like"
// shapes with junk, all driven through the full pipeline.
// ---------------------------------------------------------------------

/// A single, deliberately feature-dense valid `<beans>` document —
/// import/alias/component-scan/property-placeholder/util:properties/p-
/// namespace/qualifier/meta/property+constructor-arg/list+ref/map+entry/
/// props+prop/lookup-method/replaced-method/decorator/nested profile — so
/// truncating it at an arbitrary byte offset exercises every later layer's
/// own recovery path (unclosed tag mid-attribute, mid-child, mid-text,
/// ...), not just the root-level shapes a smaller fixture would reach.
const VALID_DOC: &str = concat!(
    "<beans xmlns:context=\"http://www.springframework.org/schema/context\" ",
    "xmlns:util=\"http://www.springframework.org/schema/util\" ",
    "xmlns:aop=\"http://www.springframework.org/schema/aop\" ",
    "xmlns:p=\"http://www.springframework.org/schema/p\">",
    "<description>desc</description>",
    "<import resource=\"classpath:other.xml\"/>",
    "<alias name=\"a\" alias=\"b\"/>",
    "<context:component-scan base-package=\"com.example\"/>",
    "<context:property-placeholder location=\"classpath:app.properties\"/>",
    "<util:properties id=\"props\" location=\"classpath:app2.properties\"/>",
    "<bean id=\"outer\" class=\"com.example.Outer\" p:name=\"literal\">",
    "<qualifier value=\"main\"><attribute key=\"k\" value=\"v\"/></qualifier>",
    "<meta key=\"k\" value=\"v\"/>",
    "<property name=\"list\"><list><ref bean=\"other\"/><value>x</value></list></property>",
    "<property name=\"map\"><map><entry key=\"k\" value=\"v\"/></map></property>",
    "<property name=\"props\"><props><prop key=\"k\">v</prop></props></property>",
    "<constructor-arg index=\"0\" value=\"42\"/>",
    "<lookup-method name=\"m\" bean=\"other\"/>",
    "<replaced-method name=\"m2\" replacer=\"other\">",
    "<arg-type match=\"java.lang.String\"/></replaced-method>",
    "<aop:scoped-proxy proxy-target-class=\"true\"/>",
    "</bean>",
    "<bean id=\"other\" class=\"com.example.Other\"/>",
    "<beans profile=\"dev\"><bean id=\"devBean\" class=\"com.example.Dev\"/></beans>",
    "</beans>",
);
const VALID_DOC_LEN: usize = VALID_DOC.len();

proptest::proptest! {
    #[test]
    fn i3_invariant1_arbitrary_bytes_never_panic(
        bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..2000)
    ) {
        assert_pipeline_never_panics(&bytes);
    }

    #[test]
    fn i3_invariant1_arbitrary_unicode_str_never_panics(s in ".{0,1000}") {
        assert_pipeline_never_panics(s.as_bytes());
    }

    /// Truncation, per the build plan's own I3 row ("arbitrary bytes · truncation"): a
    /// truncated valid document is exactly the shape most likely to land
    /// mid-tag, mid-attribute-value, mid-entity, or mid-multi-byte-UTF-8-
    /// sequence (the doc has none of the latter itself, but the cut point
    /// still lands mid-tag/mid-attribute for the vast majority of offsets)
    /// — the recovery rules (unclosed tag, orphan close, ...) exist
    /// exactly for this shape.
    #[test]
    fn i3_invariant1_truncated_valid_doc_never_panics(cut in 0usize..=VALID_DOC_LEN) {
        let bytes = &VALID_DOC.as_bytes()[..cut];
        assert_pipeline_never_panics(bytes);
    }

    /// Combines nesting depth (both `<bean>`/`<property>` mutual recursion
    /// and `<list>` self-recursion, both DEPTH_LIMIT-guarded at different
    /// call sites — see `inject_value::parse_inner_bean`/
    /// `collection::parse_collection_value`) with arbitrary junk text at
    /// the bottom, driven through the full pipeline rather than either
    /// guard's own isolated unit-level proptest.
    #[test]
    fn i3_invariant1_beans_like_shapes_with_junk_never_panic(
        bean_depth in 0usize..40,
        collection_depth in 0usize..40,
        junk in ".{0,30}",
    ) {
        let mut s = String::from("<beans><bean class=\"com.example.X\">");
        for _ in 0..bean_depth {
            s.push_str("<property name=\"p\"><bean class=\"com.example.X\">");
        }
        s.push_str("<property name=\"leaf\">");
        for _ in 0..collection_depth {
            s.push_str("<list>");
        }
        s.push_str(&junk);
        for _ in 0..collection_depth {
            s.push_str("</list>");
        }
        s.push_str("</property>");
        for _ in 0..bean_depth {
            s.push_str("</bean></property>");
        }
        s.push_str("</bean></beans>");
        assert_pipeline_never_panics(s.as_bytes());
    }

    /// Breadth, not depth: a large flat attribute count and a large flat
    /// sibling count are both `O(n)` scans/pushes this crate never bounds
    /// with `DEPTH_LIMIT` (they don't recurse) — this pins that "many" on
    /// its own, without nesting, still never panics (and, unlike the depth
    /// proptests above, is never expected to hit `NestingLimitExceeded`).
    #[test]
    fn i3_invariant1_wide_attribute_and_sibling_counts_never_panic(
        attr_count in 0usize..500,
        sibling_count in 0usize..500,
    ) {
        let mut s = String::from("<beans><bean");
        for i in 0..attr_count {
            s.push_str(&format!(" a{i}=\"v{i}\""));
        }
        s.push('>');
        for i in 0..sibling_count {
            s.push_str(&format!("<property name=\"p{i}\" value=\"v{i}\"/>"));
        }
        s.push_str("</bean></beans>");
        assert_pipeline_never_panics(s.as_bytes());
    }
}

// ---------------------------------------------------------------------
// Hostile fixtures — hand-written adversarial shapes a random generator
// is unlikely to hit on its own (build plan: "+ hostile fixtures").
// ---------------------------------------------------------------------

fn chain(open: &str, close: &str, count: usize, leaf: &str) -> String {
    let mut s = String::new();
    for _ in 0..count {
        s.push_str(open);
    }
    s.push_str(leaf);
    for _ in 0..count {
        s.push_str(close);
    }
    s
}

#[test]
fn i3_hostile_fixtures_never_panic() {
    let over_limit = DEPTH_LIMIT as usize + 20;

    let deep_list = format!(
        "<beans><bean class=\"com.example.A\"><property name=\"p\">{}</property></bean></beans>",
        chain("<list>", "</list>", over_limit, "<value>leaf</value>")
    );
    let deep_inner_bean = format!(
        "<beans><bean class=\"com.example.A\">{}</bean></beans>",
        chain(
            "<property name=\"p\"><bean class=\"com.example.A\">",
            "</bean></property>",
            over_limit,
            "<property name=\"leaf\" value=\"x\"/>",
        )
    );
    let deep_profile = format!(
        "<beans>{}</beans>",
        chain("<beans profile=\"p\">", "</beans>", over_limit, "")
    );
    let deep_map = format!(
        "<beans><bean class=\"com.example.A\"><property name=\"p\">{}</property></bean></beans>",
        // One `<map><entry>` pair per level, each level's `<entry>` value a
        // nested `<map>` (not a bare `<map>` sibling directly inside the
        // outer `<map>` — `parse_map` only ever descends into `<entry>`
        // children, so a `<map>` not wrapped in an `<entry>` would just be
        // silently skipped rather than adding a nesting hop).
        chain(
            "<map><entry key=\"k\">",
            "</entry></map>",
            over_limit,
            "<map><entry key=\"leaf\" value=\"x\"/></map>",
        )
    );

    let hostile_fixtures: &[&[u8]] = &[
        b"",
        b"<",
        b"<>",
        b"</>",
        b"\0\0\0\0\0\0\0\0",
        b"\xff\xfe\x00\x01\x02\x03garbage",
        b"\x89PNG\r\n\x1a\n\0\0\0\0", // fake PNG header, not XML at all
        b"<beans><bean id=\"a\"",     // truncated mid start-tag
        b"<beans><bean id=\"a\"/><bean", // truncated mid second start-tag
        b"<beans>&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&</beans>",
        b"<beans><![CDATA[unterminated",
        b"<beans><!-- unterminated comment",
        b"<beans><bean id=\"a\" id=\"b\" id=\"c\" id=\"d\" id=\"e\"/></beans>",
        b"<beans></a></b></c></d></e></beans>", // all-orphan close tags
        b"<beans><value>${${${${${${unterminated</value></beans>",
        b"<beans><value>#{#{#{#{#{#{unterminated</value></beans>",
        deep_list.as_bytes(),
        deep_inner_bean.as_bytes(),
        deep_profile.as_bytes(),
        deep_map.as_bytes(),
    ];

    for fixture in hostile_fixtures {
        assert_pipeline_never_panics(fixture);
    }

    // The DEPTH_LIMIT-shaped fixtures must actually reach the guard through
    // the full pipeline (not just silently succeed some other way) — pins
    // that `NestingLimitExceeded` is reachable end-to-end, complementing
    // (not duplicating) the isolated unit-level boundary proptests in
    // `src/inject_value.rs`/`src/collection.rs`/`src/dispatch.rs`.
    for (name, fixture) in [
        ("deep_list", &deep_list),
        ("deep_inner_bean", &deep_inner_bean),
        ("deep_profile", &deep_profile),
        ("deep_map", &deep_map),
    ] {
        let result = beans_xml::parse(fixture);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == DiagCode::NestingLimitExceeded),
            "{name} must trigger NestingLimitExceeded through the full pipeline: {:?}",
            result.diagnostics
        );
    }
}

// ---------------------------------------------------------------------
// P0 regression: deep-input stack overflow on `Drop` (invariant #1),
// driven through the public `parse`/`parse_bytes` entry points rather
// than `events::build_tree` directly (that direct unit-level guard lives
// in `src/events.rs`'s own `#[cfg(test)] mod tests`). 60_000 is well
// beyond `DEPTH_LIMIT` (256) and comfortably under `MAX_INPUT_BYTES` —
// before the fix (`events::build_tree` capping the raw tree's own
// structural depth), each of these reliably aborted the process with a
// stack overflow while dropping the fully-built, unbounded `XmlElement`
// tree, confirmed by running this file's tests against the pre-fix
// `src/events.rs` (`thread ... has overflowed its stack, aborting`).
// ---------------------------------------------------------------------

fn assert_no_panic_and_nesting_limit_diagnosed(source: &str) {
    let result = beans_xml::parse(source);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.code == DiagCode::NestingLimitExceeded),
        "expected a NestingLimitExceeded diagnostic: {:?}",
        result.diagnostics
    );

    let bytes_result = beans_xml::parse_bytes(source.as_bytes());
    assert!(
        bytes_result
            .diagnostics
            .iter()
            .any(|d| d.code == DiagCode::NestingLimitExceeded),
        "expected a NestingLimitExceeded diagnostic via parse_bytes: {:?}",
        bytes_result.diagnostics
    );
}

#[test]
fn i3_p0_deeply_nested_plain_elements_never_abort_the_stack() {
    // A generic (non-`<beans>`) deep chain — root detection rejects it
    // before any bean/collection walk ever runs, so this is the most
    // direct end-to-end proof that `events::build_tree` itself (not a
    // downstream depth guard) is what bounds the tree's own structural
    // depth: nothing but that layer's own `NestingLimitExceeded` can fire
    // here.
    const N: usize = 60_000;
    let source = format!("{}{}", "<foo>".repeat(N), "</foo>".repeat(N));
    assert_no_panic_and_nesting_limit_diagnosed(&source);
}

#[test]
fn i3_p0_deeply_nested_beans_never_abort_the_stack() {
    const N: usize = 60_000;
    let source = format!("{}{}", "<beans>".repeat(N), "</beans>".repeat(N));
    assert_no_panic_and_nesting_limit_diagnosed(&source);
}

#[test]
fn i3_p0_deeply_nested_collection_never_aborts_the_stack() {
    const N: usize = 60_000;
    let source = format!(
        "<beans><bean class=\"com.example.A\"><property name=\"p\">{}<value>leaf</value>{}</property></bean></beans>",
        "<list>".repeat(N),
        "</list>".repeat(N),
    );
    assert_no_panic_and_nesting_limit_diagnosed(&source);
}

#[test]
fn i3_p0_small_stack_thread_deeply_nested_input_does_not_overflow() {
    // Cross-platform reproduction of a Windows-only CI failure: on
    // windows-latest, `cargo test` died in this file with exit code
    // 0xc00000fd (STATUS_STACK_OVERFLOW) even though the three
    // `i3_p0_deeply_nested_*` tests above (same 60_000-deep shapes) passed
    // on Linux/macOS. Root cause: Windows' default thread stack is 1 MiB,
    // smaller than the ~2 MiB a Unix `cargo test` thread gets — a
    // per-level-recursion bug in `XmlElement`'s teardown that fits inside
    // 2 MiB but not 1 MiB of stack never reproduces on a Unix runner's
    // *default* thread. Spawning our own thread with an explicitly small
    // stack reproduces the same "not enough stack for `MAX_TREE_DEPTH`
    // levels of recursive teardown" condition on any OS, so this doesn't
    // need a Windows runner to catch a regression here.
    //
    // Both-directions verification performed while fixing this (not
    // re-run by CI, recorded here for the record): with `XmlElement`'s
    // `Drop` temporarily reverted to the compiler-derived one (deleting
    // the hand-written `impl Drop for XmlElement` in `src/events.rs`),
    // `cargo test --test i3_hostile_proptest
    // i3_p0_small_stack_thread_deeply_nested_input_does_not_overflow`
    // reliably aborted the whole process across 3/3 runs ("thread
    // 'i3-small-stack' has overflowed its stack" / SIGABRT) — a stack
    // overflow is not a catchable panic, so the *process* aborts rather
    // than merely failing `handle.join()`. With the iterative `Drop`
    // restored, the same command passed cleanly across 3/3 runs.
    const N: usize = 60_000;
    let source = format!("{}{}", "<foo>".repeat(N), "</foo>".repeat(N));

    let handle = std::thread::Builder::new()
        .name("i3-small-stack".to_string())
        // Well below Windows' 1 MiB default thread stack, so a regression
        // that only breaks under *that* budget still shows up here
        // regardless of which OS actually runs this test. (512 KiB was
        // tried first per this bug's own root-cause writeup but proved
        // too close to this dev machine's actual per-frame overhead to
        // reliably overflow under a debug build; 256 KiB reproduces the
        // derived-`Drop` overflow deterministically while still comfortably
        // clearing the iterative `Drop`'s ~flat stack usage.)
        .stack_size(256 * 1024)
        .spawn(move || {
            let result = beans_xml::parse(&source);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| d.code == DiagCode::NestingLimitExceeded),
                "expected a NestingLimitExceeded diagnostic: {:?}",
                result.diagnostics
            );
            // `result` (and the `XmlElement` tree owned by its
            // `Option<XmlElement>`) drops here, at the end of this
            // small-stack thread's closure — that teardown, not the parse
            // itself, is what must stay frame-bounded regardless of the
            // tree's original nesting depth.
        })
        .expect("spawning a small-stack thread should succeed");

    handle.join().expect(
        "small-stack thread must not panic/overflow while parsing+dropping a deeply nested tree",
    );
}

#[test]
fn i3_max_input_bytes_boundary_never_panics() {
    // One byte over the cap — rejected before any decode/parse is even
    // attempted (`lib.rs::parse_bytes`'s own `>` comparison), so this stays
    // a cheap, size-check-only path. Deliberately calls `parse_bytes`/
    // `is_beans_doc` directly rather than `assert_pipeline_never_panics`
    // (which also lossy-decodes into `parse(&str)` — a 10MB+ string of
    // unclosed `<` residue has no size cap on that entry point at all and
    // would just be an expensive, off-topic recovery-rule stress test, not
    // this test's own point). Complements `tests/u2_encoding.rs`'s
    // narrower `encoding`-field-only oversize check.
    let over = vec![b'<'; beans_xml::MAX_INPUT_BYTES + 1];
    assert!(!beans_xml::is_beans_doc(&over));
    assert!(beans_xml::parse_bytes(&over).beans.is_none());
}
