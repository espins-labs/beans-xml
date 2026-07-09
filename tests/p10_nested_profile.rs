//! Unit **P10** — nested `<beans profile="...">` (SB-14). Test design per
//! the internal build plan's P10 row ("test: table (multiple/negated/
//! boolean-expression raw · sibling override)") plus the task's own case: a nested block's
//! content (beans/imports/...) is fully populated by re-entering the shared
//! `parse_beans_body`, not reimplemented here.
//!
//! `dispatch::parse_nested_beans`/`parse_beans_body` are `pub(crate)` — not
//! visible from this external integration-test binary — so every test here
//! goes through the public API (`beans_xml::parse`) only, the same
//! convention `tests/p1_alias.rs`/`tests/p3_import.rs` established.

use beans_xml::{BeansFile, ImportKind};

fn parse_ok(source: &str) -> BeansFile {
    beans_xml::parse(source).beans.expect("beans root")
}

// ---------------------------------------------------------------------
// Table: profile-expression raw preservation (multi-value / negation /
// boolean forms) — never parsed as boolean logic, just captured verbatim.
// ---------------------------------------------------------------------

#[test]
fn sb14_multi_value_profile_is_preserved_raw() {
    let source = r#"<beans><beans profile="dev,test"></beans></beans>"#;
    let beans = parse_ok(source);
    assert_eq!(beans.nested_profiles.len(), 1);
    let nested = &beans.nested_profiles[0];
    assert_eq!(
        nested.profile.as_ref().map(|p| p.value.as_str()),
        Some("dev,test")
    );
}

#[test]
fn sb14_negated_profile_is_preserved_raw() {
    let source = r#"<beans><beans profile="!prod"></beans></beans>"#;
    let beans = parse_ok(source);
    assert_eq!(beans.nested_profiles.len(), 1);
    let nested = &beans.nested_profiles[0];
    assert_eq!(
        nested.profile.as_ref().map(|p| p.value.as_str()),
        Some("!prod")
    );
}

#[test]
fn sb14_boolean_expression_profile_is_preserved_raw() {
    // Spring's `Profiles.of` grammar accepts full boolean expressions —
    // this crate never evaluates/parses that grammar (spec's "SpEL/`${}`
    // **evaluation** (collection only)" non-goal, same policy applied to profile expressions),
    // it just carries the text through verbatim. Attribute values are
    // never entity-resolved either way (`XmlAttr`'s own doc comment,
    // invariant #4) so `|` needs no escaping here.
    let source = r#"<beans><beans profile="(dev &amp; !prod) | qa"></beans></beans>"#;
    let beans = parse_ok(source);
    assert_eq!(beans.nested_profiles.len(), 1);
    let nested = &beans.nested_profiles[0];
    assert_eq!(
        nested.profile.as_ref().map(|p| p.value.as_str()),
        Some("(dev &amp; !prod) | qa")
    );
}

#[test]
fn sb14_single_profile_is_preserved_raw() {
    let source = r#"<beans><beans profile="dev"></beans></beans>"#;
    let beans = parse_ok(source);
    assert_eq!(beans.nested_profiles.len(), 1);
    let nested = &beans.nested_profiles[0];
    assert_eq!(
        nested.profile.as_ref().map(|p| p.value.as_str()),
        Some("dev")
    );
}

// ---------------------------------------------------------------------
// Sibling-profile override, not DuplicateBeanId.
// ---------------------------------------------------------------------

#[test]
fn sb14_same_bean_id_across_sibling_profile_blocks_is_not_duplicate_bean_id() {
    let source = concat!(
        "<beans>",
        r#"<beans profile="dev">"#,
        r#"<bean id="dataSource" class="com.example.DevDataSource"/>"#,
        "</beans>",
        r#"<beans profile="test">"#,
        r#"<bean id="dataSource" class="com.example.TestDataSource"/>"#,
        "</beans>",
        "</beans>"
    );
    let result = beans_xml::parse(source);
    // No DuplicateBeanId — each sibling nested profile block has its own
    // independent `beans` list, this is override territory, not the
    // single-`<beans>`-block duplicate-id case the spec's `DuplicateBeanId`
    // doc comment scopes to.
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|d| matches!(d.code, beans_xml::DiagCode::DuplicateBeanId)),
        "unexpected DuplicateBeanId diagnostic: {:?}",
        result.diagnostics
    );
    let beans = result.beans.expect("beans root");
    assert_eq!(beans.nested_profiles.len(), 2);
    assert_eq!(beans.nested_profiles[0].beans.len(), 1);
    assert_eq!(beans.nested_profiles[1].beans.len(), 1);
    assert_eq!(
        beans.nested_profiles[0].beans[0]
            .id
            .as_ref()
            .map(|s| s.value.as_str()),
        Some("dataSource")
    );
    assert_eq!(
        beans.nested_profiles[0].beans[0]
            .class
            .as_ref()
            .map(|c| c.value.raw.as_str()),
        Some("com.example.DevDataSource")
    );
    assert_eq!(
        beans.nested_profiles[1].beans[0]
            .id
            .as_ref()
            .map(|s| s.value.as_str()),
        Some("dataSource")
    );
    assert_eq!(
        beans.nested_profiles[1].beans[0]
            .class
            .as_ref()
            .map(|c| c.value.raw.as_str()),
        Some("com.example.TestDataSource")
    );
}

