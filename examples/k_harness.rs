//! M0b K-1/K-2/K-3 measurement harness (per the internal design spec's
//! "kill criteria / promotion triggers" section).
//!
//! Walks a corpus directory (path from the `BEANSXML_PUBLIC_CORPUS`
//! environment variable — never a path baked into this file, so no corpus
//! content is ever committed here), parses every `<beans>`-shaped `*.xml`
//! file it finds with this crate's own public API, and emits one JSON line
//! per bean node and per bean-to-bean edge to stdout. A companion,
//! independently-written script (kept outside this repository, per this
//! crate's own `AGENTS.md` — no scoring/diff logic lives here) derives the
//! same two record sets via a different XML stack and compares them; that
//! diff is the actual K-1/K-2/K-3 measurement. This file only extracts —
//! scoring against ground truth is deliberately a separate program, so a
//! bug in one can't quietly cancel a bug in the other (this crate's own
//! "anti-self-blessing" gate for the M0b milestone).
//!
//! Every field this harness reads is already part of the published output
//! model (`src/model.rs`) — no new parsing logic, no crate-internal seam.
//! Bean-node records use `(file, effective_name, class)`, matching K-1's own
//! bar ("`<bean>` node extraction rate ... id/names/class accurate"): only a `<bean>`
//! carrying an id or a `name=` alias is recorded (an anonymous inner bean
//! with neither has no name to match against, on either side of the diff).
//! Edge records use `(file, channel, target)`, covering every channel the
//! spec's K-2 row names as the same denominator a downstream consumer
//! draws on: `property`/`constructor-arg`
//! `Ref`/`Idref` (recursed through collections, map entries, and inner
//! beans — p/c-namespace attributes normalize into these same two fields
//! upstream, in `src/bean.rs`, so no separate channel is needed for them),
//! `parent`, `depends-on`, `factory-bean`, method injection
//! (`lookup-method`/`replaced-method`), `ValueLit::spel_refs`, and
//! `NamespacedElement::refs` (both `BeansFile::namespaced` and a bean's own
//! `decorators`) — recursively through `nested_profiles`.
//!
//! Usage: `BEANSXML_PUBLIC_CORPUS=/path/to/corpus cargo run --example
//! k_harness > /some/scratch/path/out.jsonl` — redirect stdout somewhere
//! outside this repository; this crate's own leak-safety policy
//! (the internal design spec's "fixture corpus" section) never writes corpus-derived
//! content (bean ids, class names, file contents) into the repository, only
//! this fixed extraction program.

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use beans_xml::{Bean, BeansFile, Collection, InjectValue, NamespacedElement, ParseResult};

const CORPUS_ENV_VAR: &str = "BEANSXML_PUBLIC_CORPUS";
const NEEDLE: &[u8] = b"springframework.org/schema/beans";

fn main() {
    let corpus_root = match env::var(CORPUS_ENV_VAR) {
        Ok(path) => PathBuf::from(path),
        Err(_) => {
            eprintln!("k_harness: set {CORPUS_ENV_VAR} to the corpus directory before running");
            std::process::exit(2);
        }
    };

    let mut xml_files = Vec::new();
    collect_xml_files(&corpus_root, &mut xml_files);
    xml_files.sort();

    let mut candidates = 0u64;
    let mut beans_some = 0u64;
    let mut beans_none = 0u64;
    let mut bean_records = 0u64;
    let mut edge_records = 0u64;
    let mut read_errors = 0u64;

    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    for path in &xml_files {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) => {
                eprintln!("k_harness: failed to read {}: {err}", path.display());
                read_errors += 1;
                continue;
            }
        };
        if !contains_needle(&bytes, NEEDLE) {
            continue;
        }
        candidates += 1;

        let rel = relative_path(&corpus_root, path);
        let ParseResult { beans, .. } = beans_xml::parse_bytes(&bytes);
        match beans {
            Some(beans_file) => {
                beans_some += 1;
                walk_beans_file(
                    &beans_file,
                    &rel,
                    &mut out,
                    &mut bean_records,
                    &mut edge_records,
                );
            }
            None => beans_none += 1,
        }
    }

    out.flush().expect("stdout flush");

    eprintln!(
        "k_harness: xml_files={} candidates={} beans_some={} beans_none={} \
         read_errors={} bean_records={} edge_records={}",
        xml_files.len(),
        candidates,
        beans_some,
        beans_none,
        read_errors,
        bean_records,
        edge_records
    );
}

