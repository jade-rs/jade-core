"""Reject loss of source provenance or mixed-ascii drawing assertions."""
from copy import deepcopy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import check_windows_mixed_ascii as check
from check_platform_coordinates import ROOT, checked_file, read_json


class MixedASCII(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.index = read_json(ROOT / check.INDEX)
        cls.cases, cls.pending = check.verify()

    def test_actual_capture_source_and_required_coverage(self):
        self.assertEqual(len(self.cases), 16)
        self.assertEqual(len(self.pending), 1)
        self.assertEqual(sum(row['glyph_count'] for row, *_ in self.cases), 3072)
        self.assertEqual({row['input_specification']['pattern'] for row, *_ in self.cases}, {'AVATARWAVE', '%m@WAT'})
        for row, source, *_ in self.cases:
            data, spec = check.validated_source(ROOT, row)
            self.assertEqual(data, source.read_bytes())
            self.assertEqual(row['word'], self.index['word'])
            self.assertEqual(row['word']['product_version'], '16.0.19127.20936')

    def test_cannot_drop_or_swap_required_screen_xps_or_pdf_cases(self):
        for field in ('screen_gate_promotion', 'xps_gate_promotion', 'coordinate_gate_promotion'):
            changed = deepcopy(self.index)
            next(r for r in changed['fixtures'] if r[field] != 'pending')[field] = 'pending'
            with self.subTest(field=field), self.assertRaises(ValueError): check.locked_promotions(changed)
        changed = deepcopy(self.index)
        a = next(r for r in changed['fixtures'] if r['coordinate_gate_promotion'] == 'pending')
        b = next(r for r in changed['fixtures'] if r['coordinate_gate_promotion'] == 'mixed-ascii-print')
        a['coordinate_gate_promotion'], b['coordinate_gate_promotion'] = b['coordinate_gate_promotion'], a['coordinate_gate_promotion']
        with self.assertRaises(ValueError): check.locked_promotions(changed)
        for change in (lambda i: i['fixtures'].pop(),
                       lambda i: i['fixtures'][0].update(glyph_count=1)):
            changed = deepcopy(self.index); change(changed)
            with self.assertRaises(ValueError): check.locked_promotions(changed)

    def test_wrong_word_version_executable_mac_parent_or_source_is_rejected(self):
        original = check.read_json
        mutations = (lambda i: i['word'].update(product_version='16.0.0.0'),
                     lambda i: i['word'].update(executable_sha256='0' * 64),
                     lambda i: i['mac_parent_reference'].update(platform='windows'),
                     lambda i: i['fixtures'][0]['source'].update(sha256='0' * 64),
                     lambda i: i['fixtures'][0]['input_specification'].update(compatibility_mode=12),
                     lambda i: i['fixtures'][0]['word'].update(application_build='changed'),
                     lambda i: i['fixtures'][0]['input_specification'].update(font_size_pt=15.5),
                     lambda i: i['fixtures'][0]['input_specification'].update(font_style='bold'),
                     lambda i: i['fixtures'][0]['input_specification'].update(parent_name='missing'))
        for index, mutate in enumerate(mutations):
            changed = deepcopy(self.index); mutate(changed)
            with self.subTest(mutation=index), patch.object(check, 'read_json', side_effect=lambda path:
                    changed if path == ROOT / check.INDEX else original(path)), self.assertRaises(ValueError):
                check.verify()

    def test_changed_registry_is_rejected_before_capture_access(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); path = root / check.INDEX; path.parent.mkdir(parents=True)
            path.write_bytes(b'{"schema":1,"fixtures":[]}')
            with self.assertRaisesRegex(ValueError, 'registry changed'): check.verify(root)

    @staticmethod
    def coherent_models(row):
        # Synthetic coherent models test the rejection mechanism; actual native
        # comparisons run separately against immutable Word drawing files.
        model = dict(input_sha256=row['source']['sha256'], page_count=1,
                     glyphs=[[1, 'y', 10.0 + i * 5, 20.0] for i in range(row['glyph_count'])])
        return [deepcopy(model) for _ in range(8)]

    def test_every_case_rejects_screen_xps_drift_and_wrong_rendered_source(self):
        for row, *_ in self.cases:
            check.assert_models(row, *self.coherent_models(row))
            for plane in (4, 5):  # native and screen
                for axis in (2, 3):
                    models = self.coherent_models(row); models[plane]['glyphs'][0][axis] += 1
                    with self.subTest(name=row['name'], plane=plane, axis=axis), self.assertRaises(ValueError):
                        check.assert_models(row, *models)
                models = self.coherent_models(row); models[plane]['input_sha256'] = '0' * 64
                with self.subTest(name=row['name'], plane=plane), self.assertRaises(ValueError):
                    check.assert_models(row, *models)

    def test_all_pending_pdf_cases_still_reject_missing_text_pages_and_nonfinite_xy(self):
        for row, *_ in self.cases:
            for plane in (6, 7):  # physical visible and unclipped PDF
                for kind in ('missing', 'text', 'page', 'nan-x', 'inf-y'):
                    models = self.coherent_models(row); output = models[plane]
                    if kind == 'missing': output['glyphs'].pop()
                    elif kind == 'text': output['glyphs'][0][1] = 'x'
                    elif kind == 'page': output['page_count'] = 2
                    elif kind == 'nan-x': output['glyphs'][0][2] = float('nan')
                    else: output['glyphs'][0][3] = float('inf')
                    with self.subTest(name=row['name'], plane=plane, mutation=kind), self.assertRaises(ValueError):
                        check.assert_models(row, *models)

    def test_all_fifteen_required_pdf_cases_reject_xy_displacement(self):
        required = 0
        for row, *_ in self.cases:
            if row['name'] in self.pending: continue
            required += 1
            for plane in (6, 7):
                for axis in (2, 3):
                    models = self.coherent_models(row); models[plane]['glyphs'][0][axis] += 1
                    with self.subTest(name=row['name'], plane=plane, axis=axis), self.assertRaises(ValueError):
                        check.assert_models(row, *models)
        self.assertEqual(required, 15)


if __name__ == '__main__': unittest.main()
