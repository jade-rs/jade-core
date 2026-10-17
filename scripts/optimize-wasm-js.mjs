#!/usr/bin/env node
/**
 * Post-process wasm-bindgen generated JS.
 *
 * Currently a no-op. Previous attempts at optimizing the generated JS
 * (handleError removal, instanceof simplification, string decode caching)
 * all caused E2E test failures:
 *
 * - handleError removal: multiline regex failed to capture function bodies,
 *   producing empty stubs that broke Canvas operations
 * - String decode cache: (ptr << 16) ^ len key collision + allocator reuse
 *   caused wrong strings to be returned
 * - instanceof simplification: unknown interaction causing worker init timeouts
 *
 * A safer approach would require an AST-based transform (e.g. via acorn/escodegen).
 */

// No-op: just report that no optimizations were applied.
const filePath = process.argv[2] || 'crates/jade-docx-editor/pkg/jade_docx.js';
console.log(`Post-process ${filePath}: no optimizations applied (disabled for stability)`);