#[test]
fn sb14_same_bean_id_in_nested_profile_and_top_level_is_not_duplicate_bean_id() {
    // A nested profile block's `ctx.beans` is entirely independent from the
    // enclosing (parent) `<beans>` block's `ctx.beans` too, same reasoning
    // as the sibling case above — `DuplicateBeanId` is scoped to a single
    // `<beans>` block only.
    let source = concat!(
        "<beans>",
        r#"<bean id="dataSource" class="com.example.DefaultDataSource"/>"#,
        r#"<beans profile="dev">"#,
        r#"<bean id="dataSource" class="com.example.DevDataSource"/>"#,
        "</beans>",
        "</beans>"
    );
    let result = beans_xml::parse(source);
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|d| matches!(d.code, beans_xml::DiagCode::DuplicateBeanId)),
        "unexpected DuplicateBeanId diagnostic: {:?}",
        result.diagnostics
    );
}

#[test]
fn sb14_duplicate_bean_id_within_the_same_nested_profile_block_is_still_flagged() {
    // The `DuplicateBeanId` check itself still fires *inside* one nested
    // block — P10 doesn't suppress it there, only across sibling/parent
    // boundaries.
    let source = concat!(
        "<beans>",
        r#"<beans profile="dev">"#,
        r#"<bean id="dataSource" class="com.example.DevDataSourceOne"/>"#,
        r#"<bean id="dataSource" class="com.example.DevDataSourceTwo"/>"#,
        "</beans>",
        "</beans>"
    );
    let result = beans_xml::parse(source);
    let dup_count = result
        .diagnostics
        .iter()
        .filter(|d| matches!(d.code, beans_xml::DiagCode::DuplicateBeanId))
        .count();
    assert_eq!(dup_count, 1);
    let beans = result.beans.expect("beans root");
    assert_eq!(beans.nested_profiles[0].beans.len(), 2, "both preserved");
}

// ---------------------------------------------------------------------
// Nested content is fully populated (beans, imports, ...) via re-entering
// the shared parse_beans_body — not reimplemented by this unit.
// ---------------------------------------------------------------------

#[test]
fn sb14_nested_block_content_is_fully_populated() {
    let source = concat!(
        "<beans>",
        r#"<beans profile="dev">"#,
        r#"<import resource="classpath:com/example/dev-context.xml"/>"#,
        r#"<alias name="dataSource" alias="ds"/>"#,
        r#"<bean id="dataSource" class="com.example.DevDataSource">"#,
        r#"<property name="url" value="jdbc:dev"/>"#,
        "</bean>",
        "</beans>",
        "</beans>"
    );
    let beans = parse_ok(source);
    assert_eq!(beans.beans.len(), 0, "top-level has no direct <bean>");
    assert_eq!(beans.nested_profiles.len(), 1);
    let nested = &beans.nested_profiles[0];
    assert_eq!(
        nested.profile.as_ref().map(|p| p.value.as_str()),
        Some("dev")
    );

    assert_eq!(nested.imports.len(), 1);
    assert_eq!(nested.imports[0].value.kind, ImportKind::Classpath);
    assert_eq!(
        nested.imports[0].value.resource.value,
        "classpath:com/example/dev-context.xml"
    );

    assert_eq!(nested.aliases.len(), 1);
    assert_eq!(nested.aliases[0].value.name.value, "dataSource");
    assert_eq!(nested.aliases[0].value.alias.value, "ds");

    assert_eq!(nested.beans.len(), 1);
    let bean = &nested.beans[0];
    assert_eq!(
        bean.id.as_ref().map(|s| s.value.as_str()),
        Some("dataSource")
    );
    assert_eq!(
        bean.class.as_ref().map(|c| c.value.raw.as_str()),
        Some("com.example.DevDataSource")
    );
    assert_eq!(bean.properties.len(), 1);
    assert_eq!(bean.properties[0].name.value, "url");
}

