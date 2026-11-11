import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { gunzipSync } from 'node:zlib';
import { compareGlyphs, matchImageBounds } from './word_coordinate_matching.mjs';

const { chromium } = createRequire(import.meta.url)('playwright');
const root = new URL('../../', import.meta.url);
const corpus = new URL('tests/word-oracle/mac-elsa-synthetic/', root);
const wasm = new URL('crates/jade-docx-editor/pkg/', root);
const report = new URL('target/word-mac-browser-differences.json', root);
const pt = 72 / 96;
const finalRevisionView = new Set([
  'numbering-with-review-mac', 'review-duplicate-ids-mac', 'review-kinds-mac',
  'numbering-del-mac', 'numbering-del2-mac',
]);
const shapedArabic = new Set(['table-inline-image-mac', 'table-object-edge-cases-mac']);
const renderedHyphenFixtures = new Set([
  'aligned-hyphen-hanging-indent',
  'discretionary-hyphen-page-origins',
  'explicit-hyphen-wrap-boundaries',
  'hyphenation-automatic-narrow-paragraphs',
  'hyphenation-manual-narrow-paragraphs',
  'styled-hyphen-body-table',
]);
const hyphenGlyph = /^[-\u00ad\u2010\u2011]$/u;
const arabicScript = /\p{Script=Arabic}/u;
const textBoxes = new Map([
  ['floating-wrap-sides-mac', 3], ['floating-inside-outside-mac', 4],
  ['floating-simple-position-mac', 1], ['floating-square-wrap-mac', 1],
  ['floating-through-concave-mac', 1], ['floating-tight-polygon-mac', 1],
  ['floating-top-bottom-offset-mac', 1], ['floating-z-order-mac', 3],
  ['numbering-textbox-full-matrix-mac', 1], ['textbox-fixed-pitch-matrix-mac', 1],
  ['textbox-body-properties-mac', 1], ['textbox-dashed-mac', 1],
  ['textbox-tall-header-overlap-mac', 1], ['document-variable-fields-mac', 1],
  ['word-compat-comprehensive-mac', 1], ['table-object-edge-cases-mac', 1],
  ['table-inline-image-mac', 1],
  ['footnote-table-textbox-order-mac', 1], ['word-mac-probe-float-lookahead', 5],
]);

const emptyTextBoxParagraphs = new Map([
  ['floating-z-order-mac', 1],
  ['textbox-tall-header-overlap-mac', 1],
  ['word-mac-probe-float-lookahead', 5],
]);

