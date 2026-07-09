//! Regenerates every `fixtures/**/{name}.expected.json` from the current
//! parser, by running `beans_xml::parse_bytes` over each `fixtures/**/{name}.xml`
//! and pretty-printing the resulting `ParseResult` as JSON.
//!
//! Per this crate's own conformance-suite convention (`tests/conformance.rs`'s
//! own doc comment, matching batis-xml's sibling convention): `expected.json`
//! files are never hand-typed — they are generated through this
//! review-and-approve flow, then hand-reviewed for correctness (M1e-fixtures'
//! own task: independently derive a sample of fixtures' bean/ref lists by
//! reading the XML *before* looking at this tool's output) before being
//! committed and locked against regressions by `tests/conformance.rs`.
//!
//! ```sh
//! cargo run --example gen_fixtures
//! ```
//!
//! Walks every `fixtures/<group>/*.xml` file and (re)writes its sibling
//! `.expected.json`, unconditionally — re-run after any parser change that's
//! meant to affect fixture output, then `git diff` the result to see exactly
//! what changed before committing.

use std::fs;
use std::path::Path;

fn main() {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut written = 0;
    for group_entry in fs::read_dir(&fixtures_root).expect("fixtures/ exists") {
        let group_path = group_entry.expect("dir entry").path();
        if !group_path.is_dir() {
            continue;
        }
        for file_entry in fs::read_dir(&group_path).expect("group dir readable") {
            let xml_path = file_entry.expect("dir entry").path();
            if xml_path.extension().and_then(|e| e.to_str()) != Some("xml") {
                continue;
            }
            let bytes = fs::read(&xml_path).expect("read fixture xml");
            let result = beans_xml::parse_bytes(&bytes);
            let json = serde_json::to_string_pretty(&result).expect("serialize ParseResult");
            let expected_path = xml_path.with_extension("expected.json");
            fs::write(&expected_path, format!("{json}\n")).expect("write expected.json");
            println!("wrote {}", expected_path.display());
            written += 1;
        }
    }
    println!("done: {written} fixture(s) regenerated");
}
