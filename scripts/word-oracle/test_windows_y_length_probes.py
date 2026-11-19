"""Reject loss of original Word provenance or repeated-y coordinate assertions."""
from copy import deepcopy
from io import BytesIO
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from zipfile import ZipFile

from lxml import etree as E

import check_windows_y_lengths as check
from check_platform_coordinates import ROOT, checked_file, read_json
import generate_windows_y_length_probes as producer


class YLengths(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.index = read_json(ROOT / check.INDEX)
        cls.verified = check.verify()

    def test_all_four_original_inputs_reproduce_and_cover_56_paragraphs(self):
        generated = {n: (data, spec) for n, data, spec in producer.generate()}
        cases, pending, xps = self.verified
        self.assertEqual(len(cases), 4)
        self.assertEqual(len(pending), 2)
        self.assertEqual(set(xps), set(generated))
        self.assertEqual(sum(len(s['paragraphs']) for _, s in generated.values()), 56)
        self.assertEqual(sum(row['glyph_count'] for row, *_ in cases), 1092)
        for row, source, *_ in cases:
            data, spec = generated[row['name']]
            self.assertEqual(source.read_bytes(), data)
            self.assertEqual(row['input_specification'], spec)
            expected = producer.LENGTHS if spec['layout'] == 'lengths' else (48,)
            for kerning in ('off', 'on'):
                self.assertEqual(tuple(p['length'] for p in spec['paragraphs']
                                       if p['kerning'] == kerning), expected)

    def test_only_paragraph_selection_and_y_lengths_change(self):
        q = producer.q
        for _, data, spec in producer.generate():
            with self.subTest(style=spec['style'], layout=spec['layout']), \
                    ZipFile(checked_file(ROOT, spec['derived_parent'])) as old, \
                    ZipFile(BytesIO(data)) as new:
                self.assertEqual(old.namelist(), new.namelist())
                for name in old.namelist():
                    if name != 'word/document.xml':
                        self.assertEqual(old.read(name), new.read(name))
                before, after = [E.fromstring(z.read('word/document.xml')) for z in (old, new)]
                ob, = before.iter(q('body')); nb, = after.iter(q('body'))
                originals = ob.findall(q('p')); paragraphs = nb.findall(q('p'))
                self.assertEqual(len(paragraphs), len(spec['paragraphs']))
                for paragraph, item in zip(paragraphs, spec['paragraphs']):
                    template = originals[2 if item['kerning'] == 'off' else 14]
                    text, = paragraph.iter(q('t'))
                    self.assertEqual(text.text, 'y' * item['length'])
                    text.text = 'y' * 48
                    self.assertEqual(E.tostring(paragraph), E.tostring(template))
                for body in (ob, nb):
                    for paragraph in body.findall(q('p')):
                        body.remove(paragraph)
                self.assertEqual(E.tostring(before), E.tostring(after))

    def test_promoted_pdf_and_all_screen_xps_cases_cannot_be_removed_or_swapped(self):
        for field, promotion in [('coordinate_gate_promotion', 'y-length-print'),
                                 ('screen_gate_promotion', 'y-length-screen'),
                                 ('xps_gate_promotion', 'y-length-xps')]:
            index = deepcopy(self.index)
            next(r for r in index['fixtures'] if r[field] == promotion)[field] = 'pending'
            with self.subTest(field=field), self.assertRaises(ValueError):
                check.locked_promotions(index)
        index = deepcopy(self.index)
        required = next(r for r in index['fixtures'] if r['coordinate_gate_promotion'] == 'y-length-print')
        pending = next(r for r in index['fixtures'] if r['coordinate_gate_promotion'] == 'pending')
        required['coordinate_gate_promotion'], pending['coordinate_gate_promotion'] = \
            pending['coordinate_gate_promotion'], required['coordinate_gate_promotion']
        with self.assertRaises(ValueError):
            check.locked_promotions(index)

    def test_wrong_word_version_executable_or_mac_reference_is_rejected(self):
        original = check.read_json
        for edit in (lambda i: i['word'].update(product_version='16.0.0.0'),
                     lambda i: i['word'].update(executable_sha256='0' * 64),
                     lambda i: i['mac_parent_reference'].update(platform='windows')):
            index = deepcopy(self.index)
            edit(index)
            with patch.object(check, 'read_json', side_effect=lambda p:
                              index if p == ROOT / check.INDEX else original(p)), \
                    self.assertRaises(ValueError):
                check.verify()

    def run_xps_path(self, directory, mutation=None):
        cases, pending, models = self.verified
        def rendered(executable, output, actual_cases, actual_pending):
            self.assertEqual([c[0]['name'] for c in actual_cases], [c[0]['name'] for c in cases])
            self.assertEqual(actual_pending, pending)
            for case, *_ in actual_cases:
                for plane in ('native', 'screen'):
                    model = deepcopy(models[case['name']])
                    if mutation:
                        mutation(case['name'], plane, model)
                    folder = output / case['name']
                    folder.mkdir(exist_ok=True)
                    (folder / (plane + '.json')).write_text(json.dumps(model), encoding='utf-8')
        with patch.object(check, 'verify', return_value=self.verified), \
                patch.object(check, 'render_cases', side_effect=rendered):
            return check.check(Path('renderer'), directory)

    def test_every_case_reaches_both_required_xps_coordinate_assertions(self):
        with tempfile.TemporaryDirectory() as temporary:
            rows = self.run_xps_path(Path(temporary))
        self.assertEqual(len(rows), 4)
        self.assertEqual(sum(r['required_pdf'] for r in rows), 2)
        self.assertTrue(all(r['required_xps'] for r in rows))

    def test_each_case_rejects_x_and_y_displacement_in_both_render_paths(self):
        for case, *_ in self.verified[0]:
            for plane in ('native', 'screen'):
                for axis in (2, 3):
                    def displaced(name, current_plane, model):
                        if name == case['name'] and current_plane == plane:
                            model['glyphs'][0][axis] += .5
                    with self.subTest(name=case['name'], plane=plane, axis=axis), \
                            tempfile.TemporaryDirectory() as temporary, self.assertRaises(ValueError):
                        self.run_xps_path(Path(temporary), displaced)


if __name__ == '__main__':
    unittest.main()