function hash(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

function visibleGlyph(text = '') {
  const normalized = text.trim().normalize('NFC')
    .replace(/[\u2F00-\u2FD5]/g, radical => radical.normalize('NFKC'))
    .replaceAll('⻑', '長').replaceAll('⺠', '民').replaceAll('⻘', '青').replaceAll('⻲', '亀');
  return /\p{Extended_Pictographic}/u.test(normalized)
    ? normalized.replace(/[\uFE0F\u200D]/g, '')
    : normalized;
}

// Canvas rotates the image around its center. Word's PDF reports the
// axis-aligned page bounds after that transform.
function imageBoundsPt({ x, y, width, height, rotationDeg }) {
  const radians = rotationDeg * Math.PI / 180;
  const cos = Math.abs(Math.cos(radians));
  const sin = Math.abs(Math.sin(radians));
  const halfWidth = (width * cos + height * sin) / 2;
  const halfHeight = (width * sin + height * cos) / 2;
  const centerX = x + width / 2;
  const centerY = y + height / 2;
  return [
    (centerX - halfWidth) * pt,
    (centerY - halfHeight) * pt,
    (centerX + halfWidth) * pt,
    (centerY + halfHeight) * pt,
  ];
}

async function capture(page, bytes, name) {
  await page.route('https://word-coordinates.test/document.docx', route => route.fulfill({
    body: bytes,
  }));
  try {
    return await page.evaluate(async ({ revisionFinal, captureArabic, captureHyphens }) => {
      const paintedArabic = [];
      const context = OffscreenCanvasRenderingContext2D.prototype;
      const fillText = context.fillText;
      if (captureArabic) {
        context.fillText = function(text, x, y, ...rest) {
          const start = text.search(/\p{Script=Arabic}/u);
          if (start >= 0) {
            const offset = this.measureText(text.slice(0, start)).width;
            const origin = this.getTransform().transformPoint({ x: x + offset, y });
            paintedArabic.push({ text: text.slice(start), x: origin.x, y: origin.y });
          }
          return fillText.call(this, text, x, y, ...rest);
        };
      }
      const wasm = await import('/wasm/jade_docx.js');
      await wasm.default();
      wasm.setWordLayoutPlatform('mac');
      const options = new wasm.Options();
      options.show_formatting_marks = false;
      if (revisionFinal) options.renderChanges = false;
      else options.useDocumentRevisionView();
      const view = new wasm.OffscreenDocumentView(new OffscreenCanvas(816, 1056), options, 1);
      try {
        const response = await fetch('/document.docx');
        await view.load_docx(new Uint8Array(await response.arrayBuffer()));
        await view.waitForPendingImages();
        if (captureHyphens) view.beginPdfTextCapture();
        const layout = view.captureFullLayoutDebug();
        const paintedHyphens = captureHyphens
          ? Array.from({ length: layout.page_count }, (_, i) => view.takePagePdfTextRuns(i)
            .filter(({ text }) => /^[-\u2010\u2011]$/u.test(text))
            .map(({ text, transform: [, , , , x, y] }) => [text, x, y]))
          : [];
        return {
          ...layout,
          paintedArabic,
          paintedHyphens,
          dimensions: Array.from({ length: layout.page_count }, (_, i) => view.page_dimensions(i)),
        };
      } finally {
        context.fillText = fillText;
        if (captureHyphens) view.endPdfTextCapture();
        view.free();
      }
    }, {
      revisionFinal: finalRevisionView.has(name),
      captureArabic: shapedArabic.has(name),
      captureHyphens: renderedHyphenFixtures.has(name),
    });
  } finally {
    await page.unroute('https://word-coordinates.test/document.docx');
  }
}

function checkStories(name, actual) {
  const uncaptured = [];
  if (name === 'footnote-table-textbox-order-mac') {
    // Word omits the note referenced from inside the text box.
    uncaptured.push('footnotes');
    assert.equal(actual.auxiliary_lines.filter(entry => entry.story === 'footnote').length, 7);
    assert.equal(actual.auxiliary_lines.filter(entry => entry.story === 'endnote').length, 2);
  }
  if (textBoxes.has(name)) {
    uncaptured.push('text_boxes');
    assert.equal(actual.source_surface_indices_complete, true);
    assert.equal(actual.text_box_source_inventory_complete, true);
    assert.equal(actual.source_text_stories.length, textBoxes.get(name));
    assert.equal(actual.text_box_regions.length, textBoxes.get(name));
    let emptyParagraphs = 0;
    for (const { kind, paragraphIndex, runIndex } of actual.source_text_stories) {
      assert.equal(kind, 'text_box');
      const regions = actual.text_box_regions.filter(region =>
        region.paragraphIndex === paragraphIndex && region.runIndex === runIndex);
      assert.equal(regions.length, 1);
      const [region] = regions;
      assert(region.contentParagraphEnd > region.contentParagraphStart);
      for (let paragraph = region.contentParagraphStart; paragraph < region.contentParagraphEnd; paragraph++) {
        const visible = actual.auxiliary_lines.some(entry =>
          entry.page === region.page - 1 && entry.story === 'text_box' &&
          entry.line.paragraph === paragraph &&
          entry.line.fragments?.some(fragment => fragment.glyphs?.some(glyph => visibleGlyph(glyph.text))));
        if (!visible) emptyParagraphs++;
      }
    }
    assert.equal(emptyParagraphs, emptyTextBoxParagraphs.get(name) ?? 0, 'empty text box paragraphs');
  }
  assert.deepEqual(actual.known_uncaptured_stories, uncaptured);
}

function referenceGlyphs(name, page) {
  if (name === 'hidden-text-character-effects-mac') {
    // Word exports the decorative paragraphs as paths and multiple PDF layers.
    const visible = page.body.filter(([, , y]) => y > 150 && y < 165);
    assert.equal(visible.map(([text]) => text).join(''), 'Visibletail');
    return visible;
  }
  if (name === 'unicode-segmentation-mac') {
    assert.equal(page.body.filter(([text]) => text === '禰' || text === '🧑🧑🧒🧒').length, 2);
    const kana = page.body.findIndex(([text]) => text === 'ﾌ');
    assert.deepEqual(page.body.slice(kana, kana + 2).map(([text]) => text), ['ﾌ', 'ﾟ']);
    // Map the PDF text layer back to the graphemes whose origins Word exported.
    const graphemes = { '禰': '禰󠄀', '🧑🧑🧒🧒': '👨👨👧👦', 'ﾌ': 'ﾌﾟ' };
    return page.body.flatMap(([text, x, y], i) => i === kana + 1 ? [] : [[graphemes[text] ?? text, x, y]]);
  }
  return [...page.header, ...page.body, ...page.footer];
}

function* glyphOrigins(line) {
  for (const fragment of line.fragments ?? []) {
    for (const glyph of fragment.glyphs ?? []) {
      const x = fragment.x + glyph.x - 2 * line.report_x_offset;
      yield [glyph.text, x * pt, fragment.baseline * pt];
    }
  }
}

const tallGridName = 'word-mac-grid-tall-face';
const gridProbes = new URL('tests/word-oracle/mac-probes-20261107/', root);
const rubyProbes = new URL('tests/word-oracle/mac-probes-16114/', root);
const archivedGlyphProbes = new Map([
  ['word-mac-probe-grid-line-unit', gridProbes],
  ['word-mac-probe-grid-line-unit-table', gridProbes],
  ['word-mac-probe-kinsoku-widow', gridProbes],
  ['word-mac-probe-latin-east-asia-face', gridProbes],
  ['word-mac-probe-eqruby-grid', rubyProbes],
  ['word-mac-probe-eqruby-nogrid', rubyProbes],
  ['word-mac-probe-ruby-grid', rubyProbes],
  ['word-mac-probe-ruby-nogrid', rubyProbes],
  ['word-mac-probe-ruby-wide-grid', rubyProbes],
  ['word-mac-probe-ruby-wide-nogrid', rubyProbes],
]);
const tallGridTolerancePt = 0.65;

async function checkTallGrid(page) {
  const [bytes, referenceJson] = await Promise.all([
    readFile(new URL(`${tallGridName}.docx`, gridProbes)),
    readFile(new URL(`${tallGridName}.pages.json`, gridProbes)),
  ]);
  const reference = JSON.parse(referenceJson);
  assert.equal(hash(bytes), reference.docx_sha256, 'DOCX digest');
  assert.equal(reference.page_count, 265, 'reference page count');
  assert.equal(reference.pages.length, reference.page_count);
  assert.equal(reference.pages.reduce((total, { rows }) => total + rows.length, 0), 4692);
  const actual = await capture(page, bytes, tallGridName);
  assert.equal(actual.complete, true, 'layout capture is incomplete');
  assert.equal(actual.page_count, reference.page_count, 'page count');
  checkStories(tallGridName, actual);
  const glyphs = reference.pages.map(() => []);
  for (const { page: index, line } of [
    ...actual.body_lines.map(line => ({ page: line.page, line })), ...actual.auxiliary_lines,
  ]) {
    for (const glyph of glyphOrigins(line)) {
      if (glyph[0].trim()) glyphs[index].push(glyph);
    }
  }
  const differences = [];
  for (const [index, { page: number, rows: expected }] of reference.pages.entries()) {
    assert.equal(number, index + 1, 'reference page number');
    const rows = [];
    for (const [text, x, baseline] of glyphs[index].sort((a, b) => a[2] - b[2] || a[1] - b[1])) {
      let row = rows.at(-1);
      if (!row || baseline - row.baseline >= 1) {
        row = { baseline, glyphs: [] };
        rows.push(row);
      }
      row.glyphs.push([x, text]);
    }
    if (rows.length !== expected.length) {
      differences.push({ page: number, expectedRows: expected.length, actualRows: rows.length });
    }
    for (const [i, { baseline, glyphs: rowGlyphs }] of rows.entries()) {
      if (i >= expected.length) break;
      const text = rowGlyphs.sort((a, b) => a[0] - b[0]).map(([, text]) => text).join('');
      const [wordBaseline, wordText] = expected[i];
      if (text !== wordText || Math.abs(baseline - wordBaseline) > tallGridTolerancePt) {
        differences.push({ page: number, row: i + 1, expected: expected[i], actual: [baseline, text] });
      }
    }
  }
  return differences;
}

async function loadArchivedGlyphFixture(name) {
  const [bytes, referenceJson] = await Promise.all([
    readFile(new URL(`public/fixtures/${name}.docx`, root)),
    readFile(new URL(`${name}.json`, archivedGlyphProbes.get(name))),
  ]);
  const word = JSON.parse(referenceJson);
  assert.equal(hash(bytes), word.docx_sha256, 'DOCX digest');
  assert.equal(word.pages.length, word.page_count, 'reference page count');
  const reference = word.pages.map((page, index) => {
    assert.equal(page.page, index + 1, 'reference page number');
    return {
      size: [page.width_pt, page.height_pt],
      body: page.rows.flatMap(row => row.characters
        .filter(glyph => glyph.text.trim())
        .map(glyph => [glyph.text, glyph.x_pt, glyph.baseline_pt ?? row.baseline_pt])),
      header: [],
      footer: [],
      images: [],
    };
  });
  return { bytes, reference, manifest: { pages: word.page_count, axisTolerancePt: 0.65 } };
}

async function loadSyntheticFixture(name) {
  const dir = new URL(`${name}/`, corpus);
  const [manifestBytes, bytes, compressed] = await Promise.all(
    ['manifest.json', 'source.docx', 'pages.json.gz'].map(filename => readFile(new URL(filename, dir))),
  );
  const manifest = JSON.parse(manifestBytes);
  assert.equal(manifest.schema, 1);
  assert.equal(manifest.reference.platform, 'mac');
  assert.equal(hash(bytes), manifest.sourceSha256, 'DOCX digest');
  assert.equal(hash(compressed), manifest.coordinatesSha256, 'Word reference digest');
  const reference = JSON.parse(gunzipSync(compressed));
  assert.equal(reference.length, manifest.pages, 'reference page count');
  return { bytes, reference, manifest };
}

function checkPages(name, reference, actual, manifest) {
  assert.equal(actual.complete, true, 'layout capture is incomplete');
  assert.equal(actual.page_count, reference.length, 'page count');
  assert(Array.isArray(actual.objects), 'missing object bounds');
  checkStories(name, actual);
  const tolerance = manifest.axisTolerancePt;
  const registration = manifest.horizontalPrintRegistration;
  const compareAsGlyph = text => !shapedArabic.has(name) || !arabicScript.test(text);
  if (shapedArabic.has(name)) {
    // Browser shaping paints the Arabic span in a single Canvas operation.
    assert.equal(reference.length, 1);
    const text = 'مرحبا';
    const arabic = reference[0].body.filter(([glyph]) => arabicScript.test(glyph));
    assert.equal(arabic.map(([glyph]) => glyph.normalize('NFKC')).join(''), text);
    const x = Math.min(...arabic.map(([, origin]) => origin));
    const y = arabic[0][2];
    assert(actual.paintedArabic.length > 0, 'Arabic span was not painted');
    for (const span of actual.paintedArabic) {
      assert.equal(span.text, text);
      assert(Math.abs(span.x * pt - x) <= tolerance, 'painted Arabic origin');
      assert(Math.abs(span.y * pt - y) <= tolerance, 'painted Arabic baseline');
    }
  }
  const captureHyphens = renderedHyphenFixtures.has(name);
  const glyphs = reference.map((_, page) => captureHyphens
    ? actual.paintedHyphens[page].map(([text, x, y]) => [text, x * pt, y * pt])
    : []);
  const images = reference.map(() => []);
  const entries = actual.body_lines.map(line => ({ page: line.page, line }));
  const lines = name === 'hidden-text-character-effects-mac'
    ? entries.filter(({ line }) => line.paragraph === 4)
    : [...entries, ...actual.auxiliary_lines];
  for (const { page, line } of lines) {
    for (const [rawText, x, baseline] of glyphOrigins(line)) {
      const text = visibleGlyph(rawText);
      // Line-end hyphens can paint independently of the source glyphs.
      if (text && compareAsGlyph(text) && !(captureHyphens && hyphenGlyph.test(text))) {
        glyphs[page].push([text, x, baseline]);
      }
    }
  }
  for (const object of actual.objects) {
    if (object.kind !== 'image') continue;
    images[object.page - 1].push(imageBoundsPt(object));
  }

  const within = (expected, measured) => expected.every((v, i) => Math.abs(v - measured[i]) <= tolerance);
  const differences = [];
  for (const [i, expected] of reference.entries()) {
    const page = i + 1;
    const size = actual.dimensions[i];
    if (!size || !within(expected.size, [size.width * pt, size.height * pt])) {
      differences.push({ page, expectedSize: expected.size, actualSize: size });
    }
    let offset = 0;
    if (registration) {
      const firstX = expected.body[0][1];
      offset = firstX - registration.bodyLeftMarginPt;
      const lines = actual.body_lines.filter(line => line.page === i);
      assert.equal(lines.length, expected.bodyLineEnds.length, `page ${page} line count`);
      for (const [j, [x, y]] of expected.bodyLineEnds.entries()) {
        const line = lines[j];
        if (line.report_x_offset !== 0 ||
            !within([firstX, x, y], [line.start_x * pt, line.end_x * pt, line.baseline * pt])) {
          differences.push({ page, line: j + 1, expected: [firstX, x, y], actual: line });
        }
      }
    }
    const origins = referenceGlyphs(name, expected)
      .filter(([text]) => compareAsGlyph(text))
      .map(([text, x, y]) => [visibleGlyph(text), x - offset, y]);
    const { missing, extra } = compareGlyphs(origins, glyphs[i], tolerance, registration?.glyphTolerancePt);
    if (missing.length || extra.length) differences.push({ page, missing, extra });
    const expectedImages = expected.images;
    const actualImages = images[i];
    if (!matchImageBounds(expectedImages, actualImages, within)) {
      differences.push({ page, expectedImages, actualImages });
    }
  }
  return differences;
}

const directories = (await readdir(corpus, { withFileTypes: true }))
  .filter(entry => entry.isDirectory()).map(entry => entry.name).sort();
assert(directories.length > 0, 'no Word coordinate fixtures found');
const names = process.argv.length > 2 ? process.argv.slice(2) : [...directories, ...archivedGlyphProbes.keys(), tallGridName];
const browser = await chromium.launch();
const failures = [];
try {
  const page = await browser.newPage();
  await page.route('https://word-coordinates.test/', route => route.fulfill({
    contentType: 'text/html', body: '<!doctype html><title>Word coordinate comparison</title>',
  }));
  const assets = new Map();
  await page.route('https://word-coordinates.test/wasm/**', async route => {
    const filename = new URL(route.request().url()).pathname.slice('/wasm/'.length);
    if (!/^(jade_docx\.js|jade_docx_bg\.wasm|snippets\/[a-z0-9-]+\/[a-z0-9]+\.js)$/.test(filename)) {
      await route.fulfill({ status: 404 });
      return;
    }
    if (!assets.has(filename)) assets.set(filename, await readFile(new URL(filename, wasm)));
    await route.fulfill({
      body: assets.get(filename),
      contentType: filename.endsWith('.wasm') ? 'application/wasm' : 'text/javascript',
    });
  });
  for (const name of names) {
    try {
      if (name === tallGridName) {
        await page.goto('https://word-coordinates.test/');
        const differences = await checkTallGrid(page);
        if (differences.length) {
          failures.push({ name, differences });
          console.error(`${name}: ${differences.length} pagination or baseline differences`);
        } else {
          console.log(`${name}: 265 pages and 4692 row baselines within ${tallGridTolerancePt} pt`);
        }
        continue;
      }
      const isArchivedProbe = archivedGlyphProbes.has(name);
      assert(isArchivedProbe || directories.includes(name), `unknown fixture: ${name}`);
      const { bytes, reference, manifest } = await (isArchivedProbe
        ? loadArchivedGlyphFixture(name)
        : loadSyntheticFixture(name));
      await page.goto('https://word-coordinates.test/');
      const actual = await capture(page, bytes, name);
      const differences = checkPages(name, reference, actual, manifest);
      if (differences.length) {
        failures.push({ name, differences });
        const pages = [...new Set(differences.map(difference => difference.page))];
        console.error(`${name}: coordinates differ on pages ${pages.join(', ')}`);
      } else {
        console.log(`${name}: ${manifest.pages} pages within ${manifest.axisTolerancePt} pt`);
      }
    } catch (error) {
      failures.push({ name, error: error.message });
      console.error(`${name}: ${error.message}`);
    }
  }
} finally {
  await browser.close();
}
console.log(`${names.length - failures.length}/${names.length} Word coordinate fixtures passed`);
if (failures.length) {
  await mkdir(new URL('.', report), { recursive: true });
  await writeFile(report, JSON.stringify(failures, null, 2) + '\n');
  process.exitCode = 1;
}
