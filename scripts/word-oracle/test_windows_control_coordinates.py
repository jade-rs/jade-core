"""Reject lost source controls, altered visible text and unsearchable control ink."""
import collections
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import check_windows_control_coordinates as check


class WindowsControlCoordinates(unittest.TestCase):
    def test_sealed_word_and_mac_originals_are_required(self):
        rows = check.verify()
        self.assertEqual(len(rows), 2)
        by_name = {row[0]['mac_case']: row for row in rows}
        for row in rows:
            self.assertEqual(row[3]['word']['product_version'], '16.0.19127.20936')
        modern = by_name['probe-latin-join-controls-line-metrics']
        legacy = by_name['probe-latin-join-control-run-boundaries']
        self.assertEqual(modern[-1], collections.Counter({'\u200c': 8, '\u200d': 24}))
        self.assertEqual(len(modern[5]['glyphs']), 572)
        self.assertEqual(legacy[-1], collections.Counter({'\u200b': 5, '\u200c': 30, '\u200d': 30}))
        self.assertEqual(len(legacy[5]['glyphs']), 696)

    def test_removing_only_one_required_fixture_is_rejected(self):
        original = check.read_json
        def read(path):
            data = original(path)
            if Path(path).name == 'control-characters.json':
                data['fixtures'] = [case for case in data['fixtures']
                                    if case['mac_case'] != 'probe-latin-join-control-run-boundaries']
            return data
        with patch.object(check, 'read_json', side_effect=read):
            with self.assertRaisesRegex(ValueError, 'Required versioned control fixture removed'):
                check.verify()

    def test_deleting_fixture_is_rejected(self):
        with patch.object(check, 'read_json', return_value={'schema': 1, 'fixtures': []}):
            with self.assertRaisesRegex(ValueError, 'Missing control fixtures'):
                check.verify()

    def test_deleting_one_original_blank_slot_is_rejected(self):
        original = check.read_json
        def read(path):
            data = original(path)
            if Path(path).name == 'latin-join-control-blank-regions.json':
                data['regions'].pop()
            return data
        with patch.object(check, 'read_json', side_effect=read):
            with self.assertRaisesRegex(ValueError, 'Every original control-only page'):
                check.verify()

    def test_editable_projection_preserves_all_visible_origins_and_raw_source(self):
        model = dict(page_count=1, glyphs=[[1, 'A\u200c', 10., 20.],
                     [1, '\u200d', 11., 21.], [1, 'B', 12., 30.]])
        expected = collections.Counter({'\u200c': 1, '\u200d': 1})
        result = check.editable_origins(model, expected)
        self.assertEqual(result['glyphs'], [[1, 'A', 10., 20.], [1, 'B', 12., 30.]])
        self.assertEqual(model['glyphs'][0][1], 'A\u200c')
        for counts in [collections.Counter(), collections.Counter({'\u200c': 2, '\u200d': 1})]:
            with self.assertRaisesRegex(ValueError, 'lost, duplicated or changed'):
                check.editable_origins(model, counts)

    def test_standalone_zwsp_is_preserved_in_source_but_has_no_painted_origin(self):
        model = dict(page_count=1, glyphs=[[1, '\u200b', 10., 20.], [1, 'B', 10., 20.]])
        result = check.editable_origins(model, collections.Counter({'\u200b': 1}))
        self.assertEqual(result['glyphs'], [[1, 'B', 10., 20.]])
        self.assertEqual(model['glyphs'][0][1], '\u200b')

    def test_complex_and_combining_clusters_cannot_be_projected(self):
        for text in ['क\u200d', 'ا\u200d', 'A\u0301\u200d', '👩\u200d💻', 'fi\u200d']:
            with self.subTest(text=text):
                model = dict(page_count=1, glyphs=[[1, text, 10., 20.]])
                with self.assertRaisesRegex(ValueError, 'complex or combining'):
                    check.editable_origins(model, check.control_counts(text))

    def test_visible_loss_order_page_and_xy_errors_remain_failures(self):
        word = dict(page_count=1, glyphs=[[1, 'A', 10., 20.], [1, 'B', 12., 30.]])
        for glyphs in [[[1, 'A\u200d', 10., 20.]],
                       [[1, 'B', 12., 30.], [1, 'A\u200d', 10., 20.]],
                       [[2, 'A\u200d', 10., 20.], [1, 'B', 12., 30.]],
                       [[1, 'A\u200d', 11., 20.], [1, 'B', 12., 30.]]]:
            model = dict(page_count=1, glyphs=glyphs)
            projected = check.editable_origins(model, collections.Counter({'\u200d': 1}))
            with self.assertRaises(ValueError):
                check.compare(word, projected, 'control-regression')

    def test_faint_unsearchable_glyph_ink_is_rejected_without_control_metadata(self):
        import pymupdf
        with tempfile.TemporaryDirectory() as directory:
            original, native = [Path(directory)/name for name in ('word.pdf', 'native.pdf')]
            for path in [original, native]:
                with pymupdf.open() as doc:
                    doc.new_page(width=100, height=100)
                    doc.save(path)
            regions = [dict(page=1, rect=[10., 10., 20., 20.])]
            check.assert_blank_regions(original, native, regions)
            with pymupdf.open(native) as doc:
                # An outlined missing glyph can have no searchable text.
                # Even a99%-white rectangle must not pass as original blank ink.
                doc[0].draw_rect(pymupdf.Rect(12, 12, 18, 18), color=None,
                                fill=(.99, .99, .99))
                self.assertEqual(doc[0].get_texttrace(), [])
                data = doc.tobytes()
            native.write_bytes(data)
            with self.assertRaisesRegex(ValueError, 'native: extra ink'):
                check.assert_blank_regions(original, native, regions)


if __name__ == '__main__':
    unittest.main()
