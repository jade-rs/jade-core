"""Protect source conservation, visible order and complete clipped-font geometry."""
import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import check_zwnj_table_coordinates as check


class ZwnjTableCoordinates(unittest.TestCase):
    def test_both_genuine_platform_originals_and_word_version_are_required(self):
        case, source, archive, capture, document, emf, mac, model = check.verify()
        self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
        self.assertEqual(document['pages'], 8)
        self.assertEqual(len(emf['glyphs']), 358)
        self.assertEqual(len(model['glyphs']), 358)
        self.assertEqual(mac['sourceSha256'], check.SOURCE_SHA256)
        self.assertEqual(sum(check.source_control_counts(source).values()), 8)

    def test_replacing_sealed_fixture_bytes_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / check.REFERENCE
            path.parent.mkdir(parents=True)
            path.write_bytes((check.ROOT / check.REFERENCE).read_bytes() + b' ')
            with self.assertRaisesRegex(ValueError, 'Evidence changed'):
                check.verify(root)

    def test_coverage_cannot_drop_hidden_glyphs_or_source_controls(self):
        original = check.read_json
        for field in ['unclipped_glyphs', 'clipped_glyphs', 'controls', 'mac_glyphs']:
            def read(path):
                data = original(path)
                if Path(path).name == 'zwnj-table-line-metrics.json':
                    data['expected'][field] = 0
                return data
            with self.subTest(field=field), patch.object(check, 'read_json', side_effect=read):
                with self.assertRaisesRegex(ValueError, 'coverage policy changed'):
                    check.verify()

    def test_removing_one_original_blank_page_is_rejected(self):
        original = check.read_json
        def read(path):
            data = original(path)
            if Path(path).name == 'zwnj-table-line-metrics.json':
                data['blank_regions'].pop()
            return data
        with patch.object(check, 'read_json', side_effect=read):
            with self.assertRaisesRegex(ValueError, 'Every original blank control line'):
                check.verify()

    def test_mac_gate_requires_an_actual_mac_host_when_requested(self):
        with patch.object(check.sys, 'platform', 'win32'):
            with self.assertRaisesRegex(ValueError, 'Actual macOS host required'):
                check.check_native(Path('unused'), Path('unused'), 'mac', require_macos=True)
        with patch.object(check.sys, 'platform', 'darwin'):
            with self.assertRaisesRegex(ValueError, 'Actual Windows host required'):
                check.check_native(Path('unused'), Path('unused'), 'windows')

    @staticmethod
    def planes():
        glyphs = [[1, text, x, y, 'Verdana', 32.04]
                  for text, x, y in [('A', 10., 20.), ('B', 20., 20.),
                                     ('C', 10., 40.), ('D', 20., 40.)]]
        model = lambda rows: dict(page_count=1, glyphs=copy.deepcopy(rows))
        visible = [glyphs[0], glyphs[2]]
        return [model([g[:4] for g in glyphs]), model(visible), model(glyphs),
                model([g[:4] for g in visible]), model([g[:4] for g in glyphs]),
                model(visible), model([glyphs[0], glyphs[2], glyphs[1], glyphs[3]])]

    def test_hidden_metadata_can_follow_content_while_source_and_visible_order_remain_exact(self):
        check.assert_windows_planes(*self.planes())

    def test_order_loss_duplicates_axis_and_font_changes_remain_failures(self):
        for kind in ['source-order', 'visible-order', 'missing-hidden', 'duplicate-hidden',
                     'hidden-axis', 'hidden-font', 'hidden-em', 'visible-source-axis']:
            models = self.planes()
            if kind == 'source-order':
                models[4]['glyphs'].reverse()
            elif kind == 'visible-order':
                models[5]['glyphs'].reverse()
            elif kind == 'missing-hidden':
                models[6]['glyphs'].pop()
            elif kind == 'duplicate-hidden':
                models[6]['glyphs'].append(models[6]['glyphs'][2][:])
            elif kind == 'hidden-axis':
                models[6]['glyphs'][2][2] += 1.
            elif kind == 'hidden-font':
                models[6]['glyphs'][2][4] = 'ArialMT'
            elif kind == 'hidden-em':
                models[6]['glyphs'][2][5] = 31.
            else:
                models[3]['glyphs'][0][3] += 1.
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                check.assert_windows_planes(*models)

    def test_mac_uses_original_tolerance_without_allowing_order_or_coordinate_changes(self):
        original = dict(page_count=1, glyphs=[[1, 'A', 10., 20.], [1, 'B', 20., 20.]])
        check.assert_mac_original(original, copy.deepcopy(original), .65)
        for kind in ['order', 'axis', 'page', 'lost']:
            changed = copy.deepcopy(original)
            if kind == 'order':
                changed['glyphs'].reverse()
            elif kind == 'axis':
                changed['glyphs'][0][2] += .66
            elif kind == 'page':
                changed['glyphs'][0][0] = 2
            else:
                changed['glyphs'].pop()
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                check.assert_mac_original(original, changed, .65)


if __name__ == '__main__':
    unittest.main()
