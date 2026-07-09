//! Spec DoD: parse a 1 MB `<beans>` document in < 50 ms (criterion, local
//! baseline; the spec's target notes "large Quartz included" -- this
//! generator's per-bean shape mirrors an eGov-style archetype: a class,
//! `depends-on`, a `<list>`/`<map>` of properties mixing `ref`/`value`, and
//! a nested inner bean, repeated until the target size is reached).
//!
//! A second case ([`bench_parse_1mb_quartz`]) covers the spec target's
//! other named shape directly: a large Quartz scheduler context -- many
//! `JobDetailFactoryBean`/`CronTriggerFactoryBean`-style bean pairs (a
//! literal `jobClass` value per job, distinct from the shared factory
//! class every job/trigger bean instance itself uses) plus one big
//! `<property name="triggers"><list><ref .../>...` block wiring every
//! trigger into a single `SchedulerFactoryBean`-style bean -- a
//! `ref`-list-heavy shape the first case's own inner-bean/map-heavy shape
//! doesn't exercise (long flat `<list>` of `<ref>` children scanned once
//! per bean rather than nested per-item structure).
//!
//! Every class name in both generators is synthetic `com.example.*` (this
//! crate's own fixture-corpus leak-safety convention, `AGENTS.md` §5/§4) --
//! including the Quartz-shaped ones, which deliberately do **not** use the
//! real `org.springframework.scheduling.quartz.*` class names so this
//! synthetic input carries no real framework/company specifics beyond the
//! bean *shape* itself.
//!
//! The input is generated deterministically (no `proptest` randomness) so
//! the benchmark is reproducible run to run. It's a fixed string, not
//! committed to the repo (regenerated on every `cargo bench`), matching
//! batis-xml's sibling `benches/parse_1mb.rs` in shape and intent.

use beans_xml::parse;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::fmt::Write as _;

const BEAN_TEMPLATE: &str = r#"<bean id="widgetService{i}" class="com.example.service.WidgetService{i}" depends-on="dataSource">
    <property name="label" value="Widget {i}"/>
    <property name="repository" ref="widgetRepository{i}"/>
    <property name="tags">
        <list>
            <value>tag-a-{i}</value>
            <value>tag-b-{i}</value>
            <ref bean="tagResolver"/>
        </list>
    </property>
    <property name="settings">
        <map>
            <entry key="timeout" value="30"/>
            <entry key="cache" value-ref="cacheManager"/>
        </map>
    </property>
    <property name="fallback">
        <bean class="com.example.service.NullWidgetService{i}">
            <property name="reason" value="fallback-{i}"/>
        </bean>
    </property>
</bean>
<bean id="widgetRepository{i}" class="com.example.repository.WidgetRepository{i}"/>
"#;

/// Builds a `<beans>` document of at least `target_bytes` bytes by
/// repeating a fixed bean shape (core attrs, property list, property map,
/// nested inner bean) with an incrementing id/class suffix.
fn generate_beans(target_bytes: usize) -> String {
    let mut out = String::from(
        r#"<beans xmlns="http://www.springframework.org/schema/beans">
<bean id="dataSource" class="com.example.jdbc.DataSource"/>
<bean id="tagResolver" class="com.example.service.TagResolver"/>
<bean id="cacheManager" class="com.example.cache.CacheManager"/>
"#,
    );
    let mut i = 0usize;
    while out.len() < target_bytes {
        out.push_str(&BEAN_TEMPLATE.replace("{i}", &i.to_string()));
        i += 1;
    }
    out.push_str("</beans>");
    out
}

fn bench_parse_1mb(c: &mut Criterion) {
    let source = generate_beans(1_000_000);
    eprintln!(
        "parse_1mb: generated document is {} bytes, {} beans",
        source.len(),
        source.matches("<bean ").count()
    );

    let mut group = c.benchmark_group("parse_1mb_beans");
    group.bench_function("parse", |b| {
        b.iter(|| black_box(parse(black_box(&source))));
    });
    group.finish();
}

/// One job/trigger pair -- `{i}` is the incrementing suffix
/// [`generate_quartz_beans`] substitutes on every repetition. Both bean
/// instances share one synthetic factory `class=` (real Quartz/Spring XML's
/// own shape: the factory class is fixed, only the `id=` and its properties
/// vary per job), with the per-job specifics carried in `jobClass`/
/// `cronExpression` property *values* instead -- exactly where they'd sit
/// in a real Quartz scheduler context.
const QUARTZ_JOB_TRIGGER_TEMPLATE: &str = r#"<bean id="job{i}" class="com.example.scheduler.JobDetailFactoryBean">
    <property name="jobClass" value="com.example.jobs.SampleJob{i}"/>
    <property name="durability" value="true"/>
    <property name="jobDataAsMap">
        <map>
            <entry key="batchSize" value="10"/>
            <entry key="retryCount" value="3"/>
        </map>
    </property>
</bean>
<bean id="trigger{i}" class="com.example.scheduler.CronTriggerFactoryBean">
    <property name="jobDetail" ref="job{i}"/>
    <property name="cronExpression" value="0 0 {i} * * ?"/>
    <property name="misfireInstructionName" value="MISFIRE_INSTRUCTION_DO_NOTHING"/>
</bean>
"#;

/// Builds a `<beans>` document of at least `target_bytes` bytes shaped like
/// a large Quartz scheduler context: repeated `JobDetailFactoryBean`/
/// `CronTriggerFactoryBean`-style pairs
/// ([`QUARTZ_JOB_TRIGGER_TEMPLATE`]), followed by one
/// `SchedulerFactoryBean`-style bean whose `triggers` property is a single
/// `<list>` of `<ref bean="trigger{i}"/>` covering every trigger just
/// generated -- the "big triggers `<list><ref>` block" the spec's own
/// Quartz-shaped bench target calls out, distinct from the first case's
/// per-bean nested-map/inner-bean shape.
fn generate_quartz_beans(target_bytes: usize) -> String {
    let mut out = String::from(
        r#"<beans xmlns="http://www.springframework.org/schema/beans">
<bean id="dataSource" class="com.example.jdbc.DataSource"/>
"#,
    );
    let mut i = 0usize;
    let mut trigger_ids: Vec<String> = Vec::new();
    while out.len() < target_bytes {
        out.push_str(&QUARTZ_JOB_TRIGGER_TEMPLATE.replace("{i}", &i.to_string()));
        trigger_ids.push(format!("trigger{i}"));
        i += 1;
    }

    out.push_str(
        "<bean id=\"scheduler\" class=\"com.example.scheduler.SchedulerFactoryBean\">\n\
         \x20   <property name=\"triggers\">\n        <list>\n",
    );
    for id in &trigger_ids {
        let _ = writeln!(out, "            <ref bean=\"{id}\"/>");
    }
    out.push_str("        </list>\n    </property>\n</bean>\n</beans>");
    out
}

fn bench_parse_1mb_quartz(c: &mut Criterion) {
    let source = generate_quartz_beans(1_000_000);
    eprintln!(
        "parse_1mb_quartz: generated document is {} bytes, {} beans, {} triggers in scheduler list",
        source.len(),
        source.matches("<bean ").count(),
        source.matches("<ref bean=\"trigger").count()
    );

    let mut group = c.benchmark_group("parse_1mb_quartz_scheduler");
    group.bench_function("parse", |b| {
        b.iter(|| black_box(parse(black_box(&source))));
    });
    group.finish();
}

criterion_group!(benches, bench_parse_1mb, bench_parse_1mb_quartz);
criterion_main!(benches);
