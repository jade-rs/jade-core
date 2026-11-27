"""Protect immutable Word XPS evidence, platform provenance, and required XY gates."""
import copy
from io import BytesIO
import unittest
from unittest.mock import patch
from xml.etree import ElementTree as E
from zipfile import ZipFile

import check_windows_xps as gate


class WindowsXps(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.index = gate.read_json(gate.ROOT / gate.INDEX)
        cls.cases = gate.verify()

    def test_all_originals_versions_fonts_and_coverage(self):
        self.assertEqual(len(self.cases), 296)
        self.assertEqual(sum(required for _, _, required in self.cases), 283)
        self.assertEqual(sum(len(model['glyphs']) for _, model, _ in self.cases), 62464)
        self.assertEqual(self.index['mac_parent_reference']['version'], '16.114.3')
        self.assertEqual(self.index['word']['product_version'], '16.0.19127.20936')

    def test_every_required_original_passes_and_rejects_drift_on_each_axis(self):
        for row, model, required in self.cases:
            with self.subTest(case=row['name']):
                self.assertEqual(gate.assert_model(model, model, row['name'], required), 'PASS')
                for axis in (2, 3):
                    displaced = copy.deepcopy(model)
                    for glyph in displaced['glyphs']:
                        glyph[axis] += .376
                    if required:
                        with self.assertRaisesRegex(ValueError, 'axis mismatch'):
                            gate.assert_model(model, displaced, 'deliberate displacement', True)
                    else:
                        self.assertEqual(gate.assert_model(model, displaced, 'pending', False), 'DIFFERENCE')

    def test_pending_coordinates_cannot_hide_text_page_or_order_changes(self):
        expected = dict(page_count=1, glyphs=[[1, 'A', 72., 80.], [1, 'B', 80., 80.]])
        for required in (False, True):
            for actual in (dict(page_count=2, glyphs=expected['glyphs']),
                           dict(page_count=1, glyphs=expected['glyphs'][:1]),
                           dict(page_count=1, glyphs=list(reversed(expected['glyphs'])))):
                with self.subTest(required=required, actual=actual), self.assertRaises(ValueError):
                    gate.assert_model(expected, actual, 'structural mismatch', required)

    def test_required_cases_cannot_be_removed_or_replaced_by_pending_cases(self):
        removed = copy.deepcopy(self.index); removed['fixtures'].pop()
        with self.assertRaisesRegex(ValueError, 'coverage changed'):
            gate.locked_promotions(removed)
        swapped = copy.deepcopy(self.index)
        required = next(r for r in swapped['fixtures'] if r['coordinate_gate_promotion'] != 'pending')
        pending = next(r for r in swapped['fixtures'] if r['coordinate_gate_promotion'] == 'pending')
        required['coordinate_gate_promotion'], pending['coordinate_gate_promotion'] = (
            pending['coordinate_gate_promotion'], required['coordinate_gate_promotion'])
        with self.assertRaisesRegex(ValueError, 'Required XPS coordinate assertions changed'):
            gate.locked_promotions(swapped)

    def test_archived_originals_and_word_version_cannot_be_substituted(self):
        for edit, message in (
            (lambda d: d['archive'].update(sha256='0' * 64), 'Evidence changed'),
            (lambda d: d['word'].update(product_version='0.0.0.0'), 'version/executable/capture changed'),
            (lambda d: d['mac_parent_manifest'].update(sha256='0' * 64), 'Evidence changed'),
            (lambda d: d['original_font_facts'].update(sha256='0' * 64), 'Original Office font facts changed'),
        ):
            changed = copy.deepcopy(self.index); edit(changed); original = gate.read_json
            with patch.object(gate, 'read_json', side_effect=lambda path:
                              changed if path == gate.ROOT / gate.INDEX else original(path)):
                with self.assertRaisesRegex(ValueError, message):
                    gate.verify()

    def test_xps_parser_matches_independent_xml_indices(self):
        # Repeated characters have explicit advances for the first 47 glyphs.
        # Read those directly from the original XML rather than using MuPDF's
        # glyph interpretation as its own expected output.
        names = ['word-windows-positive-kern-bolditalic-s15-X48-m14',
                 'word-windows-positive-kern-regular-s16-V48-m14']
        with ZipFile(gate.checked_file(gate.ROOT, self.index['archive'])) as archive:
            for name in names:
                row, model, _ = next(c for c in self.cases if c[0]['name'] == name)
                with ZipFile(BytesIO(archive.read(row['xps']['file']))) as xps:
                    page = E.fromstring(xps.read(next(n for n in xps.namelist() if n.endswith('.fpage'))))
                self.assertEqual([n.attrib for n in page.iter() if n.tag.endswith('MatrixTransform')],
                                 [{'Matrix': '1.333333333,0,0,1.333333333,0,0'}])
                expected = []
                for node in page.iter():
                    if not node.tag.endswith('Glyphs') or node.attrib['UnicodeString'].isspace():
                        continue
                    attrs = node.attrib; text = attrs['UnicodeString']
                    self.assertEqual(len(text), 48)
                    x, y, em = (float(attrs[k]) for k in ('OriginX', 'OriginY', 'FontRenderingEmSize'))
                    widths = attrs['Indices'].split(';')
                    for i, char in enumerate(text):
                        expected.append([1, char, x, y])
                        if i < 47:
                            x += float(widths[i].split(',')[1]) * em / 100
                self.assertEqual(len(expected), len(model['glyphs']))
                for original, parsed in zip(expected, model['glyphs'], strict=True):
                    self.assertEqual(original[:2], parsed[:2])
                    for axis in (2, 3):
                        self.assertLess(abs(original[axis] - parsed[axis]), .001)


if __name__ == '__main__':
    unittest.main()