#[test]
fn sb14_nested_block_can_itself_contain_a_further_nested_profile() {
    // Recursion is bounded by `crate::DEPTH_LIMIT` (see the
    // `sb14_depth_limit_*` tests below) since `parse_nested_beans`
    // re-enters the same shared `parse_beans_body`, which checks depth
    // before recursing further — a profile block nested inside another
    // profile block is not a distinct code path.
    let source = concat!(
        "<beans>",
        r#"<beans profile="dev">"#,
        r#"<beans profile="local">"#,
        r#"<bean id="dataSource" class="com.example.LocalDevDataSource"/>"#,
        "</beans>",
        "</beans>",
        "</beans>"
    );
    let beans = parse_ok(source);
    assert_eq!(beans.nested_profiles.len(), 1);
    let dev = &beans.nested_profiles[0];
    assert_eq!(dev.profile.as_ref().map(|p| p.value.as_str()), Some("dev"));
    assert_eq!(dev.nested_profiles.len(), 1);
    let local = &dev.nested_profiles[0];
    assert_eq!(
        local.profile.as_ref().map(|p| p.value.as_str()),
        Some("local")
    );
    assert_eq!(local.beans.len(), 1);
    assert_eq!(
        local.beans[0].id.as_ref().map(|s| s.value.as_str()),
        Some("dataSource")
    );
}

#[test]
fn sb14_nested_block_span_covers_the_nested_beans_element() {
    let source =
        r#"<beans><beans profile="dev"><bean id="x" class="com.example.X"/></beans></beans>"#;
    let beans = parse_ok(source);
    let nested = &beans.nested_profiles[0];
    let slice = &source[nested.span.start as usize..nested.span.end as usize];
    assert_eq!(
        slice,
        r#"<beans profile="dev"><bean id="x" class="com.example.X"/></beans>"#
    );
}

// ---------------------------------------------------------------------
// DEPTH_LIMIT — `<beans>`-in-`<beans>` recursion (SB-16 "infinite nesting →
// must not panic" / invariant #1). `parse_nested_beans` re-enters
// `parse_beans_body`, a genuine native call-stack recursion cycle
// (`parse_beans_body` → `dispatch_root_child` → `parse_nested_beans` →
// `parse_beans_body`) — same shape as `inject_value::parse_inner_bean`'s
// and `collection::parse_collection_value`'s own DEPTH_LIMIT-guarded
// recursion, so it must be bounded the same way. These tests only reach
// `parse_beans_body`/`parse_nested_beans` through the public `parse` API
// (they're `pub(crate)`, per this file's own header note), so the
// boundary is exercised structurally — an actual chain of N nested
// `<beans>` elements — rather than by injecting a `depth` value directly.
// ---------------------------------------------------------------------

