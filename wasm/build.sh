#!/usr/bin/env bash
# One-command build for the beans-xml npm package: wasm-pack build, then
# patch package.json to the npm name (beans-xml differs from the Cargo
# package name beans-xml-wasm, same split as the batis-xml sibling crate).
# Not published from here.
#
# Usage: ./wasm/build.sh
set -euo pipefail
cd "$(dirname "$0")/.."

wasm-pack build wasm --target nodejs

cp LICENSE-MIT LICENSE-APACHE wasm/pkg/
# The compiled wasm binary statically links encoding_rs, which embeds
# WHATWG-owned encoding data under a separate BSD-3-Clause license (on top
# of encoding_rs's own Apache-2.0/MIT) -- reproduce that notice in the
# shipped package, same as the LICENSE-* files above (batis-xml sibling
# pattern, B33 cold code review).
cp wasm/THIRD_PARTY_NOTICES wasm/pkg/THIRD_PARTY_NOTICES

node - <<'NODE'
const fs = require("fs");

const pkgPath = "wasm/pkg/package.json";
const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
pkg.name = "beans-xml";
for (const f of ["LICENSE-MIT", "LICENSE-APACHE", "THIRD_PARTY_NOTICES"]) {
  if (!pkg.files.includes(f)) {
    pkg.files.push(f);
  }
}
// wasm-pack copies Cargo.toml's `repository` field into a
// `{type: "git", url: ...}` object, but leaves `url` as a bare https://
// page link -- npm's own convention is the "git+<url>.git" form so
// tooling (e.g. npm's package page, some provenance/audit tools) can tell
// it's a git remote, not just a webpage (batis-xml sibling pattern, B25).
if (pkg.repository && typeof pkg.repository.url === "string") {
  const url = pkg.repository.url;
  if (!url.startsWith("git+")) {
    pkg.repository.url = `git+${url}${url.endsWith(".git") ? "" : ".git"}`;
  }
} else if (typeof pkg.repository === "string" && pkg.repository.startsWith("https://")) {
  pkg.repository = { type: "git", url: `git+${pkg.repository}.git` };
}
// Node-only build (see wasm/README.md) -- no browser/bundler target yet,
// so pin the one runtime this package is actually tested against.
pkg.engines = { node: ">=18" };
pkg.exports = {
  ".": {
    types: "./beans_xml_wasm.d.ts",
    default: "./beans_xml_wasm.js",
  },
};
fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
NODE

echo "wasm/pkg ready: $(node -p "require('./wasm/pkg/package.json').name")@$(node -p "require('./wasm/pkg/package.json').version")"
