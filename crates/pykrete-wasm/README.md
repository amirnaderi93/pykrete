# pykrete-wasm

WebAssembly bindings for the pykrete analyzer. Exposes single-file `check_source`, `hover_at`, `complete_at`, and `definition_at` entry points from a JS host. Consumed by the in-browser playground at <https://amirnaderi93.github.io/pykrete/playground/>.

Not a general-purpose embedding library — multi-file analysis and project configuration stay CLI / LSP features. See the [main repo](https://github.com/amirnaderi93/pykrete) for the analyzer crate and docs site.
