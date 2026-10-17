.PHONY: wasm wasm-docx wasm-xlsx wasm-xlsx-release wasm-pptx wasm-pptx-release wasm-otel wasm-release wasm-release-all wasm-release-otel wasm-release-size wasm-perf wasm-perf-o4 wasm-perf-simd wasm-perf-o4-simd wasm-profiling plugins plugins-release

# Cargo and wasm-bindgen use the same optional external output directory.
ifneq ($(strip $(CARGO_TARGET_DIR)),)
export CARGO_TARGET_DIR
else
unexport CARGO_TARGET_DIR
endif
WASM_CARGO_BUILD_DIR := $(if $(strip $(CARGO_TARGET_DIR)),$(CARGO_TARGET_DIR),target)

WASM_TARGET := wasm32-unknown-unknown
WASM_OUT_DIR := crates/jade-docx-editor/pkg
WASM_OUT_NAME := jade_docx
# The spreadsheet entry point is its own module rather than another export of
# the DOCX one: a page that only opens a workbook should not download the Word
# layout engine, and the two have no code in common above `jade-painter`.
WASM_XLSX_OUT_DIR := crates/jade-xlsx-editor/pkg
WASM_XLSX_OUT_NAME := jade_xlsx
# Likewise for slides. Three entry points, one per format, so opening a deck
# downloads neither the Word layout engine nor the formula evaluator.
WASM_PPTX_OUT_DIR := crates/jade-pptx-editor/pkg
WASM_PPTX_OUT_NAME := jade_pptx
WASM_OPT_FEATURES := --enable-bulk-memory --enable-sign-ext --enable-nontrapping-float-to-int --enable-reference-types --enable-mutable-globals

# Directory served as Vite static assets by every package that sets
# `publicDir: "../../public"`. Plugin guest WASM modules are copied here so
# jade-ide can `fetch('/plugins/<name>.wasm')` at runtime. The directory is
# gitignored because it's a build output.
PLUGIN_OUT_DIR := public/plugins

# Dev-loop build. Uses the `wasm-dev` cargo profile (opt-level 1 + fully
# optimized dependencies) instead of the plain `dev` profile: opt-level 0 made
# the editor several times slower than release and invalidated perf
# measurements (docs/perf-improvement-plan-20260702.md P0). For perf
# evaluation use `wasm-release` / `wasm-profiling`.
# `wasm` builds every entry point. The playground imports the spreadsheet glue
# statically, so a build that produced only the DOCX module would fail at bundle
# time rather than at run time, and every CI job that runs `make wasm` would
# have to learn a second target.
wasm: wasm-docx wasm-xlsx wasm-pptx

wasm-docx:
	@cargo build -p jade-docx-editor --manifest-path crates/jade-docx-editor/Cargo.toml --target $(WASM_TARGET) --profile wasm-dev
	@wasm-bindgen --target web --out-dir $(WASM_OUT_DIR) --out-name $(WASM_OUT_NAME) "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/wasm-dev/jade_docx_editor.wasm"
	@node scripts/optimize-wasm-js.mjs $(WASM_OUT_DIR)/$(WASM_OUT_NAME).js

wasm-xlsx:
	@cargo build -p jade-xlsx-editor --target $(WASM_TARGET) --profile wasm-dev
	@wasm-bindgen --target web --out-dir $(WASM_XLSX_OUT_DIR) --out-name $(WASM_XLSX_OUT_NAME) "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/wasm-dev/jade_xlsx_editor.wasm"

wasm-xlsx-release:
	@cargo build -p jade-xlsx-editor --target $(WASM_TARGET) --release
	@wasm-bindgen --target web --out-dir $(WASM_XLSX_OUT_DIR) --out-name $(WASM_XLSX_OUT_NAME) "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/release/jade_xlsx_editor.wasm"
	@wasm-opt $(WASM_OPT_FEATURES) -O3 $(WASM_XLSX_OUT_DIR)/$(WASM_XLSX_OUT_NAME)_bg.wasm -o $(WASM_XLSX_OUT_DIR)/$(WASM_XLSX_OUT_NAME)_bg.wasm

wasm-pptx:
	@cargo build -p jade-pptx-editor --target $(WASM_TARGET) --profile wasm-dev
	@wasm-bindgen --target web --out-dir $(WASM_PPTX_OUT_DIR) --out-name $(WASM_PPTX_OUT_NAME) "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/wasm-dev/jade_pptx_editor.wasm"

wasm-pptx-release:
	@cargo build -p jade-pptx-editor --target $(WASM_TARGET) --release
	@wasm-bindgen --target web --out-dir $(WASM_PPTX_OUT_DIR) --out-name $(WASM_PPTX_OUT_NAME) "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/release/jade_pptx_editor.wasm"
	@wasm-opt $(WASM_OPT_FEATURES) -O3 $(WASM_PPTX_OUT_DIR)/$(WASM_PPTX_OUT_NAME)_bg.wasm -o $(WASM_PPTX_OUT_DIR)/$(WASM_PPTX_OUT_NAME)_bg.wasm

