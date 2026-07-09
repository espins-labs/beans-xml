//! Prints, for one or more `<beans>` XML files, every bean-to-bean edge
//! found in the document as a Graphviz DOT digraph: `ref`/`idref`/SpEL
//! `#{beanRef}` candidates recursed through collection items, map
//! entries, and inner (anonymous) beans, plus `parent`/`depends-on`/
//! `factory-bean`/lookup-method/replaced-method targets and namespaced
//! (`aop:`/`tx:`/`task:`/`jee:`/...) allowlisted `ref`-shaped attributes.
//!
//! References are printed **raw, unresolved** -- this crate never decides
//! whether a `ref="foo"` target actually exists (that's the consumer's
//! job); a leading `&` (FactoryBean-object marker) is stripped for
//! display since it names the same bean id as the un-prefixed form.
//!
//! Anonymous-bean/namespaced-element node keys are qualified by their
//! source file (the path given on the command line): `main` merges edges
//! from every file passed on the command line into one graph, and two
//! different files can easily contain a byte-for-byte identical anonymous
//! `<bean>` (or unqualified `<util:list>`/...) at the same span -- without
//! the file qualifier those would collide into one merged node instead of
//! staying the two distinct beans they actually are.
//!
//! Run with: cargo run --example dep_graph_dot -- path/to/applicationContext.xml [more paths...] > graph.dot

use beans_xml::{Bean, BeansFile, Collection, InjectValue, NamespacedElement};
use std::env;
use std::fmt::Write as _;
use std::fs;

/// Stable node id for a bean: its `id` when present (ids are this crate's
/// (and Spring's) own cross-file join key, so they stay unqualified), else
/// a `file`+span-keyed placeholder for the (common) anonymous inner-bean
/// case -- inner beans have no id of their own, so this keys them by their
/// source file and span instead (see this module's own doc comment for why
/// the file qualifier matters once more than one file's edges are merged).
fn node_name(bean: &Bean, file: &str) -> String {
    match &bean.id {
        Some(id) => id.value.clone(),
        None => format!("$anon@{file}:{}-{}", bean.span.start, bean.span.end),
    }
}

