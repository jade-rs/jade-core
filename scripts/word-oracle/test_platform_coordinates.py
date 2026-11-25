"""Regression checks for platform selection, immutable inputs and strict XY gates."""
import copy
import gzip
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from zipfile import ZipFile

import check_platform_coordinates as check


class PlatformCoordinates(unittest.TestCase):
    def test_inherited_right_original_retains_complete_authoring_matrix_and_word_build(self):
        name = 'probe-math-display-alignment-0'
        self.assertEqual(check.INHERITED_RIGHT_MATH_CASES, {name})
        self.assertLessEqual(check.INHERITED_RIGHT_MATH_CASES, check.PHYSICAL_FONT_CASES)
        row, = [r for r in check.verify() if r[0] == name]
        _, source, archive, document, drawing, font_source = row
        self.assertEqual(font_source, 'original-word-pdf')
        with ZipFile(archive) as bundle:
            capture = json.loads(bundle.read('capture.json').decode('utf-8-sig'))
        self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
        self.assertEqual(capture['word']['executable_sha256'],
            '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
        self.assertEqual(capture['capture_id'], check.REQUIRED_CAPTURES[name])
        import xml.etree.ElementTree as ET
        from itertools import product
        ns = {'w': 'http://schemas.openxmlformats.org/wordprocessingml/2006/main',
              'm': 'http://schemas.openxmlformats.org/officeDocument/2006/math'}
        with ZipFile(source) as bundle:
            xml = ET.fromstring(bundle.read('word/document.xml'))
            settings = ET.fromstring(bundle.read('word/settings.xml'))
        self.assertEqual(settings.find('.//m:dispDef', ns).get('{'+ns['m']+'}val'), '0')
        cases = []
        for paragraph in xml.findall('.//w:p', ns):
            math = paragraph.find('m:oMathPara', ns)
            if math is None:
                continue
            bidi = paragraph.find('w:pPr/w:bidi', ns).get('{'+ns['w']+'}val')
            alignment = paragraph.find('w:pPr/w:jc', ns).get('{'+ns['w']+'}val')
            explicit = math.find('m:oMathParaPr/m:jc', ns)
            cases.append((bidi, alignment, None if explicit is None else
                          explicit.get('{'+ns['m']+'}val')))
        self.assertEqual(len(cases), 32)
        self.assertEqual(set(cases), set(product(('0', '1'),
            ('left', 'right', 'start', 'end'), (None, 'left', 'center', 'right'))))

    def test_paragraph_mark_math_preserves_original_word_version_and_font_gate(self):
        name = 'probe-math-paragraph-mark-size'
        self.assertEqual(check.PARAGRAPH_MARK_MATH_CASES, {name})
        self.assertLessEqual(check.PARAGRAPH_MARK_MATH_CASES, check.PHYSICAL_FONT_CASES)
        row, = [r for r in check.verify() if r[0] == name]
        _, source, archive, document, drawing, font_source = row
        self.assertEqual(font_source, 'original-word-pdf')
        self.assertEqual(check.REQUIRED_CAPTURES[name], '20261114T004417585Z-59a68986')
        with ZipFile(archive) as bundle:
            capture = json.loads(bundle.read('capture.json').decode('utf-8-sig'))
        self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
        self.assertEqual(capture['word']['executable_sha256'],
            '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
        import xml.etree.ElementTree as ET
        with ZipFile(source) as bundle:
            xml = ET.fromstring(bundle.read('word/document.xml'))
        ns = {'w': 'http://schemas.openxmlformats.org/wordprocessingml/2006/main',
              'm': 'http://schemas.openxmlformats.org/officeDocument/2006/math'}
        paragraphs = [p for p in xml.findall('.//w:p', ns)
                      if p.find('m:oMathPara', ns) is not None]
        marks = [p.find('w:pPr/w:rPr/w:sz', ns) for p in paragraphs]
        self.assertEqual([None if m is None else m.get('{'+ns['w']+'}val')
                          for m in marks], [None, None, '20', '20', '32', '32'])
        self.assertFalse(xml.findall('.//m:f', ns))

    def test_rtl_merged_originals_require_each_word_build_fonts_and_vector_rules(self):
        self.assertEqual(len(check.RTL_MERGED_PERCENTAGE_CASES), 3)
        self.assertLessEqual(check.RTL_MERGED_PERCENTAGE_CASES, check.PHYSICAL_FONT_CASES)
        rows = [r for r in check.verify() if r[0] in check.RTL_MERGED_PERCENTAGE_CASES]
        self.assertEqual({r[0] for r in rows}, check.RTL_MERGED_PERCENTAGE_CASES)
        for name, source, archive, document, drawing, font_source in rows:
            with self.subTest(case=name), ZipFile(archive) as bundle:
                capture = json.loads(bundle.read('capture.json').decode('utf-8-sig'))
                self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
                self.assertEqual(capture['word']['executable_sha256'],
                    '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                self.assertEqual(check.REQUIRED_CAPTURES[name], capture['capture_id'])
                self.assertEqual(font_source, 'original-word-pdf')
                self.assertNotIn(name, check.BORDERLESS_GRID_CASES | check.BORDERLESS_MATH_CASES)

    def test_fraction_originals_keep_word_build_font_and_vector_assertions(self):
        names = check.MATH_FRACTION_RULE_CASES | check.BORDERLESS_MATH_CASES
        self.assertEqual(len(names), 20)
        self.assertEqual(len(check.BORDERLESS_MATH_CASES), 9)
        self.assertLessEqual(names, check.PHYSICAL_FONT_CASES)
        rows = [r for r in check.verify() if r[0] in names]
        self.assertEqual({r[0] for r in rows}, names)
        for name, source, archive, document, drawing, font_source in rows:
            self.assertEqual(font_source, 'original-word-pdf')
            with ZipFile(archive) as bundle:
                capture = json.loads(bundle.read('capture.json').decode('utf-8-sig'))
                self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
                self.assertEqual(capture['capture_id'], check.REQUIRED_CAPTURES[name])
            import xml.etree.ElementTree as ET
            with ZipFile(source) as bundle:
                xml = ET.fromstring(bundle.read('word/document.xml'))
            ns = {'m': 'http://schemas.openxmlformats.org/officeDocument/2006/math'}
            fractions = xml.findall('.//m:f', ns)
            self.assertTrue(fractions)
            kinds = [f.find('m:fPr/m:type', ns) for f in fractions]
            no_bars = all(k is not None and k.get('{'+ns['m']+'}val') == 'noBar' for k in kinds)
            self.assertEqual(no_bars, name in check.BORDERLESS_MATH_CASES)

    def test_numbering_originals_keep_each_word_version_and_physical_font_gate(self):
        names = {
            'numbering-alignment-matrix-mac',
            'numbering-legal-format-matrix-mac',
            'numbering-textbox-full-matrix-mac',
            'numbering-table-header-mac',
            'numbering-table-nested-list-mac',
            'numbering-table-matrix-mac',
        }
        registry = check.read_json(check.ROOT / check.INDEX)
        cases = [r for r in registry['fixtures'] if r['mac_case'] in names]
        self.assertEqual({r['mac_case'] for r in cases}, names)
        self.assertEqual(names, check.NUMBERING_LAYOUT_CASES)
        self.assertLessEqual(names, check.PHYSICAL_FONT_CASES)
        for case in cases:
            with self.subTest(case=case['mac_case']):
                mac = check.read_json(check.checked_file(check.ROOT, case['mac_manifest']))
                self.assertEqual(mac['reference']['version'], '16.114.4')
                self.assertEqual(mac['reference']['platform'], 'mac')
                self.assertEqual(len(case['windows']), 1)
                windows = case['windows'][0]
                self.assertEqual(windows['word_product_version'], '16.0.19127.20936')
                self.assertEqual(windows['word_executable_sha256'],
                                 '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                self.assertEqual(windows['capture_id'], '20261114T004334255Z-b74a0cd3')
                self.assertEqual(check.REQUIRED_CAPTURES[case['mac_case']], windows['capture_id'])
                self.assertEqual(windows['font_source'], 'original-word-pdf')

    def test_preferred_saved_grids_keep_versioned_originals_fonts_and_borders(self):
        expected = {
            'probe-width-single-pct-2500-pct-2500': (3, 107),
            'probe-width-unequal-dxa-2400-dxa-2000': (2, 347),
            'probe-width-unequal-pct-2500-auto-0': (2, 347),
            'probe-width-unequal-pct-2500-dxa-2000': (2, 347),
            'probe-unequal-variant-narrow-first-2-9-fixed': (2, 240),
            'probe-unequal-variant-three-24-fixed': (2, 665),
            'probe-unequal-variant-narrow-first-2-9-no-header': (2, 237),
            'probe-unequal-variant-three-24-no-header': (2, 653),
            'probe-unequal-continuation-border-16': (2, 653),
            'probe-unequal-continuation-border-4': (2, 653),
        }
        self.assertEqual(check.PREFERRED_GRID_BORDER_CASES, set(expected))
        self.assertLessEqual(set(expected), check.PHYSICAL_FONT_CASES)
        rows = {row[0]: row for row in check.verify() if row[0] in expected}
        self.assertEqual(set(rows), set(expected))
        for name, (pages, glyphs) in expected.items():
            _, _, archive, _, drawing, font_source = rows[name]
            with self.subTest(case=name), ZipFile(archive) as bundle:
                capture = json.loads(bundle.read('capture.json').decode('utf-8-sig'))
                self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
                self.assertEqual(capture['capture_id'], check.REQUIRED_CAPTURES[name])
                self.assertEqual(font_source, 'original-word-pdf')
                self.assertEqual(drawing['page_count'], pages)
                self.assertEqual(sum(not g[1].isspace() for g in drawing['glyphs']), glyphs)

    def test_borderless_original_requires_explicit_empty_rule_expectation(self):
        from compare_windows_borders import rectangles
        import hashlib
        names = {'probe-unequal-continuation-border-0'}
        self.assertEqual(check.BORDERLESS_GRID_CASES, names)
        rows = [row for row in check.verify() if row[0] in names]
        self.assertEqual({row[0] for row in rows}, names)
        for name, _, archive, document, drawing, font_source in rows:
            self.assertEqual(drawing['page_count'], 2)
            self.assertEqual(sum(not g[1].isspace() for g in drawing['glyphs']), 653)
            self.assertEqual(font_source, 'original-word-pdf')
            with ZipFile(archive) as bundle, tempfile.TemporaryDirectory() as directory:
                data = bundle.read(document['pdf'])
                self.assertEqual(hashlib.sha256(data).hexdigest(), document['pdf_sha256'])
                original = Path(directory) / 'original.pdf'
                original.write_bytes(data)
                self.assertEqual(rectangles(original), [[], []])
        empty = dict(page=1, word_rectangles=0, native_rectangles=0,
                     word_uncovered_count=0, native_uncovered_count=0)
        with patch('compare_windows_borders.compare', return_value=[empty]):
            self.assertEqual(check.assert_original_vector_borders(
                None, None, 'nil', expected_empty=True), [empty])
            with self.assertRaisesRegex(ValueError, 'original-vector-borders'):
                check.assert_original_vector_borders(None, None, 'positive-rule-case')
        for bad in [[], [dict(empty, word_rectangles=1)],
                    [dict(empty, native_rectangles=1)],
                    [dict(empty, word_uncovered_count=1)],
                    [dict(empty, native_uncovered_count=1)]]:
            with patch('compare_windows_borders.compare', return_value=bad):
                with self.assertRaisesRegex(ValueError, 'original-vector-borders'):
                    check.assert_original_vector_borders(None, None, 'nil', expected_empty=True)

    def test_vector_border_gate_rejects_missing_extra_or_empty_ink(self):
        good = dict(page=1, word_rectangles=3, native_rectangles=4,
                    word_uncovered_count=0, native_uncovered_count=0)
        with patch('compare_windows_borders.compare', return_value=[good]):
            self.assertEqual(check.assert_original_vector_borders(None, None, 'table'), [good])
        with patch('compare_windows_borders.TOLERANCE_PT', .5):
            with self.assertRaisesRegex(ValueError, 'tolerance changed'):
                check.assert_original_vector_borders(None, None, 'table')
        invalid = [[], [dict(good, word_uncovered_count=1)],
                   [dict(good, native_uncovered_count=1)],
                   [dict(good, word_rectangles=0)], [dict(good, native_rectangles=0)],
                   [good, dict(good, page=2, native_uncovered_count=1)]]
        for results in invalid:
            with self.subTest(results=results), patch(
                    'compare_windows_borders.compare', return_value=results):
                with self.assertRaisesRegex(ValueError, 'original-vector-borders'):
                    check.assert_original_vector_borders(None, None, 'table')

    def test_word_display_aliases_retain_distinct_embedded_postscript_styles(self):
        row = next(r for r in check.verify() if r[0] == 'aptos-ascii-advances-mac')
        _, _, manifest, document, _, _ = row
        original = manifest.parent/document['pdf']
        faces = {g[4] for g in check.pdf_origins(original, font_metrics=True)['glyphs']}
        self.assertLessEqual({'Aptos', 'Aptos-Italic', 'Aptos-Bold', 'Aptos-BoldItalic',
                              'AptosDisplay', 'AptosDisplay-Italic', 'AptosDisplay-Bold',
                              'AptosDisplay-BoldItalic'}, faces)
        # The aliases come from each embedded program, retaining the actual
        # bold/italic identity rather than collapsing all styles to a family.
        self.assertNotIn('Aptos,Italic', faces)
        self.assertNotIn('Aptos Display,Italic', faces)

    def test_type3_vector_fonts_retain_declared_face_size_and_origins(self):
        import pymupdf
        def write_pdf(path, face):
            with pymupdf.open() as doc:
                page = doc.new_page(width=200, height=200)
                def obj(value, stream=None):
                    xref = doc.get_new_xref()
                    doc.update_object(xref, value)
                    if stream is not None:
                        doc.update_stream(xref, stream)
                    return xref
                glyph = obj('<< >>', b'600 0 0 0 600 700 d1 0 0 600 700 re f')
                cmap = obj('<< >>', b'/CIDInit /ProcSet findresource begin 12 dict begin begincmap '
                           b'/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def '
                           b'/CMapName /Test def /CMapType 2 def 1 begincodespacerange '
                           b'<00> <FF> endcodespacerange 1 beginbfchar <41> <0041> endbfchar '
                           b'endcmap CMapName currentdict /CMap defineresource pop end end')
                descriptor = ''
                if face is not None:
                    ref = obj('<< /Type /FontDescriptor /FontName /ABCDEF+' + face
                              + ' /Flags 4 /ItalicAngle 0 /CapHeight 700 /StemV 80 >>')
                    descriptor = f'/FontDescriptor {ref} 0 R'
                font = obj(f'<< /Type /Font /Subtype /Type3 /FontMatrix [.001 0 0 .001 0 0] '
                           f'/FontBBox [0 0 600 700] /FirstChar 65 /LastChar 65 /Widths [600] '
                           f'/Encoding << /Type /Encoding /Differences [65 /A] >> '
                           f'/CharProcs << /A {glyph} 0 R >> /Resources << >> '
                           f'/ToUnicode {cmap} 0 R {descriptor} >>')
                contents = obj('<< >>', b'BT /F1 10 Tf 50 100 Td (A) Tj ET')
                doc.xref_set_key(page.xref, 'Resources', f'<< /Font << /F1 {font} 0 R >> >>')
                page.set_contents(contents)
                doc.save(path)
        with tempfile.TemporaryDirectory() as directory:
            expected = dict(page_count=1, glyphs=[[1, 'A', 50., 100., 'FixtureFace', 10.]])
            for face in ['FixtureFace', 'DifferentFace', None]:
                path = Path(directory)/(str(face)+'.pdf')
                write_pdf(path, face)
                with self.subTest(face=face):
                    if face is None:
                        with self.assertRaisesRegex(ValueError, 'Type3 font descriptor'):
                            check.pdf_origins(path, font_metrics=True)
                        continue
                    actual = check.pdf_origins(path, font_metrics=True)
                    self.assertEqual(actual['glyphs'][0][4], face)
                    self.assertAlmostEqual(actual['glyphs'][0][5], 10.)
                    if face == 'FixtureFace':
                        self.assertEqual(check.compare(expected, actual, 'Type3 face', font_metrics=True), 1)
                    else:
                        with self.assertRaises(ValueError):
                            check.compare(expected, actual, 'wrong Type3 face', font_metrics=True)

    def test_original_print_cases_require_face_size_and_origin_assertions(self):
        required = {'2col-mac', 'align-center-mac', 'alignment-mac', 'aptos-ascii-advances-mac', 'aptos-kerning-mac', 'aptos-numeric-ranges-mac', 'aptos-range-compat-14-en-US-mac', 'aptos-range-compat-15-en-US-mac', 'ascii-punctuation-mac', 'borderless-table-mac', 'cell-alignment-mac', 'char-spacing-mac', 'column-break-mac', 'column-table-fixed-preferred-width-mac', 'column-table-fixed-preferred-width-smaller-mac', 'courier-new-line-metrics', 'courier-new-regular-adjacent', 'decimal-tabs-mac', 'direction-with-selection-mac', 'endnote-mac', 'eqruby-grid-mac', 'eqruby-nogrid-mac', 'exact-and-minimum-lines-mac', 'floating-inside-outside-mac', 'floating-simple-position-mac', 'floating-square-wrap-mac', 'floating-through-concave-mac', 'floating-tight-polygon-mac', 'floating-top-bottom-offset-mac', 'floating-wrap-sides-mac', 'floating-z-order-mac', 'footnote-long-continuation-mac', 'footnote-mac', 'footnote-table-each-page-mac', 'footnote-table-textbox-order-mac', 'fractional-picture-anchors-mac', 'header-footer-edge-cases-mac', 'header-footer-sectionpages-mac', 'header-footer-sections-mac', 'header-tab-overflow-long-mac', 'indents-mac', 'inline-image-position-grid-mac', 'inline-image-position-nogrid-mac', 'inline-image-rotation-table-picture-only-mac', 'jade-script-baseline-20261030', 'kana-mac', 'keep-mac', 'latin-break-budget-620', 'latin-break-budget-638', 'latin-break-budget-645', 'latin-break-budget-652', 'latin-break-budget-659', 'line-numbering-mac', 'line-spacing-mac', 'merged-table-over-page-mac', 'mirror-margins-mac', 'mirror-margins-parity-mac', 'nested-after-spacing-oversized-48-0', 'nested-after-spacing-repeat-header', 'nested-after-spacing-short-5pt', 'nested-merged-tables-mac', 'note-styles-creation-mac', 'numbering-by-style-mac', 'numbering-restart-mac', 'odd-even-stories-mac', 'paragraph-spacing-mac', 'probe-column-fixed-dxa-4400', 'probe-column-fixed-dxa-6800', 'probe-contextual-paragraph-spacing-mode14', 'probe-contextual-paragraph-spacing-mode15', 'probe-continuous-column-explicit-break', 'probe-fixed-auto-table-grid-1000-cell-pct-2500', 'probe-fixed-auto-table-grid-2000-cell-pct-1250', 'probe-fixed-auto-table-grid-2000-cell-pct-5000', 'probe-fixed-auto-table-grid-4000-cell-pct-2500', 'probe-hyphenation-split-word-sweep-off', 'probe-paragraph-border-wrap', 'probe-second-column-centered-table', 'probe-table-flow-cell-spacing-start-column-1', 'probe-table-flow-continuous-start-column-1', 'probe-table-flow-continuous-start-column-2', 'probe-table-flow-continuous-vertical-merge-start-column-1', 'probe-table-flow-continuous-vertical-merge-start-column-2', 'probe-table-flow-split-rows-continuous-start-column-1', 'probe-table-flow-split-rows-start-column-1', 'probe-table-flow-split-rows-start-column-2', 'probe-table-flow-start-column-1', 'probe-table-flow-start-column-2', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-1', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-2', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-3', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-1', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-2', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-3', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-1', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-2', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-3', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-1', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-2', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-3', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-1', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-2', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-3', 'probe-table-flow-unequal-continuous-right-three-columns-start-column-1', 'probe-table-flow-unequal-continuous-right-three-columns-start-column-2', 'probe-table-flow-unequal-continuous-right-three-columns-start-column-3', 'probe-table-flow-unequal-right-start-column-1', 'probe-table-flow-unequal-right-start-column-2', 'probe-table-flow-unequal-start-column-1', 'probe-table-flow-unequal-start-column-2', 'probe-unequal-autofit-section-width-matrix', 'probe-unequal-autofit-wrapped-header', 'probe-unequal-center-narrow-first-1-1', 'probe-unequal-center-narrow-first-1-9', 'probe-unequal-center-narrow-first-2-9', 'probe-unequal-center-three-10', 'probe-unequal-center-three-13', 'probe-unequal-center-three-17', 'probe-unequal-center-three-24', 'probe-unequal-center-three-4', 'probe-unequal-center-wide-first-1-1', 'probe-unequal-center-wide-first-1-9', 'probe-unequal-center-wide-first-2-9', 'probe-unequal-control-1-16-with-tail', 'probe-unequal-control-1-16-without-tail', 'probe-unequal-control-1-21-with-tail', 'probe-unequal-control-1-21-without-tail', 'probe-unequal-control-1-22-with-tail', 'probe-unequal-control-1-22-without-tail', 'probe-unequal-control-1-23-with-tail', 'probe-unequal-control-1-23-without-tail', 'probe-unequal-control-1-24-without-tail', 'probe-unequal-control-2-12-with-tail', 'probe-unequal-control-2-12-without-tail', 'probe-unequal-control-2-19-with-tail', 'probe-unequal-control-2-19-without-tail', 'probe-unequal-control-3-14-with-tail', 'probe-unequal-control-3-14-without-tail', 'probe-unequal-control-3-8-with-tail', 'probe-unequal-control-3-8-without-tail', 'probe-unequal-explicit-paragraph-width-3300', 'probe-unequal-explicit-paragraph-width-3566', 'probe-unequal-explicit-paragraph-width-3800', 'probe-unequal-left-narrow-first-1-1', 'probe-unequal-left-narrow-first-1-9', 'probe-unequal-left-narrow-first-2-1', 'probe-unequal-left-narrow-first-2-9', 'probe-unequal-left-start-column-1', 'probe-unequal-left-wide-first-1-1', 'probe-unequal-left-wide-first-1-9', 'probe-unequal-left-wide-first-2-1', 'probe-unequal-left-wide-first-2-9', 'probe-unequal-right-narrow-first-1-1', 'probe-unequal-right-narrow-first-1-9', 'probe-unequal-right-narrow-first-2-9', 'probe-unequal-right-three-10', 'probe-unequal-right-three-17', 'probe-unequal-right-three-24', 'probe-unequal-right-three-4', 'probe-unequal-right-three-7', 'probe-unequal-right-wide-first-1-1', 'probe-unequal-right-wide-first-1-9', 'probe-unequal-right-wide-first-2-9', 'probe-unequal-variant-narrow-first-2-9-center', 'probe-unequal-variant-narrow-first-2-9-dxa', 'probe-unequal-variant-narrow-first-2-9-left', 'probe-unequal-variant-narrow-first-2-9-long', 'probe-unequal-variant-narrow-first-2-9-right', 'probe-unequal-variant-three-24-center', 'probe-unequal-variant-three-24-dxa', 'probe-unequal-variant-three-24-left', 'probe-unequal-variant-three-24-long', 'probe-unequal-variant-three-24-right', 'probe-width-equal-auto-0-pct-2500', 'probe-width-equal-pct-2500-auto-0', 'probe-width-equal-pct-2500-dxa-2000', 'probe-width-equal-pct-2500-pct-2500', 'probe-width-equal-pct-2500-pct-5000', 'probe-width-equal-pct-5000-pct-2500', 'probe-width-short-narrow-first-1-1', 'probe-width-short-narrow-first-1-4', 'probe-width-short-narrow-first-1-5', 'probe-width-short-narrow-first-1-9', 'probe-width-short-narrow-first-2-1', 'probe-width-short-narrow-first-2-4', 'probe-width-short-narrow-first-2-5', 'probe-width-short-narrow-first-2-9', 'probe-width-short-wide-first-1-1', 'probe-width-short-wide-first-1-4', 'probe-width-short-wide-first-1-5', 'probe-width-short-wide-first-1-9', 'probe-width-short-wide-first-2-1', 'probe-width-short-wide-first-2-4', 'probe-width-short-wide-first-2-5', 'probe-width-short-wide-first-2-9', 'probe-width-single-auto-0-pct-2500', 'probe-width-single-dxa-2400-dxa-2000', 'probe-width-unequal-auto-0-pct-2500', 'probe-width-unequal-pct-2500-pct-2500', 'probe-width-unequal-pct-2500-pct-5000', 'probe-width-unequal-pct-5000-pct-2500', 'ruby-grid-mac', 'ruby-nogrid-mac', 'ruby-wide-grid-mac', 'ruby-wide-nogrid-mac', 'run-styles-mac', 'script-position-mac', 'section-break-types-mac', 'sections-mac', 'story-inline-picture-metrics-mac', 'table-bidi-alignment-mac', 'table-bidi-visual-mac', 'table-border-conflicts-mac', 'table-boundary-cant-split-mac', 'table-cell-fit-text-mac', 'table-diagonal-borders-mac', 'table-flow-continuous-start-column-1-mac', 'table-flow-continuous-vertical-merge-start-column-2-mac', 'table-flow-split-rows-start-column-1-mac', 'table-hidden-rows-mac', 'table-hide-mark-mac', 'table-legacy-hmerge-mac', 'table-oversized-row-after-spacing-fresh-fit', 'table-repeating-header-mac', 'table-row-after-spacing-short-5pt', 'table-vertical-merge-page-break-mac', 'table3x2merged-mac', 'tabs-geometry-mac', 'tabs-mac', 'textbox-body-properties-mac', 'textbox-exact-line-matrix-mac', 'textbox-fixed-pitch-matrix-mac', 'textbox-tall-header-overlap-mac', 'toc-instrtext-edge-cases-mac', 'toc-sequence-page-number-mac', 'underline-kinds-mac', 'unequal-columns-mac', 'vert-align-sweep-2-browser-mac', 'vert-align-sweep-browser-mac', 'widow-control-mac', 'word-compat-comprehensive-mac', 'word-mac-probe-distribute-single', 'word-mac-probe-float-lookahead', 'word-mac-probe-floating-horizontal-14', 'word-mac-probe-floating-horizontal-15', 'word-mac-probe-grid-line-unit', 'word-mac-probe-grid-line-unit-table', 'word-mac-probe-indent', 'word-mac-probe-numbering-label', 'word-mac-probe-numbering-label-faces', 'word-mac-probe-object-space-runs-unbalanced', 'word-mac-probe-pairs-kerned', 'word-mac-probe-squeeze-conditions-nocompress', 'word-mac-probe-squeeze-plain', 'word-mac-probe-vert-align-sweep-2', 'wrong-num-mac', 'probe-math-display-alignment-1', 'probe-math-display-rtl-wrap-1', 'probe-math-italic-advances-lowercase', 'probe-math-italic-advances-uppercase', 'probe-math-relation-wrap-after', 'probe-math-relation-wrap-before', 'probe-math-relation-wrap-repeat',
                             'probe-width-single-pct-2500-pct-2500',
                             'probe-width-unequal-dxa-2400-dxa-2000',
                             'probe-width-unequal-pct-2500-auto-0',
                             'probe-width-unequal-pct-2500-dxa-2000',
}
        required |= {'probe-complex-font-metrics-thonburi',
                     'probe-complex-font-sweep-thonburi', 'probe-devanagari-line-box-devanagari', 'probe-mixed-sangam-matrix-240', 'probe-mixed-sangam-matrix-276'}
        required |= {'numbering-textbox-full-matrix-mac', 'numbering-alignment-matrix-mac', 'numbering-legal-format-matrix-mac'}
        required |= {'numbering-table-nested-list-mac', 'numbering-table-header-mac', 'numbering-table-matrix-mac'}
        required |= {'header-tab-overflow-short-mac'}
        required.add('fixed-table-width-matrix-mac')
        required |= {'probe-unequal-variant-narrow-first-2-9-no-header', 'probe-unequal-variant-three-24-no-header', 'probe-unequal-continuation-border-0', 'probe-unequal-continuation-border-16', 'probe-unequal-continuation-border-4'}
        required |= {'probe-unequal-variant-narrow-first-2-9-fixed', 'probe-unequal-variant-three-24-fixed'}
        required |= {'probe-table-flow-unequal-automatic-grid-start-column-1', 'probe-table-flow-unequal-automatic-grid-start-column-2'}
        required |= {'probe-math-fraction-first-vs-prev',
         'probe-math-fraction-larger-neighbors',
         'probe-math-fraction-leading',
         'probe-math-fraction-neighbors',
         'probe-math-fraction-size-20',
         'probe-math-fraction-size-24',
         'probe-math-fraction-size-28',
         'probe-math-fraction-size-32',
         'probe-math-fraction-size-36',
         'probe-math-fraction-size-40',
         'probe-math-fraction-spacing-matrix',
         'probe-math-repeat-bar-fraction-24',
         'probe-math-repeat-bar-fraction-32',
         'probe-math-repeat-fraction-20',
         'probe-math-repeat-fraction-24',
         'probe-math-repeat-fraction-28',
         'probe-math-repeat-fraction-36',
         'probe-math-repeat-fraction-40',
         'probe-math-standalone-fractions-12',
         'probe-math-standalone-fractions-explicit-sizes'}
        required |= {'probe-table-flow-unequal-continuous-vertical-merge-three-columns-rtl-section-start-column-1',
         'probe-table-flow-unequal-continuous-vertical-merge-three-columns-rtl-section-start-column-2',
         'probe-table-flow-unequal-continuous-vertical-merge-three-columns-rtl-section-start-column-3'}
        required |= {'probe-math-paragraph-mark-size'}
        required |= {'probe-math-display-alignment-0'}
        self.assertEqual(required, check.PHYSICAL_FONT_CASES)
        self.assertEqual(check.PHYSICAL_FONT_CASES, {row[0] for row in check.verify()})

    def test_three_originals_retain_actual_mac_pdf_and_versioned_windows_input(self):
        import hashlib
        from collections import Counter
        facts = [{'name': 'tabs-geometry-mac', 'source_sha256': '4b2ab628789bfb0502aa779951fe4f94358a782b5e8f1fdf9b2e8d90deb358de', 'capture_id': '20261114T130556208Z-6d6ad7b2', 'Mac_pages': 1, 'Windows_pages': 1, 'Mac_pdf': {'file': 'tests/word-oracle/mac-glyph-corpus-20261019/pdf/tabs.pdf', 'sha256': 'f2d3ff3dceea7c9fd34195ae2d93d2e1947579099daab3d22d7f825d1c3814ba'}, 'Windows_pdf_sha256': '0b6b2f0f6f55481793599ebf37ec82a34b5b7bd426cf4b154c5a04e409e85e0e'}, {'name': '2col-mac', 'source_sha256': '02e7f6a0da7e08ab377c854bb0fc1b1085298900266c063a473bcab134e20ff8', 'capture_id': '20261114T231659226Z-87aed8b1', 'Mac_pages': 1, 'Windows_pages': 2, 'Mac_pdf': {'file': 'tests/word-oracle/mac-glyph-corpus-20261019/pdf/2col.pdf', 'sha256': 'd46d124de6b33cc3204a79d7ab4cd3d6d473963d7ebc7c8b8424ce53c7d9f7dc'}, 'Windows_pdf_sha256': '044f5f10e4984d17f9f02843c8a67a8f9b7e1de04c55e114dc6c027fe5092d1e'}, {'name': 'char-spacing-mac', 'source_sha256': '6064f2d4aa131ac92bc583e1e0c119e955b3d32af49222cf640801ab3a04e2c1', 'capture_id': '20261114T231718277Z-f02fe908', 'Mac_pages': 1, 'Windows_pages': 1, 'Mac_pdf': {'file': 'tests/word-oracle/mac-glyph-corpus-20261019/pdf/char-spacing.pdf', 'sha256': 'db73aabac2e3e75e58fa91411046627cbea7d49705d0d30dbde2232a585d2501'}, 'Windows_pdf_sha256': 'bcb285d6c8c4afb22787dead9411c48f604e67e5bf5fc87309a3860097a1c5e8'}]
        rows = check.verify()
        for fact in facts:
            name = fact['name']
            with self.subTest(case=name):
                self.assertEqual(check.REQUIRED_CAPTURES[name], fact['capture_id'])
                row, = [r for r in rows if r[0] == name and r[2].parent.name == fact['capture_id']]
                _, source, capture_file, document, _, font_source = row
                capture = check.read_json(capture_file)
                self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), fact['source_sha256'])
                self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
                self.assertEqual(capture['word']['application_build'], '16.0.19127')
                self.assertEqual(capture['word']['executable_sha256'], '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                self.assertEqual(capture['capture_id'], fact['capture_id'])
                self.assertEqual(document['pages'], fact['Windows_pages'])
                self.assertEqual(document['pdf_sha256'], fact['Windows_pdf_sha256'])
                self.assertEqual(font_source, 'original-word-pdf')
                self.assertEqual((capture_file.parent/'inputs'/(document['name']+'.docx')).read_bytes(), source.read_bytes())
                manifest = check.read_json(source.parent/'manifest.json')
                pdf = check.checked_file(check.ROOT, fact['Mac_pdf'])
                raw = check.read_json(pdf.parent.parent/(pdf.stem+'.json'))
                self.assertEqual(raw['docx_sha256'], fact['source_sha256'])
                self.assertEqual(raw['word_version'], '16.114')
                self.assertEqual(raw['word_build'], '16.114.26101123')
                self.assertEqual(raw['pdf_sha256'], fact['Mac_pdf']['sha256'])
                self.assertEqual(manifest['reference']['pdfSha256'], raw['pdf_sha256'])
                self.assertEqual(manifest['pages'], fact['Mac_pages'])
                pages = json.loads(gzip.decompress((source.parent/'pages.json.gz').read_bytes()))
                expected = Counter((i,*g) for i,p in enumerate(pages,1) for story in ('header','body','footer') for g in p[story])
                observed = Counter((g[0],g[1],round(g[2],3),round(g[3],3)) for g in check.pdf_origins(pdf)['glyphs'] if not g[1].isspace())
                self.assertEqual(expected, observed)
                if name == '2col-mac':
                    self.assertEqual((manifest['pages'], document['pages']), (1,2))

    def test_three_original_captures_cannot_be_removed(self):
        original = check.read_json
        for name in ['2col-mac', 'char-spacing-mac', 'tabs-geometry-mac']:
            index = copy.deepcopy(original(check.ROOT/check.INDEX))
            index['fixtures'] = [x for x in index['fixtures'] if x['mac_case'] != name]
            with self.subTest(case=name), patch.object(check, 'read_json', side_effect=lambda path: index if path == check.ROOT/check.INDEX else original(path)):
                with self.assertRaisesRegex(ValueError, 'Shared fixture coverage removed'):
                    check.verify()

    def test_three_original_captures_reject_wrong_word_version(self):
        original = check.read_json
        for name in ['2col-mac', 'char-spacing-mac', 'tabs-geometry-mac']:
            index = copy.deepcopy(original(check.ROOT/check.INDEX))
            case = next(x for x in index['fixtures'] if x['mac_case'] == name)
            case['windows'][0]['word_product_version'] = '16.0.19127.20860'
            with self.subTest(case=name), patch.object(check, 'read_json', side_effect=lambda path: index if path == check.ROOT/check.INDEX else original(path)):
                with self.assertRaisesRegex(ValueError, 'Windows Word version changed'):
                    check.verify()

    def test_physical_font_metrics_reject_size_or_face_changes_at_same_origin(self):
        word = dict(page_count=1, glyphs=[[1, 'A', 50., 80., 'ArialMT', 10.56]])
        self.assertEqual(check.compare(word, copy.deepcopy(word), 'font', font_metrics=True), 1)
        for font, size in [('ArialMT', 10.5), ('Calibri', 10.56),
                           ('ArialMT', float('nan')), ('ArialMT', 0.)]:
            jade = dict(page_count=1, glyphs=[[1, 'A', 50., 80., font, size]])
            with self.subTest(font=font, size=size), self.assertRaises(ValueError):
                check.compare(word, jade, 'font drift', font_metrics=True)

    def test_physical_font_matching_keeps_geometry_and_distinct_partners(self):
        word = dict(page_count=1, glyphs=[[1, 'A', x, 80., 'ArialMT', 10.56]
                                         for x in (0., .2, 1.)])
        jade = copy.deepcopy(word)
        jade['glyphs'].reverse()
        self.assertEqual(check.compare(word, jade, 'reordered paint', font_metrics=True), 3)
        jade['glyphs'][0][2] += .376
        with self.assertRaises(ValueError):
            check.compare(word, jade, 'moved font', font_metrics=True)
        jade = copy.deepcopy(word)
        jade['glyphs'][1][2] = 1.
        with self.assertRaises(ValueError):
            check.compare(word, jade, 'duplicate font', font_metrics=True)

    def test_font_metric_pdf_extraction_retains_real_original_sizes(self):
        pdf = check.ROOT / 'tests/word-oracle/mac-glyph-corpus-20261019/pdf/word-compat-comprehensive.pdf'
        plain = check.pdf_origins(pdf)
        metrics = check.pdf_origins(pdf, font_metrics=True)
        self.assertEqual(plain['glyphs'], [g[:4] for g in metrics['glyphs']])
        self.assertEqual(check.compare(metrics, metrics, 'original font', font_metrics=True), 1427)


    def test_comprehensive_keeps_same_input_versions_and_actual_mac_pdf(self):
        from collections import Counter
        name = 'word-compat-comprehensive-mac'
        self.assertIn(name, check.INDEPENDENT_OUTPUT_CASES)
        self.assertEqual(check.REQUIRED_CAPTURES[name], '20261114T004202539Z-7cf29738')
        _, source, archive, document, drawing, fonts = next(row for row in check.verify() if row[0] == name)
        raw = check.read_json(check.ROOT / 'tests/word-oracle/mac-glyph-corpus-20261019/word-compat-comprehensive.json')
        self.assertEqual(raw['docx_sha256'], '8f415f8bb3c647433142b355e3ae4c85e94f1d16e6da99c479c1e464e7f1b256')
        self.assertEqual(document['sha256'], raw['docx_sha256'])
        self.assertEqual(raw['word_version'], '16.114')
        self.assertEqual(raw['word_build'], '16.114.26101123')
        self.assertEqual(raw['pdf_sha256'], '8eed62f2dddc77025367200ae3140bc77c484d820a8fa03ba9d6fc0c5e624068')
        manifest = check.read_json(source.parent / 'manifest.json')
        self.assertEqual(manifest['reference']['version'], raw['word_version'])
        self.assertEqual(manifest['reference']['build'], raw['word_build'])
        self.assertEqual(manifest['reference']['pdfSha256'], raw['pdf_sha256'])
        pdf = check.checked_file(check.ROOT, dict(file='tests/word-oracle/mac-glyph-corpus-20261019/pdf/word-compat-comprehensive.pdf', sha256=raw['pdf_sha256']))
        pages = json.loads(gzip.decompress((source.parent / 'pages.json.gz').read_bytes()))
        golden = [(i, *g) for i, p in enumerate(pages, 1) for story in ('header', 'body', 'footer') for g in p[story]]
        observed = [(g[0], g[1], round(g[2], 3), round(g[3], 3)) for g in check.pdf_origins(pdf)['glyphs'] if not g[1].isspace()]
        self.assertEqual(Counter(golden), Counter(observed))
        self.assertEqual(len(golden), 1427)
        self.assertEqual(fonts, 'original-word-pdf')
        self.assertEqual(document['pages'], 2)
        check.checked_file(check.ROOT, dict(file=archive.relative_to(check.ROOT).as_posix(), sha256='186eafa1c44e8f2baab5244f7a26a802b4b0c51870b4736e62ec45b6661e6e78'))

    def test_comprehensive_original_cannot_be_removed(self):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        index['fixtures'] = [c for c in index['fixtures'] if c['mac_case'] != 'word-compat-comprehensive-mac']
        with patch.object(check, 'read_json', side_effect=lambda path: index if path == check.ROOT / check.INDEX else original(path)):
            with self.assertRaisesRegex(ValueError, 'Shared fixture coverage removed'):
                check.verify()

    def test_five_output_controls_preserve_versioned_same_input_originals(self):
        from collections import Counter
        facts = [{'name': 'floating-inside-outside-mac', 'source_sha256': '7928b10c2849bac71a61b74b282cdc700afb1344e77dbc9f1134fdd6c7a79312', 'Mac_pdf_sha256': 'fb8f0d699f67a0d5bdb63ab5147d1daeafd273d54a7ef5d38cee6dfd4588e52e', 'capture_id': '20261114T004143028Z-b4cb9887', 'archive_sha256': 'd1cc6d692fca913877eca1615ee8dfedb9c2f51d83b3268a4c0e6f10c19e9913'}, {'name': 'header-footer-edge-cases-mac', 'source_sha256': 'd6c325b84a42e5df290543623f000e00eafdbdbe72322b2f56fd391ba7a37d69', 'Mac_pdf_sha256': '69c1531d3c0f13639328725bfeedccac891b0e2b8d92209bdcec1ac3fb1e35ce', 'capture_id': '20261114T004143028Z-b4cb9887', 'archive_sha256': 'd1cc6d692fca913877eca1615ee8dfedb9c2f51d83b3268a4c0e6f10c19e9913'}, {'name': 'textbox-body-properties-mac', 'source_sha256': 'a360a8668dc3d745f0d617be078ca16197ab6d58e7c1aee7687802d74e1182cc', 'Mac_pdf_sha256': '415f3a50685e702ef1efa03cb5811ff10172fe9a7773d6e70ee61ead3031c288', 'capture_id': '20261114T004202539Z-7cf29738', 'archive_sha256': '186eafa1c44e8f2baab5244f7a26a802b4b0c51870b4736e62ec45b6661e6e78'}, {'name': 'textbox-tall-header-overlap-mac', 'source_sha256': '544e9bedd0958929959c11e46a520afe98f534d39e8466ac9d4f0c5ec05b3898', 'Mac_pdf_sha256': '5c83c9c02678792f3d19083a5da182fcefa8b2e8767e462415b4ffa6a52abc47', 'capture_id': '20261114T004202539Z-7cf29738', 'archive_sha256': '186eafa1c44e8f2baab5244f7a26a802b4b0c51870b4736e62ec45b6661e6e78'}, {'name': 'toc-instrtext-edge-cases-mac', 'source_sha256': '3dbe67594235f371babbe632095e927e2d961a975d4830291dc60db57d1a3fba', 'Mac_pdf_sha256': 'baefbb18951e0d57d8080cd5a91199881e807041b9707a4ad97d333554bae746', 'capture_id': '20261114T004202539Z-7cf29738', 'archive_sha256': '186eafa1c44e8f2baab5244f7a26a802b4b0c51870b4736e62ec45b6661e6e78'}]
        verified = {row[0]: row for row in check.verify()}
        for fact in facts:
            with self.subTest(name=fact['name']):
                name = fact['name']
                self.assertIn(name, check.INDEPENDENT_OUTPUT_CASES)
                self.assertEqual(check.REQUIRED_CAPTURES[name], fact['capture_id'])
                _, source, archive, document, drawing, fonts = verified[name]
                raw = check.read_json(check.ROOT / (
                    'tests/word-oracle/mac-glyph-corpus-20261019/'
                    + name.removesuffix('-mac') + '.json'))
                manifest = check.read_json(source.parent / 'manifest.json')
                self.assertEqual(raw['docx_sha256'], fact['source_sha256'])
                self.assertEqual(document['sha256'], fact['source_sha256'])
                self.assertEqual(raw['word_version'], '16.114')
                self.assertEqual(raw['word_build'], '16.114.26101123')
                self.assertEqual(manifest['reference']['version'], raw['word_version'])
                self.assertEqual(manifest['reference']['build'], raw['word_build'])
                self.assertEqual(raw['pdf_sha256'], fact['Mac_pdf_sha256'])
                self.assertEqual(manifest['reference']['pdfSha256'], raw['pdf_sha256'])
                pdf = check.checked_file(check.ROOT, dict(file=(
                    'tests/word-oracle/mac-glyph-corpus-20261019/pdf/'
                    + name.removesuffix('-mac') + '.pdf'), sha256=raw['pdf_sha256']))
                pages = json.loads(gzip.decompress((source.parent / 'pages.json.gz').read_bytes()))
                golden = [(i, *g) for i, page in enumerate(pages, 1)
                          for story in ('header', 'body', 'footer') for g in page[story]]
                observed = [(g[0], g[1], round(g[2], 3), round(g[3], 3))
                            for g in check.pdf_origins(pdf)['glyphs'] if not g[1].isspace()]
                self.assertEqual(Counter(golden), Counter(observed))
                self.assertTrue(golden)
                self.assertEqual(fonts, 'original-word-pdf')
                check.checked_file(check.ROOT, dict(file=archive.relative_to(check.ROOT).as_posix(),
                                                   sha256=fact['archive_sha256']))

    def test_each_new_output_control_is_required(self):
        names = ['floating-inside-outside-mac', 'header-footer-edge-cases-mac', 'textbox-body-properties-mac', 'textbox-tall-header-overlap-mac', 'toc-instrtext-edge-cases-mac']
        original = check.read_json
        for name in names:
            with self.subTest(name=name):
                index = copy.deepcopy(original(check.ROOT / check.INDEX))
                index['fixtures'] = [c for c in index['fixtures'] if c['mac_case'] != name]
                with patch.object(check, 'read_json', side_effect=lambda path:
                                  index if path == check.ROOT / check.INDEX else original(path)):
                    with self.assertRaisesRegex(ValueError, 'Shared fixture coverage removed'):
                        check.verify()

    def test_tight_polygon_preserves_same_input_versions_and_original_mac_pdf_baselines(self):
        from collections import Counter
        name = 'floating-tight-polygon-mac'
        self.assertIn(name, check.INDEPENDENT_OUTPUT_CASES)
        self.assertEqual(check.REQUIRED_CAPTURES[name], '20261114T004143028Z-b4cb9887')
        _, source, archive, document, drawing, fonts = next(
            row for row in check.verify() if row[0] == name)
        original = check.read_json(check.ROOT / (
            'tests/word-oracle/mac-glyph-corpus-20261019/floating-tight-polygon.json'))
        manifest = check.read_json(source.parent / 'manifest.json')
        self.assertEqual(original['docx_sha256'], document['sha256'])
        self.assertEqual(original['word_version'], '16.114')
        self.assertEqual(original['word_build'], '16.114.26101123')
        self.assertEqual(manifest['reference']['version'], original['word_version'])
        self.assertEqual(manifest['reference']['build'], original['word_build'])
        self.assertEqual(manifest['reference']['pdfSha256'], original['pdf_sha256'])
        pdf = check.checked_file(check.ROOT, dict(file=(
            'tests/word-oracle/mac-glyph-corpus-20261019/pdf/floating-tight-polygon.pdf'),
            sha256=original['pdf_sha256']))
        pages = json.loads(gzip.decompress((source.parent / 'pages.json.gz').read_bytes()))
        golden = [(i, *g) for i, page in enumerate(pages, 1)
                  for story in ('header', 'body', 'footer') for g in page[story]]
        # An old JSON row groups two baselines only0.24pt apart. The unchanged
        # Mac golden preserves the PDF's actual per-character origins.
        observed = [(g[0], g[1], round(g[2], 3), round(g[3], 3))
                    for g in check.pdf_origins(pdf)['glyphs'] if not g[1].isspace()]
        self.assertEqual(Counter(golden), Counter(observed))
        self.assertEqual(len(golden), 432)
        self.assertEqual(fonts, 'original-word-pdf')
        self.assertEqual(document['pages'], 1)
        self.assertTrue(archive.is_file())

    def test_tight_polygon_version_expectation_cannot_be_removed(self):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        index['fixtures'] = [case for case in index['fixtures']
                             if case['mac_case'] != 'floating-tight-polygon-mac']
        with patch.object(check, 'read_json', side_effect=lambda path:
                          index if path == check.ROOT / check.INDEX else original(path)):
            with self.assertRaisesRegex(ValueError, 'Shared fixture coverage removed'):
                check.verify()

    def test_float_lookahead_keeps_same_input_originals_and_all_four_planes(self):
        name = 'word-mac-probe-float-lookahead'
        self.assertNotIn(name, check.INDEPENDENT_OUTPUT_CASES)
        self.assertEqual(check.REQUIRED_CAPTURES[name], '20261114T004202539Z-7cf29738')
        _, source, archive, document, drawing, fonts = next(
            row for row in check.verify() if row[0] == name)
        self.assertEqual(fonts, 'original-word-pdf')
        original = check.read_json(check.ROOT / (
            'tests/word-oracle/mac-probes-20261019/' + name + '.json'))
        manifest = check.read_json(source.parent / 'manifest.json')
        self.assertEqual(document['sha256'], original['docx_sha256'])
        self.assertEqual(manifest['reference']['version'], original['word_version'])
        self.assertEqual(manifest['reference']['build'], original['word_build'])
        self.assertEqual(original['word_build'], '16.114.26101123')
        self.assertEqual(manifest['reference']['pdfSha256'], original['pdf_sha256'])
        pages = json.loads(gzip.decompress((source.parent / 'pages.json.gz').read_bytes()))
        expected = [dict(size=[page['width_pt'], page['height_pt']],
                         body=[[g['text'], g['x_pt'], row['baseline_pt']]
                               for row in page['rows'] for g in row['characters']])
                    for page in original['pages']]
        self.assertEqual(len(pages), 5)
        for page, reference in zip(pages, expected):
            self.assertEqual(page['size'], reference['size'])
            self.assertEqual(page['body'], reference['body'])
            self.assertFalse(page['header'] or page['footer'])
        self.assertEqual(document['pages'], 5)
        self.assertEqual(len(drawing['glyphs']), 1930)
        self.assertTrue(archive.is_file())

    def test_float_lookahead_version_expectation_cannot_be_removed(self):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        index['fixtures'] = [case for case in index['fixtures']
                             if case['mac_case'] != 'word-mac-probe-float-lookahead']
        with patch.object(check, 'read_json', side_effect=lambda path:
                          index if path == check.ROOT / check.INDEX else original(path)):
            with self.assertRaisesRegex(ValueError, 'Shared fixture coverage removed'):
                check.verify()

    def test_registered_originals_and_both_platform_references(self):
        cases = check.verify()
        self.assertGreaterEqual(len(cases), 115)
        self.assertGreaterEqual(sum(case[2].suffix == '.zip' for case in cases), 91)

    def test_section_start_spacing_preserves_versioned_original_outputs(self):
        name = 'header-footer-sectionpages-mac'
        self.assertIn(name, check.INDEPENDENT_OUTPUT_CASES)
        self.assertEqual(check.REQUIRED_CAPTURES[name], '20261114T004143028Z-b4cb9887')
        index = check.read_json(check.ROOT / check.INDEX)
        case = next(case for case in index['fixtures'] if case['mac_case'] == name)
        windows, = case['windows']
        self.assertEqual(windows['word_product_version'], '16.0.19127.20936')
        self.assertEqual(windows['word_executable_sha256'],
                         '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
        self.assertEqual(windows['output_context'], 'independent-screen-print')
        manifest = check.read_json(check.ROOT / case['mac_manifest']['file'])
        self.assertEqual(manifest['reference']['version'], '16.114')
        self.assertEqual(manifest['reference']['build'], '16.114.26101123')
        self.assertEqual(manifest['coordinatesSha256'],
                         'af42cc67cd23b5846fcbd791c8dfccdf8c7219e8cb6a8c25703e344311f7993f')
        self.assertEqual(case['source']['sha256'],
                         '9904e5c52526c3a8fd897b78747c865a06d17d656f3e7c5a12c891c3d2cbd65f')
        self.assertEqual(len([case for case in check.verify() if case[0] == name]), 1)

    def test_legacy_floating_text_edges_keep_original_platform_exports(self):
        name = 'word-mac-probe-floating-horizontal-14'
        self.assertNotIn(name, check.INDEPENDENT_OUTPUT_CASES)
        self.assertEqual(check.REQUIRED_CAPTURES[name], '20261114T004202539Z-7cf29738')
        _, source, archive, document, drawing, fonts = next(
            case for case in check.verify() if case[0] == name)
        self.assertEqual(fonts, 'original-word-pdf')
        original = check.read_json(check.ROOT / (
            'tests/word-oracle/mac-floating-horizontal-20261030/' + name + '.json'))
        directory = check.ROOT / 'tests/word-oracle/mac-elsa-synthetic' / name
        manifest = check.read_json(directory / 'manifest.json')
        self.assertEqual(document['sha256'], original['docx_sha256'])
        self.assertEqual(manifest['reference']['version'], original['word_version'])
        self.assertNotIn('build', manifest['reference'])  # Preserve the existing manifest.
        self.assertEqual(original['word_build'], '16.114.26102345')
        self.assertEqual(manifest['reference']['pdfSha256'], original['pdf_sha256'])
        pages = json.loads(gzip.decompress((directory / 'pages.json.gz').read_bytes()))
        expected = [dict(size=[page['width_pt'], page['height_pt']], header=[],
                         body=[[g['text'], g['x_pt'], g['baseline_pt']]
                               for row in page['rows'] for g in row['characters']], footer=[], images=[])
                    for page in original['pages']]
        self.assertEqual(pages, expected)
        with ZipFile(archive) as bundle, tempfile.TemporaryDirectory() as temporary:
            pdf = Path(temporary) / 'original-word.pdf'
            pdf.write_bytes(bundle.read(document['pdf']))
            check.compare(drawing, check.pdf_origins(pdf), name + '/original-EMF-PDF')
        self.assertTrue(source.is_file())

    def test_grid_line_units_preserve_original_mac_and_windows_exports(self):
        cases = {case[0]: case for case in check.verify()}
        for name in ('word-mac-probe-grid-line-unit',
                     'word-mac-probe-grid-line-unit-table'):
            with self.subTest(name=name):
                self.assertNotIn(name, check.INDEPENDENT_OUTPUT_CASES)
                self.assertEqual(check.REQUIRED_CAPTURES[name],
                                 '20261114T004214564Z-4ebc9049')
                _, source, archive, document, drawing, fonts = cases[name]
                self.assertEqual(fonts, 'original-word-pdf')
                original = check.read_json(check.ROOT / (
                    'tests/word-oracle/mac-probes-20261107/' + name + '.json'))
                directory = check.ROOT / 'tests/word-oracle/mac-elsa-synthetic' / name
                manifest = check.read_json(directory / 'manifest.json')
                self.assertEqual(document['sha256'], original['docx_sha256'])
                self.assertEqual(manifest['reference']['version'], original['word_version'])
                self.assertEqual(manifest['reference']['build'], original['word_build'])
                self.assertEqual(manifest['reference']['pdfSha256'], original['pdf_sha256'])
                pages = json.loads(gzip.decompress((directory / 'pages.json.gz').read_bytes()))
                expected = [dict(size=[page['width_pt'], page['height_pt']], header=[],
                                 body=[[glyph['text'], glyph['x_pt'], glyph['baseline_pt']]
                                       for row in page['rows'] for glyph in row['characters']],
                                 footer=[]) for page in original['pages']]
                self.assertEqual(pages, expected)  # No rounded or native-derived golden.
                self.assertEqual(len(pages), original['page_count'])
                with ZipFile(archive) as bundle, tempfile.TemporaryDirectory() as temporary:
                    pdf = Path(temporary) / 'original-word.pdf'
                    pdf.write_bytes(bundle.read(document['pdf']))
                    check.compare(drawing, check.pdf_origins(pdf), name + '/original-EMF-PDF')
                self.assertTrue(source.is_file())

    def test_grid_line_unit_version_expectations_cannot_be_removed(self):
        original = check.read_json
        for name in ('word-mac-probe-grid-line-unit',
                     'word-mac-probe-grid-line-unit-table'):
            index = copy.deepcopy(original(check.ROOT / check.INDEX))
            index['fixtures'] = [case for case in index['fixtures'] if case['mac_case'] != name]
            with self.subTest(name=name), patch.object(
                    check, 'read_json', side_effect=lambda path:
                    index if path == check.ROOT / check.INDEX else original(path)):
                with self.assertRaisesRegex(ValueError, 'Shared fixture coverage removed'):
                    check.verify()

    def test_windows_kerned_punctuation_keeps_shared_four_plane_assertions(self):
        name = 'word-mac-probe-pairs-kerned'
        self.assertNotIn(name, check.INDEPENDENT_OUTPUT_CASES)
        self.assertEqual(check.REQUIRED_CAPTURES[name], '20261114T004214564Z-4ebc9049')
        cases = [case for case in check.verify() if case[0] == name]
        self.assertEqual(len(cases), 1)
        _, source, archive, document, drawing, font_source = cases[0]
        self.assertEqual(font_source, 'original-word-pdf')
        self.assertEqual(document['sha256'],
                         'f6bc744c18f1098e7945c231a42b87af37335461eab8a42b96eb5132d41a4562')
        self.assertEqual(len(drawing['glyphs']), 96)
        with ZipFile(archive) as bundle, tempfile.TemporaryDirectory() as temporary:
            pdf = Path(temporary) / 'original-word.pdf'
            pdf.write_bytes(bundle.read(document['pdf']))
            check.compare(drawing, check.pdf_origins(pdf), name + '/original-EMF-PDF')
        self.assertTrue(source.is_file())

    def test_windows_kerned_punctuation_reference_cannot_be_removed(self):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        index['fixtures'] = [case for case in index['fixtures']
                             if case['mac_case'] != 'word-mac-probe-pairs-kerned']
        with patch.object(check, 'read_json', side_effect=lambda path:
                          index if path == check.ROOT / check.INDEX else original(path)):
            with self.assertRaisesRegex(ValueError, 'Shared fixture coverage removed'):
                check.verify()

    def test_existing_four_plane_assertions_cannot_be_changed_to_independent_outputs(self):
        self.reject_index_edit(lambda c: c['windows'][0].update(
            output_context='independent-screen-print'), 'coordinate assertions changed')

    def test_original_independent_output_cases_cannot_lose_their_context(self):
        original = check.read_json
        for name in check.INDEPENDENT_OUTPUT_CASES:
            for value in (None, 'shared-screen-pdf', 'unknown'):
                index = copy.deepcopy(original(check.ROOT / check.INDEX))
                windows, = next(c for c in index['fixtures'] if c['mac_case'] == name)['windows']
                if value is None:
                    windows.pop('output_context')
                else:
                    windows['output_context'] = value
                with self.subTest(case=name, context=value), patch.object(
                        check, 'read_json', side_effect=lambda path:
                        index if path == check.ROOT / check.INDEX else original(path)):
                    with self.assertRaisesRegex(ValueError, 'coordinate assertions changed'):
                        check.verify()

    def assert_output_models(self, *, screen=None, snapshot=None, physical=None,
                             unclipped=None, changed_pixels=False, self_comparison=False):
        drawing = dict(page_count=1, glyphs=[[1, 'A', 72., 80.]])
        word_pdf = dict(page_count=1, glyphs=[[1, 'A', 72.8, 80.]])
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            before, after, word, painted = [directory / name for name in
                                           ('screen.png', 'snapshot.png', 'word.pdf', 'native.pdf')]
            before.write_bytes(b'screen-pixels')
            after.write_bytes(b'changed' if changed_pixels else b'screen-pixels')
            word.write_bytes(b'original-word-pdf')
            painted.write_bytes(b'original-word-pdf' if self_comparison else b'independent-native-pdf')
            def origins(path, all_glyphs=False):
                if path == word:
                    return word_pdf
                return unclipped if all_glyphs and unclipped is not None else (
                    physical if physical is not None else word_pdf)
            with patch.object(check, 'pdf_origins', side_effect=origins):
                check.assert_independent_outputs(
                    drawing, screen if screen is not None else drawing,
                    snapshot if snapshot is not None else drawing, before, after, word, painted, 'outputs')

    def test_genuine_screen_print_differences_pass_their_own_output_assertions(self):
        self.assert_output_models()
        with self.assertRaisesRegex(ValueError, 'axis mismatch'):
            check.compare(dict(page_count=1, glyphs=[[1, 'A', 72.8, 80.]]),
                          dict(page_count=1, glyphs=[[1, 'A', 72., 80.]]), 'wrong output golden')

    def test_independent_contexts_are_supported_by_the_original_word_outputs(self):
        index = check.read_json(check.ROOT / check.INDEX)
        for case in index['fixtures']:
            if case['mac_case'] not in check.INDEPENDENT_OUTPUT_CASES:
                continue
            windows, = case['windows']
            archive, _, document, drawing = check.archive_reference(check.ROOT, case, windows)
            with self.subTest(case=case['mac_case']), tempfile.TemporaryDirectory() as temporary:
                path = Path(temporary) / 'word.pdf'
                with ZipFile(archive) as bundle:
                    path.write_bytes(bundle.read(document['pdf']))
                original_pdf = check.pdf_origins(path)
                # Word itself produces distinct origins for these outputs.
                # No native renderer is used to manufacture either expectation.
                if case['mac_case'] == 'table-hidden-rows-mac':
                    # These genuine Word outputs differ within the comparison
                    # tolerance. Native UI and print must still use their own
                    # originals: their offsets need not cancel each other.
                    self.assertEqual(drawing['page_count'], original_pdf['page_count'])
                    self.assertEqual([g[:2] for g in drawing['glyphs']],
                                     [g[:2] for g in original_pdf['glyphs']])
                    maximum_x = max(abs(a[2] - b[2]) for a, b in
                                    zip(drawing['glyphs'], original_pdf['glyphs']))
                    self.assertAlmostEqual(maximum_x, 0.2638610839843807, places=6)
                elif case['mac_case'] == 'table-cell-fit-text-mac':
                    # These independently captured Word screen/print originals
                    # differ within tolerance too. Pin their actual difference;
                    # native UI and PDF must still match their own originals.
                    self.assertEqual(drawing['page_count'], 1)
                    self.assertEqual(drawing['page_count'], original_pdf['page_count'])
                    self.assertEqual(len(drawing['glyphs']), 351)
                    self.assertEqual([g[:2] for g in drawing['glyphs']],
                                     [g[:2] for g in original_pdf['glyphs']])
                    pairs = list(zip(drawing['glyphs'], original_pdf['glyphs']))
                    self.assertAlmostEqual(max(abs(a[2] - b[2]) for a, b in pairs),
                                           0.3598864746093682, places=6)
                    self.assertAlmostEqual(max(abs(a[3] - b[3]) for a, b in pairs),
                                           0.02002685546875682, places=6)
                else:
                    for expected, actual in ((drawing, original_pdf), (original_pdf, drawing)):
                        with self.assertRaisesRegex(ValueError, 'axis mismatch'):
                            check.compare(expected, actual, 'original Word screen/print difference')

    def test_screen_and_exported_snapshot_must_both_match_word_emf(self):
        shifted = dict(page_count=1, glyphs=[[1, 'A', 73., 80.]])
        for models in ({'screen': shifted}, {'snapshot': shifted}):
            with self.subTest(models=models), self.assertRaisesRegex(ValueError, 'axis mismatch'):
                self.assert_output_models(**models)
        # Export must retain exact live origins even within the Word tolerance.
        with self.assertRaisesRegex(ValueError, 'changed live logical origins'):
            self.assert_output_models(snapshot=dict(page_count=1, glyphs=[[1, 'A', 72.1, 80.]]))

    def test_vector_pdf_checks_each_axis_missing_glyphs_pages_and_unclipped_origins(self):
        bad = [dict(page_count=1, glyphs=[[1, 'A', 73.176, 80.]]),
               dict(page_count=1, glyphs=[[1, 'A', 72.8, 80.376]]),
               dict(page_count=1, glyphs=[]),
               dict(page_count=2, glyphs=[[1, 'A', 72.8, 80.]])]
        for model in bad:
            for plane in ('physical', 'unclipped'):
                with self.subTest(plane=plane, model=model), self.assertRaises(ValueError):
                    self.assert_output_models(**{plane: model})

    def test_independent_outputs_reject_changed_live_pixels_and_word_self_comparison(self):
        with self.assertRaisesRegex(ValueError, 'changed live screen pixels'):
            self.assert_output_models(changed_pixels=True)
        with self.assertRaisesRegex(ValueError, 'Word PDF self-comparison'):
            self.assert_output_models(self_comparison=True)

    def reject_archive_edit(self, edit, message):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        case = next(c for c in index['fixtures'] if 'archive' in c['windows'][0])
        edit(case['windows'][0])
        with patch.object(check, 'read_json', side_effect=lambda path:
                          index if path == check.ROOT / check.INDEX else original(path)):
            with self.assertRaisesRegex(ValueError, message):
                check.verify()

    def test_archive_bytes_and_original_metadata_are_required(self):
        self.reject_archive_edit(lambda w: w['archive'].update(sha256='0' * 64),
                                 'Evidence changed')
        self.reject_archive_edit(lambda w: w.update(capture_sha256='0' * 64),
                                 'Original archived capture metadata changed')

    def test_archived_word_version_executable_and_capture_cannot_be_relabelled(self):
        self.reject_archive_edit(lambda w: w.update(word_product_version='0.0.0.0'),
                                 'Windows Word version changed')
        self.reject_archive_edit(lambda w: w.update(word_executable_sha256='0' * 64),
                                 'Windows Word executable changed')
        self.reject_archive_edit(lambda w: w.update(capture_id='other-capture'),
                                 'Archived capture identity changed')

    def test_archived_input_and_original_font_programs_cannot_be_substituted(self):
        self.reject_archive_edit(lambda w: w.update(fixture='other-input'),
                                 'Missing or duplicate archived Windows input')
        self.reject_archive_edit(lambda w: w.update(font_source='expected-positions'),
                                 'Unknown archived font source')

    def test_archive_reference_cannot_silently_choose_a_second_origin(self):
        self.reject_archive_edit(lambda w: w.update(drawing_reference=w['archive']),
                                 'Ambiguous original Windows reference')

    def test_old_mac_version_gaps_remain_original_provenance(self):
        index = check.read_json(check.ROOT / check.INDEX)
        archived = [c for c in index['fixtures'] if 'archive' in c['windows'][0]]
        missing = []
        for case in archived:
            manifest = check.read_json(check.checked_file(check.ROOT, case['mac_manifest']))
            if 'version' not in manifest['reference']:
                missing.append(case['mac_case'])
        self.assertEqual(set(missing), {'fixed-table-width-matrix-mac', 'column-table-fixed-preferred-width-mac', 'column-table-fixed-preferred-width-smaller-mac', 'courier-new-line-metrics', 'courier-new-regular-adjacent', 'inline-image-rotation-table-picture-only-mac', 'latin-break-budget-620', 'latin-break-budget-638', 'latin-break-budget-645', 'latin-break-budget-652', 'latin-break-budget-659', 'nested-after-spacing-oversized-48-0', 'nested-after-spacing-repeat-header', 'nested-after-spacing-short-5pt', 'probe-column-fixed-dxa-4400', 'probe-column-fixed-dxa-6800', 'probe-continuous-column-explicit-break', 'probe-fixed-auto-table-grid-1000-cell-pct-2500', 'probe-fixed-auto-table-grid-2000-cell-pct-1250', 'probe-fixed-auto-table-grid-2000-cell-pct-5000', 'probe-fixed-auto-table-grid-4000-cell-pct-2500', 'probe-second-column-centered-table', 'probe-table-flow-cell-spacing-start-column-1', 'probe-table-flow-continuous-start-column-1', 'probe-table-flow-continuous-start-column-2', 'probe-table-flow-continuous-vertical-merge-start-column-1', 'probe-table-flow-continuous-vertical-merge-start-column-2', 'probe-table-flow-split-rows-continuous-start-column-1', 'probe-table-flow-split-rows-start-column-1', 'probe-table-flow-split-rows-start-column-2', 'probe-table-flow-start-column-1', 'probe-table-flow-start-column-2', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-1', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-2', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-3', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-1', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-2', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-3', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-1', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-2', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-3', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-1', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-2', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-3', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-1', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-2', 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-3', 'probe-table-flow-unequal-continuous-right-three-columns-start-column-1', 'probe-table-flow-unequal-continuous-right-three-columns-start-column-2', 'probe-table-flow-unequal-continuous-right-three-columns-start-column-3', 'probe-table-flow-unequal-right-start-column-1', 'probe-table-flow-unequal-right-start-column-2', 'probe-table-flow-unequal-start-column-1', 'probe-table-flow-unequal-start-column-2', 'probe-unequal-autofit-section-width-matrix', 'probe-unequal-autofit-wrapped-header', 'probe-unequal-center-narrow-first-1-1', 'probe-unequal-center-narrow-first-1-9', 'probe-unequal-center-narrow-first-2-9', 'probe-unequal-center-three-10', 'probe-unequal-center-three-13', 'probe-unequal-center-three-17', 'probe-unequal-center-three-24', 'probe-unequal-center-three-4', 'probe-unequal-center-wide-first-1-1', 'probe-unequal-center-wide-first-1-9', 'probe-unequal-center-wide-first-2-9', 'probe-unequal-control-1-16-with-tail', 'probe-unequal-control-1-16-without-tail', 'probe-unequal-control-1-21-with-tail', 'probe-unequal-control-1-21-without-tail', 'probe-unequal-control-1-22-with-tail', 'probe-unequal-control-1-22-without-tail', 'probe-unequal-control-1-23-with-tail', 'probe-unequal-control-1-23-without-tail', 'probe-unequal-control-1-24-without-tail', 'probe-unequal-control-2-12-with-tail', 'probe-unequal-control-2-12-without-tail', 'probe-unequal-control-2-19-with-tail', 'probe-unequal-control-2-19-without-tail', 'probe-unequal-control-3-14-with-tail', 'probe-unequal-control-3-14-without-tail', 'probe-unequal-control-3-8-with-tail', 'probe-unequal-control-3-8-without-tail', 'probe-unequal-explicit-paragraph-width-3300', 'probe-unequal-explicit-paragraph-width-3566', 'probe-unequal-explicit-paragraph-width-3800', 'probe-unequal-left-narrow-first-1-1', 'probe-unequal-left-narrow-first-1-9', 'probe-unequal-left-narrow-first-2-1', 'probe-unequal-left-narrow-first-2-9', 'probe-unequal-left-start-column-1', 'probe-unequal-left-wide-first-1-1', 'probe-unequal-left-wide-first-1-9', 'probe-unequal-left-wide-first-2-1', 'probe-unequal-left-wide-first-2-9', 'probe-unequal-right-narrow-first-1-1', 'probe-unequal-right-narrow-first-1-9', 'probe-unequal-right-narrow-first-2-9', 'probe-unequal-right-three-10', 'probe-unequal-right-three-17', 'probe-unequal-right-three-24', 'probe-unequal-right-three-4', 'probe-unequal-right-three-7', 'probe-unequal-right-wide-first-1-1', 'probe-unequal-right-wide-first-1-9', 'probe-unequal-right-wide-first-2-9', 'probe-unequal-variant-narrow-first-2-9-center', 'probe-unequal-variant-narrow-first-2-9-dxa', 'probe-unequal-variant-narrow-first-2-9-left', 'probe-unequal-variant-narrow-first-2-9-long', 'probe-unequal-variant-narrow-first-2-9-right', 'probe-unequal-variant-three-24-center', 'probe-unequal-variant-three-24-dxa', 'probe-unequal-variant-three-24-left', 'probe-unequal-variant-three-24-long', 'probe-unequal-variant-three-24-right', 'probe-width-equal-auto-0-pct-2500', 'probe-width-equal-pct-2500-auto-0', 'probe-width-equal-pct-2500-dxa-2000', 'probe-width-equal-pct-2500-pct-2500', 'probe-width-equal-pct-2500-pct-5000', 'probe-width-equal-pct-5000-pct-2500', 'probe-width-short-narrow-first-1-1', 'probe-width-short-narrow-first-1-4', 'probe-width-short-narrow-first-1-5', 'probe-width-short-narrow-first-1-9', 'probe-width-short-narrow-first-2-1', 'probe-width-short-narrow-first-2-4', 'probe-width-short-narrow-first-2-5', 'probe-width-short-narrow-first-2-9', 'probe-width-short-wide-first-1-1', 'probe-width-short-wide-first-1-4', 'probe-width-short-wide-first-1-5', 'probe-width-short-wide-first-1-9', 'probe-width-short-wide-first-2-1', 'probe-width-short-wide-first-2-4', 'probe-width-short-wide-first-2-5', 'probe-width-short-wide-first-2-9', 'probe-width-single-auto-0-pct-2500', 'probe-width-single-dxa-2400-dxa-2000', 'probe-width-unequal-auto-0-pct-2500', 'probe-width-unequal-pct-2500-pct-2500', 'probe-width-unequal-pct-2500-pct-5000', 'probe-width-unequal-pct-5000-pct-2500', 'table-flow-continuous-start-column-1-mac', 'table-flow-continuous-vertical-merge-start-column-2-mac', 'table-flow-split-rows-start-column-1-mac', 'table-oversized-row-after-spacing-fresh-fit', 'table-row-after-spacing-short-5pt', 'textbox-exact-line-matrix-mac', 'vert-align-sweep-browser-mac', 'probe-math-display-alignment-1', 'probe-math-display-rtl-wrap-1', 'probe-math-italic-advances-lowercase', 'probe-math-italic-advances-uppercase', 'probe-math-relation-wrap-after', 'probe-math-relation-wrap-before', 'probe-math-relation-wrap-repeat', 'probe-complex-font-metrics-thonburi', 'probe-complex-font-sweep-thonburi', 'probe-devanagari-line-box-devanagari', 'probe-mixed-sangam-matrix-240', 'probe-mixed-sangam-matrix-276',
            'probe-unequal-variant-narrow-first-2-9-no-header',
            'probe-unequal-variant-three-24-no-header',
            'probe-unequal-continuation-border-0',
            'probe-unequal-continuation-border-16',
            'probe-unequal-continuation-border-4',
            'probe-unequal-variant-narrow-first-2-9-fixed',
            'probe-unequal-variant-three-24-fixed',
            'probe-table-flow-unequal-automatic-grid-start-column-1',
            'probe-table-flow-unequal-automatic-grid-start-column-2',
                             'probe-width-single-pct-2500-pct-2500',
                             'probe-width-unequal-dxa-2400-dxa-2000',
                             'probe-width-unequal-pct-2500-auto-0',
                             'probe-width-unequal-pct-2500-dxa-2000',
} | {'probe-math-fraction-first-vs-prev',
 'probe-math-fraction-larger-neighbors',
 'probe-math-fraction-leading',
 'probe-math-fraction-neighbors',
 'probe-math-fraction-size-20',
 'probe-math-fraction-size-24',
 'probe-math-fraction-size-28',
 'probe-math-fraction-size-32',
 'probe-math-fraction-size-36',
 'probe-math-fraction-size-40',
 'probe-math-fraction-spacing-matrix',
 'probe-math-repeat-bar-fraction-24',
 'probe-math-repeat-bar-fraction-32',
 'probe-math-repeat-fraction-20',
 'probe-math-repeat-fraction-24',
 'probe-math-repeat-fraction-28',
 'probe-math-repeat-fraction-36',
 'probe-math-repeat-fraction-40',
 'probe-math-standalone-fractions-12',
 'probe-math-standalone-fractions-explicit-sizes'} | {'probe-table-flow-unequal-continuous-vertical-merge-three-columns-rtl-section-start-column-1',
 'probe-table-flow-unequal-continuous-vertical-merge-three-columns-rtl-section-start-column-2',
 'probe-table-flow-unequal-continuous-vertical-merge-three-columns-rtl-section-start-column-3'} | {'probe-math-paragraph-mark-size'} | {'probe-math-display-alignment-0'})

    def test_original_word_platform_differences_are_not_regressions(self):
        # This identical DOCX has different origins in the two actual Word
        # exports. A shared golden would incorrectly fail one platform.
        name, _, _, _, windows, _ = next(
            case for case in check.verify() if case[0] == 'endnote-mac')
        directory = check.ROOT / 'tests/word-oracle/mac-elsa-synthetic' / name
        pages = json.loads(gzip.decompress((directory / 'pages.json.gz').read_bytes()))
        mac = dict(page_count=len(pages), glyphs=[
            [number, *glyph]
            for number, page in enumerate(pages, 1)
            for story in ('header', 'body', 'footer')
            for glyph in page[story]])
        self.assertEqual(check.compare(mac, mac, 'Mac original'), 12)
        self.assertEqual(check.compare(windows, windows, 'Windows original'), 12)
        for expected, actual in ((mac, windows), (windows, mac)):
            with self.subTest(expected=expected['glyphs'][0]), self.assertRaisesRegex(
                    ValueError, 'axis mismatch'):
                check.compare(expected, actual, 'wrong platform golden')

    def reject_index_edit(self, edit, message):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        edit(index['fixtures'][0])
        with patch.object(check, 'read_json', side_effect=lambda path:
                          index if path == check.ROOT / check.INDEX else original(path)):
            with self.assertRaisesRegex(ValueError, message):
                check.verify()

    def test_all_shared_originals_pass_independently_and_reject_xy_drift(self):
        # Exercise every imported pair, not just the example whose OS origins
        # differ. The native/Browser CI jobs render these same source bytes.
        for name, _, _, _, windows, _ in check.verify():
            directory = check.ROOT / 'tests/word-oracle/mac-elsa-synthetic' / name
            pages = json.loads(gzip.decompress((directory / 'pages.json.gz').read_bytes()))
            mac = dict(page_count=len(pages), glyphs=[
                [number, *glyph]
                for number, page in enumerate(pages, 1)
                for story in ('header', 'body', 'footer')
                for glyph in page[story]])
            for platform, reference in (('mac', mac), ('windows', windows)):
                with self.subTest(case=name, platform=platform):
                    self.assertGreater(check.compare(reference, reference, platform), 0)
                    for axis in (2, 3):
                        shifted = copy.deepcopy(reference)
                        for glyph in shifted['glyphs']:
                            glyph[axis] += 2
                        with self.assertRaisesRegex(ValueError, 'axis mismatch'):
                            check.compare(reference, shifted, f'{name}/{platform}/{axis}')

    def test_windows_reference_cannot_replace_mac_reference(self):
        self.reject_index_edit(lambda c: c.update(mac_manifest=c['windows'][0]['capture']),
                               'Wrong Mac manifest')

    def test_word_version_cannot_be_relabelled(self):
        self.reject_index_edit(lambda c: c['windows'][0].update(word_product_version='0.0.0.0'),
                               'Windows Word version changed')

    def test_unknown_font_source_is_rejected(self):
        self.reject_index_edit(lambda c: c['windows'][0].update(font_source='expected-positions'),
                               'Unknown font source')

    def test_mac_source_cannot_be_swapped(self):
        self.reject_index_edit(lambda c: c['source'].update(sha256='0' * 64), 'Evidence changed')

    def test_windows_expectations_cannot_be_removed(self):
        self.reject_index_edit(lambda c: c.update(windows=[]), 'Missing Windows')

    def test_duplicate_platform_cases_are_rejected(self):
        self.reject_index_edit(lambda c: c['windows'].append(c['windows'][0]), 'Duplicate Word')

    def test_registered_case_cannot_be_removed(self):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        index['fixtures'].pop()
        with patch.object(check, 'read_json', side_effect=lambda path:
                          index if path == check.ROOT / check.INDEX else original(path)):
            with self.assertRaisesRegex(ValueError, 'Shared fixture coverage removed'):
                check.verify()

    def test_each_axis_is_checked_without_translation(self):
        reference = dict(page_count=1, glyphs=[[1, 'A', 72., 80.], [1, 'B', 80., 80.]])
        for axis in (2, 3):
            with self.subTest(axis=axis):
                actual = copy.deepcopy(reference)
                for glyph in actual['glyphs']:
                    glyph[axis] += .375
                self.assertEqual(check.compare(reference, actual, 'boundary'), 2)
                actual['glyphs'][0][axis] += .0001
                with self.assertRaisesRegex(ValueError, 'axis'):
                    check.compare(reference, actual, 'drift')

    def test_missing_text_and_page_changes_fail(self):
        reference = dict(page_count=1, glyphs=[[1, 'A', 72., 80.]])
        for actual in [dict(page_count=2, glyphs=reference['glyphs']),
                       dict(page_count=1, glyphs=[]),
                       dict(page_count=1, glyphs=[[1, 'B', 72., 80.]]),
                       dict(page_count=1, glyphs=[[1, 'A', float('nan'), 80.]])]:
            with self.subTest(actual=actual), self.assertRaises(ValueError):
                check.compare(reference, actual, 'negative')

    def test_floating_text_near_a_superscript_keeps_strict_origin_matching(self):
        # Word's 28pt exact textbox and adjacent body superscript straddle
        # the old 0.25pt reading-line grouping boundary after PDF rounding.
        word = dict(page_count=1, glyphs=[[1, 'B', 352.27, 137.18],
                                         [1, '7', 75.14, 137.42]])
        jade = dict(page_count=1, glyphs=[[1, 'B', 352.15, 137.14],
                                         [1, '7', 75.18, 137.43]])
        self.assertEqual(check.compare(word, jade, 'adjacent origins'), 2)
        jade['glyphs'][0][3] = 137.18 + .376
        with self.assertRaisesRegex(ValueError, 'axis'):
            check.compare(word, jade, 'real displacement')

    def test_overlapping_repeated_characters_need_distinct_partners(self):
        word = dict(page_count=1, glyphs=[[1, 'A', x, 80] for x in (0, .4, .8)])
        jade = dict(page_count=1, glyphs=[[1, 'A', x, 80] for x in (.2, .6, -.2)])
        self.assertEqual(check.compare(word, jade, 'ambiguous nearest origin'), 3)
        # Every reference has a nearby candidate, but two references would
        # have to reuse the same actual glyph. An existence-only check fails.
        word['glyphs'] = [[1, 'A', x, 80] for x in (0, .2, 1)]
        jade['glyphs'] = [[1, 'A', x, 80] for x in (0, 1, 1)]
        with self.assertRaisesRegex(ValueError, 'distinct glyph'):
            check.compare(word, jade, 'duplicate cannot replace missing paint')


if __name__ == '__main__':
    unittest.main()
