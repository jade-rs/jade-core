import json
import re
from pathlib import Path
import tempfile
import unittest

from check_windows_font_coordinates import INDEX, ROOT, verify, has_visible_vector_ink
from check_platform_coordinates import compare
from generate_windows_footnote_probes import generate


class WindowsFontCoordinatesTests(unittest.TestCase):
    def test_fractional_white_paper_is_invisible_but_later_white_erasure_is_rejected(self):
        import pymupdf
        with tempfile.TemporaryDirectory() as directory:
            for kind in ('fractional-paper', 'text-erasure', 'image-erasure'):
                with self.subTest(kind=kind), pymupdf.open() as document:
                    page = document.new_page(width=595.5, height=842.25)
                    page.draw_rect(page.rect, color=None, fill=(1, 1, 1))
                    authored_paper = pymupdf.Rect(0, 0, 595.3, 841.9)
                    if kind == 'fractional-paper':
                        page.draw_rect(authored_paper, color=None, fill=(1, 1, 1))
                    if kind == 'image-erasure':
                        pixel = pymupdf.Pixmap(pymupdf.csRGB, pymupdf.IRect(0, 0, 1, 1), False)
                        pixel.clear_with(0)
                        page.insert_image(pymupdf.Rect(72, 72, 82, 82), pixmap=pixel)
                    else:
                        page.insert_text((72, 90), 'Visible text')
                    if kind != 'fractional-paper':
                        # A page-sized white fill after text/images is erasure,
                        # even though the previous check treated it as paper.
                        page.draw_rect(page.rect, color=None, fill=(1, 1, 1))
                    pdf = Path(directory) / (kind + '.pdf')
                    document.save(pdf)
                    self.assertEqual(has_visible_vector_ink(pdf), kind != 'fractional-paper')

    def test_filled_and_stroked_guides_are_detected_without_counting_white_paper(self):
        import pymupdf
        with tempfile.TemporaryDirectory() as directory:
            for kind in ('paper', 'stroke', 'filled-guide'):
                with self.subTest(kind=kind), pymupdf.open() as document:
                    page = document.new_page(width=612, height=792)
                    page.draw_rect(page.rect, color=None, fill=(1, 1, 1))
                    page.insert_text((72, 90), 'Table text')
                    if kind == 'stroke':
                        page.draw_line((72, 72), (180, 72), color=(.6, .6, .6), width=.75)
                    elif kind == 'filled-guide':
                        page.draw_rect(pymupdf.Rect(72, 72, 180, 72.75), color=None,
                                       fill=(.6, .6, .6))
                    pdf = Path(directory) / (kind + '.pdf')
                    document.save(pdf)
                    self.assertEqual(has_visible_vector_ink(pdf), kind != 'paper')

    def test_generated_probes_match_original_word_inputs_byte_for_byte(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 43)
            files = list(output.glob('*.docx'))
            self.assertEqual(len(files), 43)
            for source in files:
                with self.subTest(source=source.name):
                    self.assertEqual(source.read_bytes(),
                                     (ROOT / 'public/fixtures' / source.name).read_bytes())

    def test_arial_runtime_pairs_match_independent_source_font_facts(self):
        facts = json.loads((ROOT / 'tests/word-oracle/windows/arial-kerning-font-facts.json')
                           .read_text())
        self.assertEqual(facts['units_per_em'], 2048)
        self.assertEqual(facts['font_version'], 'Version 7.07')
        self.assertEqual(facts['font_sha256'],
                         'edfbd22a3d088fc2f5d7c79b8c9c732df6dc878a60f0a80be0f92a3c7a634404')
        expected = [(ord(p['left']) << 8 | ord(p['right']), p['units'])
                    for p in facts['pairs']]
        source = (ROOT / 'crates/jade-docx-layout/src/line_metrics/windows_arial_kern.rs')
        actual = [(int(pair, 16), int(units)) for pair, units in
                  re.findall(r'\(0x([0-9a-f]+), (-?\d+)\)', source.read_text())]
        self.assertEqual(len(expected), 96)
        self.assertEqual(expected, sorted(set(expected)))
        self.assertEqual(actual, expected)

    def test_original_evidence_and_coverage(self):
        self.assertEqual(len(verify()), 619)
        self.assertEqual(sum(bool(case[0].get('solid_paragraph_background')) for case in verify()), 185)

    def test_isolated_missing_font_captures_preserve_word_selected_faces(self):
        import pymupdf
        cases = [row for row in verify() if row[0]['name'] == 'hello'
                 or row[0]['name'].startswith('word-windows-edge-font-independent-')]
        self.assertEqual(len(cases), 10)
        for case, _, manifest, document, _ in cases:
            with self.subTest(name=case['name']):
                capture = json.loads(manifest.read_text(encoding='utf-8'))
                self.assertEqual(len(capture['documents']), 1)
                self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
                self.assertEqual(capture['word']['executable_sha256'],
                                 '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                self.assertEqual(document['open_mode'], 'normal-read-only')
                name = case['name']
                expected = 'Calibri' if any(token in name for token in
                    ('-swiss-', '-auto-', '-modern-')) else 'Cambria'
                with pymupdf.open(manifest.parent / document['pdf']) as pdf:
                    families = {font[3].split('+')[-1] for page in pdf for font in page.get_fonts()}
                self.assertEqual(families, {expected})

    def test_all_original_paragraph_only_break_controls_remain_registered(self):
        cases = [row for row in verify() if row[0]['capture']['file'].split('/')[-2] in {
            '20261114T185441333Z-8f9ddd27'}]
        self.assertEqual(len(cases), 26)
        self.assertEqual(sum(row[0]['glyph_count'] for row in cases), 640)
        for case, _, manifest, document, _ in cases:
            with self.subTest(name=case['name']):
                self.assertEqual(case['word_product_version'], '16.0.19127.20936')
                self.assertTrue(case['no_visible_vector_ink'])
                self.assertFalse(has_visible_vector_ink(manifest.parent / document['pdf']))

    def test_table_identity_and_row_exceptions_keep_original_word_evidence(self):
        cases = [row for row in verify() if row[0]['capture']['file'].split('/')[-2] in {
            '20261114T195153941Z-3fc093f1', '20261114T200241643Z-226501da',
            '20261114T200434720Z-56b2954e', '20261114T200901748Z-eb28e42c'}]
        self.assertEqual(len(cases), 70)
        self.assertEqual(sum(row[0]['glyph_count'] for row in cases), 560)
        for case, _, manifest, document, _ in cases:
            with self.subTest(name=case['name']):
                capture = json.loads(manifest.read_text())
                self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
                self.assertEqual(capture['word']['executable_sha256'],
                                 '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                self.assertEqual(document['open_mode'], 'normal-read-only')
                self.assertTrue(case['no_visible_vector_ink'])
                self.assertFalse(has_visible_vector_ink(manifest.parent / document['pdf']))

    def test_table_controls_preserve_original_paint_expectations(self):
        cases = [row for row in verify() if row[0]['capture']['file'].split('/')[-2] in {
            '20261114T145808245Z-5e0616c6', '20261114T150706450Z-bf0874d1',
            '20261114T153827842Z-26d28c84', '20261114T154407780Z-238f8f19',
            '20261114T154728212Z-b6643dea', '20261114T155441810Z-2a8f6bc2',
            '20261114T155628362Z-b94002af', '20261114T160240896Z-e6021239',
            '20261114T160607246Z-5c23befe'}]
        self.assertEqual(len(cases), 77)
        self.assertEqual(sum(bool(row[0].get('no_visible_vector_ink')) for row in cases), 70)
        for case, _, manifest, document, _ in cases:
            with self.subTest(name=case['name']):
                self.assertEqual(case['word_product_version'], '16.0.19127.20936')
                self.assertEqual(bool(case.get('no_visible_vector_ink')),
                                 not has_visible_vector_ink(manifest.parent / document['pdf']))

    def test_primary_style_identity_is_preserved_with_an_effects_copy(self):
        cases = [row for row in verify() if row[0]['capture']['file'].split('/')[-2]
                 == '20261114T203937561Z-a3f5a222']
        self.assertEqual(len(cases), 18)
        self.assertEqual(sum(row[0]['glyph_count'] for row in cases), 144)
        for case, _, manifest, document, _ in cases:
            with self.subTest(name=case['name']):
                self.assertEqual(case['word_product_version'], '16.0.19127.20936')
                self.assertEqual(document['open_mode'], 'normal-read-only')
                self.assertTrue(case['no_visible_vector_ink'])
                self.assertFalse(has_visible_vector_ink(manifest.parent / document['pdf']))

    def test_column_and_typography_controls_keep_original_no_ink_expectations(self):
        names = {'2col', 'align-center', 'char-spacing', 'character-styles'}
        cases = [row for row in verify() if row[0]['name'] in names]
        self.assertEqual({row[0]['name'] for row in cases}, names)
        self.assertEqual(sum(row[0]['glyph_count'] for row in cases), 2623)
        for case, _, manifest, document, _ in cases:
            with self.subTest(name=case['name']):
                capture = json.loads(manifest.read_text(encoding='utf-8'))
                self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
                self.assertEqual(capture['word']['executable_sha256'],
                                 '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                self.assertEqual(document['open_mode'], 'normal-read-only')
                self.assertTrue(case['no_visible_vector_ink'])
                self.assertFalse(has_visible_vector_ink(manifest.parent / document['pdf']))

    def test_removed_case_is_rejected(self):
        data = json.loads((ROOT / INDEX).read_text())
        data['fixtures'].pop()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / INDEX).parent.mkdir(parents=True)
            (root / INDEX).write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, 'coverage/expectations changed'):
                verify(root)

    def test_displacement_and_missing_paint_are_rejected(self):
        expected = dict(page_count=1, glyphs=[[1, 'W', 72, 80], [1, 'o', 82, 80]])
        for glyphs in ([[1, 'W', 72.376, 80], [1, 'o', 82, 80]],
                       [[1, 'W', 72, 80]],
                       [[1, 'o', 72, 80], [1, 'W', 82, 80]]):
            with self.assertRaises(ValueError):
                compare(expected, dict(page_count=1, glyphs=glyphs), 'font regression')