wasm-otel:
	@cargo build -p jade-docx-editor --manifest-path crates/jade-docx-editor/Cargo.toml --target $(WASM_TARGET) --features otel --profile wasm-dev
	@wasm-bindgen --target web --out-dir $(WASM_OUT_DIR) --out-name $(WASM_OUT_NAME) "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/wasm-dev/jade_docx_editor.wasm"
	@node scripts/optimize-wasm-js.mjs $(WASM_OUT_DIR)/$(WASM_OUT_NAME).js

# Production builds with wasm-opt -O3 optimization (speed-optimized).
# Pairs with the speed-first `[profile.release]` (opt-level = 3) in Cargo.toml.
# `wasm-release` is DOCX only (the publish workflow ships that module alone);
# `wasm-release-all` is the release counterpart of `wasm` and is what the
# Amplify deploy runs, so the demo does not serve the `wasm-dev` profile.
wasm-release-all: wasm-release wasm-xlsx-release wasm-pptx-release

wasm-release:
	@cargo build -p jade-docx-editor --manifest-path crates/jade-docx-editor/Cargo.toml --target $(WASM_TARGET) --release
	@wasm-bindgen --target web --out-dir $(WASM_OUT_DIR) --out-name $(WASM_OUT_NAME) "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/release/jade_docx_editor.wasm"
	@node scripts/optimize-wasm-js.mjs $(WASM_OUT_DIR)/$(WASM_OUT_NAME).js
	@wasm-opt $(WASM_OPT_FEATURES) -O3 $(WASM_OUT_DIR)/$(WASM_OUT_NAME)_bg.wasm -o $(WASM_OUT_DIR)/$(WASM_OUT_NAME)_bg.wasm

wasm-release-otel:
	@cargo build -p jade-docx-editor --manifest-path crates/jade-docx-editor/Cargo.toml --target $(WASM_TARGET) --release --features otel
	@wasm-bindgen --target web --out-dir $(WASM_OUT_DIR) --out-name $(WASM_OUT_NAME) "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/release/jade_docx_editor.wasm"
	@node scripts/optimize-wasm-js.mjs $(WASM_OUT_DIR)/$(WASM_OUT_NAME).js
	@wasm-opt $(WASM_OPT_FEATURES) -O3 $(WASM_OUT_DIR)/$(WASM_OUT_NAME)_bg.wasm -o $(WASM_OUT_DIR)/$(WASM_OUT_NAME)_bg.wasm

# Size-optimized build (smaller binary, slower runtime).
# Uses the `release-size` cargo profile (opt-level = "s") and wasm-opt -Oz.
# This is the previous behaviour of `wasm-release`; use it when download size
# matters more than throughput.
wasm-release-size:
	@cargo build -p jade-docx-editor --manifest-path crates/jade-docx-editor/Cargo.toml --target $(WASM_TARGET) --profile release-size
	@wasm-bindgen --target web --out-dir $(WASM_OUT_DIR) --out-name $(WASM_OUT_NAME) "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/release-size/jade_docx_editor.wasm"
	@node scripts/optimize-wasm-js.mjs $(WASM_OUT_DIR)/$(WASM_OUT_NAME).js
	@wasm-opt $(WASM_OPT_FEATURES) -Oz $(WASM_OUT_DIR)/$(WASM_OUT_NAME)_bg.wasm -o $(WASM_OUT_DIR)/$(WASM_OUT_NAME)_bg.wasm

# Performance-optimized build (larger size but faster runtime)
# Uses -O3 for wasm-opt and opt-level=3 for rustc
wasm-perf:
	@cargo build -p jade-docx-editor --manifest-path crates/jade-docx-editor/Cargo.toml --target $(WASM_TARGET) --profile perf
	@wasm-bindgen --target web --out-dir $(WASM_OUT_DIR) --out-name $(WASM_OUT_NAME) "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/perf/jade_docx_editor.wasm"
	@node scripts/optimize-wasm-js.mjs $(WASM_OUT_DIR)/$(WASM_OUT_NAME).js
	@wasm-opt $(WASM_OPT_FEATURES) -O3 $(WASM_OUT_DIR)/$(WASM_OUT_NAME)_bg.wasm -o $(WASM_OUT_DIR)/$(WASM_OUT_NAME)_bg.wasm

