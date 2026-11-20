"""Reject loss of source provenance or device-remainder drawing assertions."""
from copy import deepcopy
from io import BytesIO
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from zipfile import ZipFile

from lxml import etree as E

import check_windows_device_remainders as check
from check_platform_coordinates import ROOT, checked_file, read_json
import generate_windows_device_remainder_probes as producer


class DeviceRemainders(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.index = read_json(ROOT / check.INDEX)
        cls.cases, cls.pending = check.verify()

    def test_all_sources_reproduce_with_full_word_and_font_identity(self):
        generated = {name: (data, spec) for name, data, spec in producer.generate()}
        self.assertEqual(len(self.cases), 36)
        self.assertEqual(len(self.pending), 5)
        self.assertEqual(sum(row['glyph_count'] for row, *_ in self.cases), 2724)
        self.assertEqual(sum(spec['paragraphs'] for _, spec in generated.values()), 108)
        self.assertEqual({r['input_specification']['compatibility_mode'] for r, *_ in self.cases}, {14})
        for row, source, *_ in self.cases:
            with self.subTest(name=row['name']):
                data, spec = generated[row['name']]
                self.assertEqual((source.read_bytes(), row['input_specification']), (data, spec))
                self.assertEqual(row['word'], self.index['word'])
                self.assertEqual(row['word']['product_version'], '16.0.19127.20936')
                self.assertIn('word-16.0.19127.20936', row['source']['file'])

    def test_only_source_text_lengths_change(self):
        q = producer.q
        self.assertEqual(len(producer.LENGTHS), 18)
        for name, data, spec in producer.generate():
            with self.subTest(name=name), ZipFile(checked_file(ROOT, spec['derived_parent'])) as before, \
                    ZipFile(BytesIO(data)) as after:
                self.assertEqual(before.namelist(), after.namelist())
                for member in before.namelist():
                    if member != 'word/document.xml':
                        self.assertEqual(before.read(member), after.read(member))
                old, new = [E.fromstring(z.read('word/document.xml')) for z in (before, after)]
                old_text, new_text = [list(tree.iter(q('t'))) for tree in (old, new)]
                self.assertEqual(len(old_text), spec['paragraphs'])
                self.assertEqual(len(new_text), spec['paragraphs'])
                for original, changed in zip(old_text, new_text):
                    self.assertEqual(original.text, spec['character'] * 48)
                    self.assertEqual(changed.text, spec['character'] * spec['length'])
                    changed.text = original.text
                self.assertEqual(E.tostring(old), E.tostring(new))

    def test_cannot_drop_or_swap_required_screen_xps_or_pdf_cases(self):
        for field in ('screen_gate_promotion', 'xps_gate_promotion', 'coordinate_gate_promotion'):
            changed = deepcopy(self.index)
            next(r for r in changed['fixtures'] if r[field] != 'pending')[field] = 'pending'
            with self.subTest(field=field), self.assertRaises(ValueError): check.locked_promotions(changed)
        changed = deepcopy(self.index)
        a = next(r for r in changed['fixtures'] if r['coordinate_gate_promotion'] == 'pending')
        b = next(r for r in changed['fixtures'] if r['coordinate_gate_promotion'] == 'device-remainder-print')
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
                     lambda i: i['fixtures'][0]['word'].update(application_build='changed'))
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

    def test_all_thirty_one_required_pdf_cases_reject_xy_displacement(self):
        required = 0
        for row, *_ in self.cases:
            if row['name'] in self.pending: continue
            required += 1
            for plane in (6, 7):
                for axis in (2, 3):
                    models = self.coherent_models(row); models[plane]['glyphs'][0][axis] += 1
                    with self.subTest(name=row['name'], plane=plane, axis=axis), self.assertRaises(ValueError):
                        check.assert_models(row, *models)
        self.assertEqual(required, 31)


if __name__ == '__main__': unittest.main()
