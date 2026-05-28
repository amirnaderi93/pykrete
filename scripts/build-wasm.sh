#!/usr/bin/env bash
# Build the pykrete-wasm crate into an npm-style package the Starlight
# docs site can import. Output lands in docs-site/pykrete-wasm-pkg/
# (gitignored) — `wasm-pack build` emits:
#   - pykrete_wasm_bg.wasm   (the compiled module)
#   - pykrete_wasm.js        (ES-module glue, calls `instantiate`)
#   - pykrete_wasm.d.ts      (TypeScript types for `check_source`)
#   - package.json           (npm metadata, `wasm-pack` generates it)
#
# `--target web` picks the ES-module flavour Starlight / Astro can
# import directly via a top-level `import`. Other targets exist
# (`bundler` for webpack, `nodejs` for Node) — `web` is what a
# browser-first playground needs.
#
# Prereq: `wasm-pack` on PATH. Install with `cargo install wasm-pack`
# (the CI workflow does this too).

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out_dir="$repo_root/docs-site/pykrete-wasm-pkg"

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "error: wasm-pack not found on PATH." >&2
  echo "       install with: cargo install wasm-pack" >&2
  exit 1
fi

cd "$repo_root/crates/pykrete-wasm"

# `--target web` → ES-module glue suitable for a `<script type=module>`
# import in the browser. `--out-name pykrete_wasm` keeps the emitted
# filenames stable (default already matches the crate name, but
# pinning makes it explicit).
wasm-pack build \
  --target web \
  --out-dir "$out_dir" \
  --out-name pykrete_wasm

echo
echo "wasm package written to: $out_dir"
ls -lh "$out_dir"