# Alternative performance build using wasm-opt -O4 (often faster, usually larger output)
wasm-perf-o4:
	@cargo build -p jade-docx-editor --manifest-path crates/jade-docx-editor/Cargo.toml --target $(WASM_TARGET) --profile perf
	@wasm-bindgen --target web --out-dir $(WASM_OUT_DIR) --out-name $(WASM_OUT_NAME) "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/perf/jade_docx_editor.wasm"
	@node scripts/optimize-wasm-js.mjs $(WASM_OUT_DIR)/$(WASM_OUT_NAME).js
	@wasm-opt $(WASM_OPT_FEATURES) -O4 $(WASM_OUT_DIR)/$(WASM_OUT_NAME)_bg.wasm -o $(WASM_OUT_DIR)/$(WASM_OUT_NAME)_bg.wasm

# SIMD build (requires WebAssembly SIMD-capable runtime)
wasm-perf-simd:
	@RUSTFLAGS="-C target-feature=+simd128" cargo build -p jade-docx-editor --manifest-path crates/jade-docx-editor/Cargo.toml --target $(WASM_TARGET) --profile perf
	@wasm-bindgen --target web --out-dir $(WASM_OUT_DIR) --out-name $(WASM_OUT_NAME) "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/perf/jade_docx_editor.wasm"
	@node scripts/optimize-wasm-js.mjs $(WASM_OUT_DIR)/$(WASM_OUT_NAME).js
	@wasm-opt $(WASM_OPT_FEATURES) --enable-simd -O3 $(WASM_OUT_DIR)/$(WASM_OUT_NAME)_bg.wasm -o $(WASM_OUT_DIR)/$(WASM_OUT_NAME)_bg.wasm

# SIMD + wasm-opt -O4 build for max throughput experiments
wasm-perf-o4-simd:
	@RUSTFLAGS="-C target-feature=+simd128" cargo build -p jade-docx-editor --manifest-path crates/jade-docx-editor/Cargo.toml --target $(WASM_TARGET) --profile perf
	@wasm-bindgen --target web --out-dir $(WASM_OUT_DIR) --out-name $(WASM_OUT_NAME) "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/perf/jade_docx_editor.wasm"
	@node scripts/optimize-wasm-js.mjs $(WASM_OUT_DIR)/$(WASM_OUT_NAME).js
	@wasm-opt $(WASM_OPT_FEATURES) --enable-simd -O4 $(WASM_OUT_DIR)/$(WASM_OUT_NAME)_bg.wasm -o $(WASM_OUT_DIR)/$(WASM_OUT_NAME)_bg.wasm

# Build all sample decoration plugins as standalone `cdylib` wasm modules
# and stage them under `$(PLUGIN_OUT_DIR)` so the Vite dev server and
# production build can serve them as static assets. These are not bundled
# into the host; the editor loads them dynamically at runtime (see
# `crates/jade-preview-plugin-api` for the ABI).
#
# Debug build — useful during plugin development. Pass-through to
# `target/wasm32-unknown-unknown/debug/circle_mark_plugin.wasm` followed by
# a copy into the public dir.
plugins:
	@cargo build -p circle-mark-plugin -p diff-highlight-plugin --target $(WASM_TARGET)
	@mkdir -p $(PLUGIN_OUT_DIR)
	@cp "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/debug/circle_mark_plugin.wasm" $(PLUGIN_OUT_DIR)/circle_mark_plugin.wasm
	@cp "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/debug/diff_highlight_plugin.wasm" $(PLUGIN_OUT_DIR)/diff_highlight_plugin.wasm

# Size-optimized plugin build. Emits
# `target/wasm32-unknown-unknown/release/circle_mark_plugin.wasm` and
# copies it to `$(PLUGIN_OUT_DIR)` for shipping alongside the IDE.
plugins-release:
	@cargo build -p circle-mark-plugin -p diff-highlight-plugin --target $(WASM_TARGET) --release
	@mkdir -p $(PLUGIN_OUT_DIR)
	@cp "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/release/circle_mark_plugin.wasm" $(PLUGIN_OUT_DIR)/circle_mark_plugin.wasm
	@cp "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/release/diff_highlight_plugin.wasm" $(PLUGIN_OUT_DIR)/diff_highlight_plugin.wasm

# Profiling build with debug info
wasm-profiling:
	@cargo build -p jade-docx-editor --manifest-path crates/jade-docx-editor/Cargo.toml --target $(WASM_TARGET) --profile profiling
	@wasm-bindgen --target web --out-dir $(WASM_OUT_DIR) --out-name $(WASM_OUT_NAME) "$(WASM_CARGO_BUILD_DIR)/$(WASM_TARGET)/profiling/jade_docx_editor.wasm"
	@node scripts/optimize-wasm-js.mjs $(WASM_OUT_DIR)/$(WASM_OUT_NAME).js
	@wasm-opt $(WASM_OPT_FEATURES) -O3 --debuginfo $(WASM_OUT_DIR)/$(WASM_OUT_NAME)_bg.wasm -o $(WASM_OUT_DIR)/$(WASM_OUT_NAME)_bg.wasm