/// Recursively collects every `*.xml` file under `dir` into `out`, skipping
/// unreadable directory entries rather than aborting the whole scan (this is
/// a measurement tool over a third-party corpus, not the crate under test —
/// leniency here is a convenience, not a contract).
fn collect_xml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_xml_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("xml") {
            out.push(path);
        }
    }
}

/// Byte-substring search (no allocation, no encoding assumptions) — `NEEDLE`
/// is pure ASCII, so it is found intact regardless of the file's declared
/// or actual encoding (EUC-KR/CP949 are ASCII-compatible for byte values
/// below 0x80, and this crate's own encoding detection runs later, only on
/// files this cheap pre-filter already selected).
fn contains_needle(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Path relative to the corpus root, forward-slash separated, for a stable
/// cross-platform key in the emitted records — falls back to the file name
/// alone in the (unexpected) case `path` isn't actually under `root`.
fn relative_path(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn emit_bean(out: &mut impl Write, file: &str, name: &str, class: Option<&str>, count: &mut u64) {
    let record = serde_json::json!({
        "kind": "bean",
        "file": file,
        "name": name,
        "class": class,
    });
    writeln!(out, "{record}").expect("stdout write");
    *count += 1;
}

fn emit_edge(out: &mut impl Write, file: &str, channel: &str, target: &str, count: &mut u64) {
    let record = serde_json::json!({
        "kind": "edge",
        "file": file,
        "channel": channel,
        "target": target,
    });
    writeln!(out, "{record}").expect("stdout write");
    *count += 1;
}

/// One `<beans>` document — top-level or a `nested_profiles` block — walked
/// into bean/edge records. Recurses into `nested_profiles` itself (SB-14),
/// so a `<beans profile="...">` block nested arbitrarily deep is covered by
/// the same call, never a second copy of this walk.
fn walk_beans_file(
    beans_file: &BeansFile,
    file: &str,
    out: &mut impl Write,
    bean_count: &mut u64,
    edge_count: &mut u64,
) {
    for bean in &beans_file.beans {
        walk_bean(bean, file, out, bean_count, edge_count);
    }
    for namespaced in &beans_file.namespaced {
        walk_namespaced(namespaced, file, out, edge_count);
    }
    for nested in &beans_file.nested_profiles {
        walk_beans_file(nested, file, out, bean_count, edge_count);
    }
}

/// One `<bean>` — top-level or an anonymous `InjectValue::Inner` reached
/// through a property/constructor-arg/collection value — walked into its
/// own bean record (only when it carries an id or a `name=` alias — an
/// anonymous inner bean has no effective name to match against) plus every
/// edge channel K-2's denominator names.
fn walk_bean(
    bean: &Bean,
    file: &str,
    out: &mut impl Write,
    bean_count: &mut u64,
    edge_count: &mut u64,
) {
    let effective_name = bean
        .id
        .as_ref()
        .map(|s| s.value.as_str())
        .or_else(|| bean.names.first().map(|s| s.value.as_str()));
    if let Some(name) = effective_name {
        let class = bean.class.as_ref().map(|c| c.value.raw.as_str());
        emit_bean(out, file, name, class, bean_count);
    }

    if let Some(parent) = &bean.parent {
        emit_edge(out, file, "parent", &parent.value.raw, edge_count);
    }
    for dep in &bean.depends_on {
        emit_edge(out, file, "depends_on", &dep.value.raw, edge_count);
    }
    if let Some(factory_bean) = &bean.factory_bean {
        emit_edge(
            out,
            file,
            "factory_bean",
            &factory_bean.value.raw,
            edge_count,
        );
    }
    for lookup_method in &bean.lookup_methods {
        if let Some(target) = &lookup_method.value.bean {
            emit_edge(out, file, "lookup_method", &target.value.raw, edge_count);
        }
    }
    for replaced_method in &bean.replaced_methods {
        if let Some(target) = &replaced_method.value.replacer {
            emit_edge(out, file, "replaced_method", &target.value.raw, edge_count);
        }
    }
    for property in &bean.properties {
        walk_inject_value(
            &property.value,
            file,
            "property_ref",
            out,
            bean_count,
            edge_count,
        );
    }
    for constructor_arg in &bean.constructor_args {
        walk_inject_value(
            &constructor_arg.value,
            file,
            "ctor_ref",
            out,
            bean_count,
            edge_count,
        );
    }
    for decorator in &bean.decorators {
        walk_namespaced(decorator, file, out, edge_count);
    }
}

/// One `InjectValue` — a `<property>`/`<constructor-arg>`'s own value, a
/// collection item, or a map entry key/value — walked for `Ref`/`Idref`
/// edges (tagged with the caller's `channel`: `"property_ref"` or
/// `"ctor_ref"`, the same channel all the way down through nested
/// collections/inner beans, since the spec's K-2 denominator doesn't
/// subdivide further than "which top-level wiring point this edge hangs
/// off of"), `ValueLit::spel_refs` (its own `"spel_ref"` channel,
/// independent of `channel`), and inner beans (recursing back into
/// [`walk_bean`] — same "recursion unification" shape this crate's own parser
/// follows for `InjectValue::Inner`).
fn walk_inject_value(
    value: &InjectValue,
    file: &str,
    channel: &str,
    out: &mut impl Write,
    bean_count: &mut u64,
    edge_count: &mut u64,
) {
    match value {
        InjectValue::Ref(bean_ref) | InjectValue::Idref(bean_ref) => {
            emit_edge(out, file, channel, &bean_ref.value.raw, edge_count);
        }
        InjectValue::Value(value_lit) => {
            for spel_ref in &value_lit.spel_refs {
                emit_edge(out, file, "spel_ref", &spel_ref.value, edge_count);
            }
        }
        InjectValue::Inner(inner_bean) => {
            walk_bean(inner_bean, file, out, bean_count, edge_count);
        }
        InjectValue::Collection(collection) => {
            walk_collection(
                &collection.value,
                file,
                channel,
                out,
                bean_count,
                edge_count,
            );
        }
        // `Null` plus the forward-compat `Unrecognized` fallback (and any
        // future `#[non_exhaustive]` variant this build doesn't know about
        // yet) carry no ref-shaped edge.
        _ => {}
    }
}

/// A `Collection` — `list`/`set`/`array` items, `map` entry keys/values, or
/// `props` entries — walked the same way `parse_collection_value` builds
/// each shape, recursing back into [`walk_inject_value`] for every item/
/// key/value (which is itself what reaches a nested collection or an inner
/// bean arbitrarily deep — `quartz`'s own `<list><ref>` triggers shape,
/// named directly in the spec's edge-set definition, is exactly this path).
fn walk_collection(
    collection: &Collection,
    file: &str,
    channel: &str,
    out: &mut impl Write,
    bean_count: &mut u64,
    edge_count: &mut u64,
) {
    match collection {
        Collection::List { items, .. }
        | Collection::Set { items, .. }
        | Collection::Array { items, .. } => {
            for item in items {
                walk_inject_value(item, file, channel, out, bean_count, edge_count);
            }
        }
        Collection::Map { entries, .. } => {
            for entry in entries {
                walk_inject_value(&entry.key, file, channel, out, bean_count, edge_count);
                walk_inject_value(&entry.value, file, channel, out, bean_count, edge_count);
            }
        }
        Collection::Props { entries, .. } => {
            for entry in entries {
                for spel_ref in &entry.value.spel_refs {
                    emit_edge(out, file, "spel_ref", &spel_ref.value, edge_count);
                }
            }
        }
        // The forward-compat `Unrecognized` fallback (and any future
        // `#[non_exhaustive]` variant) carries no items/entries to walk.
        _ => {}
    }
}

/// A preserved `NamespacedElement` (`BeansFile::namespaced` top-level, or a
/// `Bean::decorators` entry) — its own `refs` (the `NS_REF_ALLOWLIST`
/// harvest `src/namespaced.rs` already recursed through the whole subtree
/// to collect) becomes one `"ns_ref"` edge per entry. No further recursion
/// happens here: `NamespacedElement` has no nested `Vec<NamespacedElement>`
/// of its own — the parser's own recursive harvest is already flat by the
/// time it reaches this model.
fn walk_namespaced(
    namespaced: &NamespacedElement,
    file: &str,
    out: &mut impl Write,
    edge_count: &mut u64,
) {
    for bean_ref in &namespaced.refs {
        emit_edge(out, file, "ns_ref", &bean_ref.value.raw, edge_count);
    }
}