/// `ref="&factoryBean"` targets the `FactoryBean` object itself, not the
/// product it creates -- same underlying bean id either way, so strip the
/// marker for display. This crate's model stores `BeanRef.raw` **raw,
/// unresolved** XML text (invariant #4 -- entity decoding is a downstream
/// concern, not this crate's), so the well-formed-XML spelling of that
/// marker is the *entity* form `&amp;factoryBean`, not a bare
/// `&factoryBean` (which isn't well-formed XML, though this crate's own
/// recovery rule 5 keeps it verbatim rather than rejecting it). Strip the
/// entity form first: stripping a bare leading `&` from `&amp;factoryBean`
/// would wrongly yield the garbage node `amp;factoryBean`.
fn strip_factory_marker(raw: &str) -> &str {
    raw.strip_prefix("&amp;")
        .or_else(|| raw.strip_prefix('&'))
        .unwrap_or(raw)
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn edge(out: &mut Vec<(String, String)>, from: &str, to: &str) {
    out.push((from.to_string(), strip_factory_marker(to).to_string()));
}

/// Recurses into an `InjectValue`, emitting an edge from `src` for every
/// `ref`/`idref`/SpEL bean-reference candidate found -- diving through
/// collection items, map entries, and inner beans (each of which becomes
/// its own source node for edges of its own, in addition to the edge from
/// `src` to the inner bean itself). `file` is threaded through purely to
/// qualify an inner bean's own anonymous node key, should it turn out to be
/// anonymous itself (`node_name`'s own doc comment).
fn collect_inject_value_edges(
    src: &str,
    value: &InjectValue,
    file: &str,
    out: &mut Vec<(String, String)>,
) {
    match value {
        InjectValue::Ref(r) => edge(out, src, &r.value.raw),
        InjectValue::Idref(r) => edge(out, src, &r.value.raw),
        InjectValue::Value(lit) => {
            for spel in &lit.spel_refs {
                edge(out, src, &spel.value);
            }
        }
        InjectValue::Inner(inner) => {
            let inner_name = node_name(inner, file);
            edge(out, src, &inner_name);
            collect_bean_edges(inner, &inner_name, file, out);
        }
        InjectValue::Collection(c) => match &c.value {
            Collection::List { items, .. }
            | Collection::Set { items, .. }
            | Collection::Array { items, .. } => {
                for item in items {
                    collect_inject_value_edges(src, item, file, out);
                }
            }
            Collection::Map { entries, .. } => {
                for entry in entries {
                    collect_inject_value_edges(src, &entry.key, file, out);
                    collect_inject_value_edges(src, &entry.value, file, out);
                }
            }
            Collection::Props { entries, .. } => {
                for entry in entries {
                    for spel in &entry.value.spel_refs {
                        edge(out, src, &spel.value);
                    }
                }
            }
            // `Collection` is `#[non_exhaustive]` (forward-compat, see its
            // own doc comment): `Unrecognized` plus any future variant this
            // build doesn't know about yet has no edges to extract.
            _ => {}
        },
        // `InjectValue` is `#[non_exhaustive]` for the same reason.
        _ => {}
    }
}

/// Top-level (`<beans>`-body) namespaced elements (`util:list`,
/// `jee:jndi-lookup`, ...) — these register a bean in their own right (see
/// `collect_file_edges`'s own doc comment), so they get their own source
/// node: their `id` when present, else a `file`-qualified anonymous key
/// exactly like `node_name`'s own bean case.
fn collect_namespaced_edges(el: &NamespacedElement, file: &str, out: &mut Vec<(String, String)>) {
    let src = el
        .id
        .as_ref()
        .map(|s| s.value.clone())
        .unwrap_or_else(|| format!("$anon@{file}:{}-{}", el.span.start, el.span.end));
    for r in &el.refs {
        edge(out, &src, &r.value.raw);
    }
}

/// A `<bean>`'s own decorator children (`aop:scoped-proxy`, an
/// allowlisted-ref-bearing `aop:advisor`/`aop:aspect`/..., see
/// `NS_REF_ALLOWLIST`) are wiring *on that bean*, not a separate bean of
/// their own -- unlike a top-level namespaced element
/// (`collect_namespaced_edges`), a decorator has no independent identity a
/// `ref` elsewhere in the document could ever target. Every allowlisted ref
/// it carries is therefore emitted directly from the *containing bean's*
/// `src`, not from the decorator's own id/anon key -- otherwise those edges
/// show up in the graph attached to a node nothing else ever points at or
/// from, disconnected from the bean they actually describe.
fn collect_decorator_edges(src: &str, el: &NamespacedElement, out: &mut Vec<(String, String)>) {
    for r in &el.refs {
        edge(out, src, &r.value.raw);
    }
}

/// Emits every outgoing edge for one bean: `parent`/`depends-on`/
/// `factory-bean`/lookup-method/replaced-method targets, every property
/// and constructor-arg value (recursed), and decorator (`aop:scoped-
/// proxy`, ...) allowlisted refs -- then recurses into nested/inner beans
/// via `collect_inject_value_edges` so no level of nesting is skipped.
fn collect_bean_edges(bean: &Bean, src: &str, file: &str, out: &mut Vec<(String, String)>) {
    if let Some(parent) = &bean.parent {
        edge(out, src, &parent.value.raw);
    }
    for dep in &bean.depends_on {
        edge(out, src, &dep.value.raw);
    }
    if let Some(factory_bean) = &bean.factory_bean {
        edge(out, src, &factory_bean.value.raw);
    }
    for lookup in &bean.lookup_methods {
        if let Some(target) = &lookup.value.bean {
            edge(out, src, &target.value.raw);
        }
    }
    for replaced in &bean.replaced_methods {
        if let Some(replacer) = &replaced.value.replacer {
            edge(out, src, &replacer.value.raw);
        }
    }
    for property in &bean.properties {
        collect_inject_value_edges(src, &property.value, file, out);
    }
    for arg in &bean.constructor_args {
        collect_inject_value_edges(src, &arg.value, file, out);
    }
    for decorator in &bean.decorators {
        collect_decorator_edges(src, decorator, out);
    }
}

/// Walks a `BeansFile` (and its `nested_profiles`, at any depth) and
/// returns every bean-to-bean edge found, plus every top-level id-bearing
/// `NamespacedElement` (`util:list`, `jee:jndi-lookup`, ...) — those
/// register a bean the same way `<bean id=...>` does (see
/// `NamespacedElement`'s own doc comment), so a `ref` pointing at one
/// isn't dangling even though this crate never parses their internals.
/// `file` is the path this `BeansFile` was parsed from, unchanged for every
/// nested profile -- a nested `<beans profile=...>` block is still part of
/// the same source file, so its own anonymous beans need the same
/// qualifier its parent's do.
fn collect_file_edges(file_data: &BeansFile, file: &str, out: &mut Vec<(String, String)>) {
    for bean in &file_data.beans {
        let src = node_name(bean, file);
        collect_bean_edges(bean, &src, file, out);
    }
    for el in &file_data.namespaced {
        collect_namespaced_edges(el, file, out);
    }
    for nested in &file_data.nested_profiles {
        collect_file_edges(nested, file, out);
    }
}

fn main() {
    let paths: Vec<String> = env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: dep_graph_dot <applicationContext.xml> [more paths...] > graph.dot");
        std::process::exit(1);
    }

    let mut edges = Vec::new();
    for path in &paths {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(err) => {
                eprintln!("{path}: read error: {err}");
                continue;
            }
        };
        let result = beans_xml::parse_bytes(&bytes);
        let Some(beans_file) = &result.beans else {
            eprintln!("{path}: not a <beans> document -- see diagnostics, skipping");
            continue;
        };
        collect_file_edges(beans_file, path, &mut edges);
    }

    let mut dot = String::from("digraph beans {\n");
    for (from, to) in &edges {
        let _ = writeln!(dot, "  \"{}\" -> \"{}\";", escape(from), escape(to));
    }
    dot.push('}');
    println!("{dot}");
}
