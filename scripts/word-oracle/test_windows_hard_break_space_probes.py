"""Keep original Word controls and fail closed on missing or displaced assertions."""
from copy import deepcopy
from io import BytesIO
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from zipfile import ZipFile

from lxml import etree as E

import check_windows_hard_break_spaces as check
from check_platform_coordinates import ROOT, read_json
import generate_windows_hard_break_space_probes as producer


class HardBreakSpaces(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.index = read_json(ROOT / check.INDEX)
        cls.verified = check.verify()
        cls.generated = {n: (data, spec) for n, data, spec in producer.generate()}

    def test_all_originals_reproduce_with_required_coordinates(self):
        cases, fonts, xps, models = self.verified
        self.assertEqual(len(cases), 32)
        self.assertEqual(len(fonts), 32)
        self.assertEqual(len(xps), 32)
        self.assertEqual(set(models), set(self.generated))
        self.assertEqual(sum(s['unchanged_parent_bytes'] for _, s in self.generated.values()), 2)
        for row in cases:
            data, spec = self.generated[row['name']]
            self.assertEqual((ROOT / row['source']['file']).read_bytes(), data)
            self.assertEqual(row['input_specification'], spec)
            expected_font = True
            expected_xps = True
            self.assertEqual(row['name'] in fonts, expected_font)
            self.assertEqual(row['name'] in xps, expected_xps)

    def test_only_selected_text_endings_and_compatibility_change(self):
        q = producer.q
        for name, (data, spec) in self.generated.items():
            parent = spec['derived_parent']
            if 'archive' in parent:
                with ZipFile(ROOT / parent['archive']['file']) as archive:
                    original = archive.read('inputs/' + parent['name'] + '.docx')
            else:
                original = self.generated[parent['name']][0]
            with self.subTest(name=name), ZipFile(BytesIO(original)) as old, ZipFile(BytesIO(data)) as new:
                self.assertEqual(old.namelist(), new.namelist())
                for member in old.namelist():
                    if member not in spec['changed_parts']:
                        self.assertEqual(old.read(member), new.read(member))
                before, after = [E.fromstring(z.read('word/document.xml')) for z in (old, new)]
                old_run, = before.iter(q('r')); new_run, = after.iter(q('r'))
                first = next(new_run.iter(q('t')))
                self.assertEqual(first.text, spec['text'])
                first.text = next(old_run.iter(q('t'))).text
                if spec['character_set'] == 'mixed' and spec['line_ending'] == 'paragraph':
                    for node in list(old_run):
                        if node.tag in (q('br'), q('t')) and node is not next(old_run.iter(q('t'))):
                            old_run.remove(node)
                self.assertEqual(E.tostring(before), E.tostring(after))
                before, after = [E.fromstring(z.read('word/settings.xml')) for z in (old, new)]
                for tree in (before, after):
                    for node in tree.iter(q('compatSetting')):
                        if node.get(q('name')) == 'compatibilityMode':
                            node.set(q('val'), '15')
                self.assertEqual(E.tostring(before), E.tostring(after))

    def test_required_promotions_cannot_be_removed_renamed_or_masked(self):
        for field, promotion in [('font_gate_promotion', 'hard-break-font'), ('xps_gate_promotion', 'hard-break-xps')]:
            for operation in ('remove', 'rename', 'unknown'):
                index = deepcopy(self.index)
                required = next(r for r in index['fixtures'] if r[field] == promotion)
                if operation == 'remove':
                    required[field] = 'pending'
                elif operation == 'rename':
                    required['name'] += '-unexpected'
                else:
                    required[field] = 'unknown'
                with self.subTest(field=field, operation=operation), self.assertRaises(ValueError):
                    check.locked_promotions(index)
        index = deepcopy(self.index)
        index['fixtures'].append(deepcopy(index['fixtures'][0]))
        with self.assertRaises(ValueError):
            check.locked_promotions(index)

    def test_wrong_word_identity_or_source_is_rejected(self):
        original = check.read_json
        for edit in (lambda i: i['word'].update(product_version='16.0.0.0'),
                     lambda i: i['word'].update(executable_sha256='0' * 64),
                     lambda i: i['fixtures'][0]['source'].update(sha256='0' * 64)):
            index = deepcopy(self.index)
            edit(index)
            with patch.object(check, 'read_json', side_effect=lambda p: index if p == ROOT / check.INDEX else original(p)), self.assertRaises(ValueError):
                check.verify()

    def rendered(self, path, mutation=None):
        cases, _, required, models = self.verified
        for row in cases:
            if row['name'] not in required:
                continue
            folder = path / row['capture_id'] / row['name']
            folder.mkdir(parents=True)
            for plane in ('native', 'visible'):
                model = deepcopy(models[row['name']])
                model['input_sha256'] = row['source']['sha256']
                if mutation:
                    mutation(row['name'], plane, model)
                (folder / (plane + '.json')).write_bytes(json.dumps(model).encode())

    def test_every_required_case_reaches_both_xps_assertions(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(check, 'verify', return_value=self.verified):
            directory = Path(temporary)
            self.rendered(directory)
            results = check.check_rendered_fonts(directory)
            self.assertEqual(len(results), 32)
            self.assertTrue(all(set(r['planes']) == {'native', 'visible'} for r in results))

    def test_each_case_rejects_x_y_drift_and_wrong_input_on_both_paths(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(check, 'verify', return_value=self.verified):
            directory = Path(temporary)
            self.rendered(directory)
            for row in self.verified[0]:
                if row['name'] not in self.verified[2]:
                    continue
                for plane in ('native', 'visible'):
                    path = directory / row['capture_id'] / row['name'] / (plane + '.json')
                    original = path.read_bytes()
                    for axis in (2, 3, 'input'):
                        model = json.loads(original)
                        if axis == 'input':
                            model['input_sha256'] = '0' * 64
                        else:
                            model['glyphs'][0][axis] += .5
                        path.write_bytes(json.dumps(model).encode())
                        with self.subTest(name=row['name'], plane=plane, axis=axis), self.assertRaises(ValueError):
                            check.check_rendered_fonts(directory)
                        path.write_bytes(original)


if __name__ == '__main__':
    unittest.main()