/// Builds `n` levels of `<beans profile="pK">` nested one inside the next,
/// with a single `<bean id="deepest" .../>` at the very center.
fn nested_beans_chain(n: u32) -> String {
    let mut source = String::from("<beans>");
    for i in 0..n {
        source.push_str(&format!(r#"<beans profile="p{i}">"#));
    }
    source.push_str(r#"<bean id="deepest" class="com.example.Deepest"/>"#);
    for _ in 0..n {
        source.push_str("</beans>");
    }
    source.push_str("</beans>");
    source
}

#[test]
fn sb14_depth_one_below_limit_recurses_fully_with_no_diagnostic() {
    // DEPTH_LIMIT - 1 nested levels: the innermost `parse_beans_body` call
    // runs at `depth == DEPTH_LIMIT - 1`, strictly below the guard, so the
    // whole chain — including the deepest bean — is genuinely walked.
    let n = beans_xml::DEPTH_LIMIT - 1;
    let source = nested_beans_chain(n);
    let result = beans_xml::parse(&source);
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|d| matches!(d.code, beans_xml::DiagCode::NestingLimitExceeded)),
        "unexpected NestingLimitExceeded at depth {}: {:?}",
        n,
        result.diagnostics
    );
    let mut current = result.beans.expect("beans root");
    for _ in 0..n {
        assert_eq!(current.nested_profiles.len(), 1);
        current = current.nested_profiles.into_iter().next().unwrap();
    }
    assert_eq!(current.beans.len(), 1, "the deepest bean must be reached");
    assert_eq!(
        current.beans[0].id.as_ref().map(|s| s.value.as_str()),
        Some("deepest")
    );
}

#[test]
fn sb14_depth_at_and_beyond_limit_downgrades_to_nesting_limit_exceeded_not_a_crash() {
    // DEPTH_LIMIT + 5 nested levels: the walk must stop at DEPTH_LIMIT
    // rather than recursing further (and, pre-fix, overflowing the native
    // call stack) — a bounded `NestingLimitExceeded` diagnostic instead of
    // a crash, per SB-16 and invariant #1 ("no panics/infinite loops on any
    // byte input ... depth-256 hard constant").
    let n = beans_xml::DEPTH_LIMIT + 5;
    let source = nested_beans_chain(n);
    let result = beans_xml::parse(&source); // must not panic/abort
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| matches!(d.code, beans_xml::DiagCode::NestingLimitExceeded)),
        "expected a NestingLimitExceeded diagnostic beyond depth {}",
        beans_xml::DEPTH_LIMIT
    );

    // Walk the chain that *did* get built: exactly DEPTH_LIMIT nested
    // `BeansFile`s deep (levels checked at parse_beans_body's own
    // `depth` 1..=DEPTH_LIMIT), the last of which is the empty stub the
    // guard returns instead of recursing into its own child.
    let mut current = result.beans.expect("beans root");
    for level in 1..=beans_xml::DEPTH_LIMIT {
        assert_eq!(
            current.nested_profiles.len(),
            1,
            "expected one more nested level at {level}"
        );
        current = current.nested_profiles.into_iter().next().unwrap();
    }
    assert!(
        current.nested_profiles.is_empty(),
        "the over-limit level must not recurse into its own nested <beans> child"
    );
    assert!(
        current.beans.is_empty(),
        "the over-limit level must not see the deepest <bean> at all"
    );
}

#[test]
fn sb14_deeply_nested_input_never_panics_or_overflows_the_stack() {
    // Regression test for the cold-review finding: a chain of nested
    // `<beans>` blocks, well within MAX_INPUT_BYTES and far past
    // DEPTH_LIMIT, that previously drove *unbounded* native call-stack
    // recursion through `parse_beans_body` <-> `parse_nested_beans`
    // (SIGABRT) because that cycle carried no depth counter. With the
    // depth guard in place, this cycle's own recursion is bounded at
    // `DEPTH_LIMIT` regardless of how deep the input nests, so
    // `beans_xml::parse` must return normally here.
    //
    // NOTE: 2_000 is chosen to comfortably exercise this fix (far past
    // DEPTH_LIMIT=256) while staying well clear of a separate, unrelated
    // stack-depth ceiling: dropping an extremely deep `XmlElement`/
    // `XmlNode` tree (U1's `events.rs` output, built *before* this unit's
    // dispatch even runs) uses the compiler-derived recursive `Drop`
    // glue, which is its own still-unbounded recursion independent of
    // this fix — reproducible with plain deeply-nested non-`<beans>`
    // elements too (confirmed separately), so it is not this unit's
    // recursion cycle and is out of scope here; it would need a fix in
    // the U1 tree representation itself, and it's also sensitive to the
    // calling thread's stack size (e.g. `cargo test`'s ~2MiB worker-thread
    // default vs. a larger main-thread stack), unlike this unit's own now
    // input-size-independent recursion.
    let source = nested_beans_chain(2_000);
    let result = beans_xml::parse(&source);
    assert!(result
        .diagnostics
        .iter()
        .any(|d| matches!(d.code, beans_xml::DiagCode::NestingLimitExceeded)));
}

#[test]
fn sb14_multiple_sibling_profile_blocks_are_all_collected_in_order() {
    let source = concat!(
        "<beans>",
        r#"<beans profile="dev"></beans>"#,
        r#"<beans profile="test"></beans>"#,
        r#"<beans profile="!prod"></beans>"#,
        "</beans>"
    );
    let beans = parse_ok(source);
    assert_eq!(beans.nested_profiles.len(), 3);
    let profiles: Vec<&str> = beans
        .nested_profiles
        .iter()
        .map(|p| p.profile.as_ref().map(|s| s.value.as_str()).unwrap_or(""))
        .collect();
    assert_eq!(profiles, vec!["dev", "test", "!prod"]);
}
