//! Prints, for one or more `<beans>` XML files, a table of every bean's
//! id/names/class -- the flattest possible view of "what beans does this
//! file define", including beans nested inside `<beans profile="...">`
//! blocks (`BeansFile::nested_profiles`, recursed the same way
//! `dep_graph_dot` walks them).
//!
//! Run with: cargo run --example bean_list -- path/to/applicationContext.xml [more paths...]

use beans_xml::{Bean, BeansFile};
use std::env;
use std::fs;

/// Collects every `Bean` reachable from `file`, including ones nested
/// inside `<beans profile="...">` blocks -- `nested_profiles` is a `Vec<
/// BeansFile>` at any depth, so this recurses rather than only looking at
/// the top level.
fn collect_beans<'a>(file: &'a BeansFile, out: &mut Vec<&'a Bean>) {
    out.extend(file.beans.iter());
    for nested in &file.nested_profiles {
        collect_beans(nested, out);
    }
}

fn bean_row(bean: &Bean) -> (String, String, String) {
    let id = bean
        .id
        .as_ref()
        .map(|s| s.value.clone())
        .unwrap_or_else(|| "<anonymous>".to_string());
    let names = if bean.names.is_empty() {
        String::new()
    } else {
        bean.names
            .iter()
            .map(|n| n.value.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let class = bean
        .class
        .as_ref()
        .map(|c| c.value.raw.clone())
        .unwrap_or_else(|| "<none>".to_string());
    (id, names, class)
}

fn main() {
    let paths: Vec<String> = env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: bean_list <applicationContext.xml> [more paths...]");
        std::process::exit(1);
    }

    for path in &paths {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(err) => {
                eprintln!("{path}: read error: {err}");
                continue;
            }
        };
        let result = beans_xml::parse_bytes(&bytes);
        println!("== {path} (encoding: {:?}) ==", result.encoding);

        let Some(beans_file) = &result.beans else {
            println!("  (not a <beans> document -- see diagnostics)");
            continue;
        };

        let mut beans = Vec::new();
        collect_beans(beans_file, &mut beans);

        if beans.is_empty() {
            println!("  (no beans)");
        } else {
            let (id_h, names_h, class_h) = ("id", "names", "class");
            println!("  {id_h:<24} {names_h:<28} {class_h}");
            for bean in &beans {
                let (id, names, class) = bean_row(bean);
                println!("  {id:<24} {names:<28} {class}");
            }
        }

        if !result.diagnostics.is_empty() {
            println!("  diagnostics:");
            for d in &result.diagnostics {
                println!("    {:?}: {}", d.code, d.message);
            }
        }
    }
}
