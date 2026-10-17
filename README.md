# jade-core

The Rust engine behind jade, a WYSIWYG DOCX editor targeting Microsoft Office 365 Word compatibility, growing to `.xlsx` and `.pptx`.

This repository holds the engine crates only: OOXML readers and writers, layout, painting, the editing runtime (including the Rust/WASM DOCX entry point), and the headless APIs / CLIs / MCP servers. The browser UI, the IDE, the playground and the spreadsheet / slide browser entry points (`jade-xlsx-editor`, `jade-pptx-editor`) live in a separate repository that consumes this one. The native desktop app (gpui) lives in [jade-rs/jade-office](https://github.com/jade-rs/jade-office).

## Layout

```
crates/
  # format-neutral
  jade-painter/ jade-canvas-painter/ jade-skia-painter/   2D painting trait and backends
  jade-opc/                                               zip container and .rels shared by every OOXML format
  jade-text-layout/                                       line-break rules and a greedy filler
  jade-selection/  jade-diff-align/  jade-preview-plugin-api/  jade-plugins/

  # DOCX / XLSX / PPTX
  jade-docx-*/  jade-xlsx-*/  jade-pptx-*/                reader, writer, layout, render, headless-* (the docx editor is the WASM entry point)
  jade-docx-native-render/  jade-xlsx-native-render/      native (Skia) renderers
third_party/ironcalc-base/                                vendored formula engine
fonts/  public/fixtures/  tests/word-oracle/              test data used by the crate tests
```

## Build

```sh
cargo test --workspace            # engine crates
make wasm                         # crates/jade-docx-editor/pkg (needs wasm-bindgen-cli 0.2.114)
make plugins-release              # sample decoration plugins -> $(PLUGIN_OUT_DIR)
```

`jade-skia-painter`, `jade-docx-native-render` and `jade-xlsx-native-render` are outside the workspace on purpose (they pull in Skia's prebuilt binaries); build them from their own directories.

## License

MIT. See [LICENSE](LICENSE).

Third-party dependency licenses: see THIRD_PARTY_LICENSES.html (regenerate with `cargo about generate about.hbs --all-features`).

`third_party/ironcalc-base` is vendored upstream code and keeps its own licenses (MIT / Apache-2.0).
