"""Check reproducible source inputs, untouched Mac bytes and Word provenance."""
from io import BytesIO
from itertools import product
from zipfile import ZipFile
import unittest
from lxml import etree as E

import generate_windows_top_bottom_probes as producer
from check_platform_coordinates import ROOT, checked_file, read_json


class TopBottomProbes(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.metadata = read_json(ROOT / 'tests/word-oracle/windows/top-bottom-observations.json')
        cls.rows = cls.metadata['fixtures']

    def test_reproduces_all_captured_source_bytes_and_parameter_combinations(self):
        generated = {name: (data, facts) for name, data, facts in producer.generate()}
        self.assertEqual(len(generated), 24)
        self.assertEqual(set(generated), {r['name'] for r in self.rows})
        combinations = set()
        for row in self.rows:
            with self.subTest(case=row['name']):
                data, facts = generated[row['name']]
                self.assertEqual(data, checked_file(ROOT, row['source']).read_bytes())
                self.assertEqual(facts, row['input_specification'])
                combinations.add(tuple(facts[key] for key in (
                    'font_size_pt', 'vertical_offset_pt', 'distance_bottom_pt', 'compatibility_mode')))
        self.assertEqual(combinations, set(product((11, 14), (26.25, 52.5, 78.75), (0, 7.5), (14, 15))))
        with self.assertRaisesRegex(ValueError, 'Original Mac float source changed'):
            list(producer.generate(b'changed original'))

    def test_original_text_and_all_unrelated_package_parts_remain_unchanged(self):
        q = producer.q
        with ZipFile(ROOT / producer.PARENT) as parent:
            original = E.fromstring(parent.read('word/document.xml'))
            original_text = [node.text for node in original.iter(q('t'))]
            for name, data, facts in producer.generate():
                with self.subTest(case=name), ZipFile(BytesIO(data)) as child:
                    self.assertEqual(child.namelist(), parent.namelist())
                    for part in parent.namelist():
                        if part not in facts['changed_parts']:
                            self.assertEqual(child.read(part), parent.read(part), part)
                    document = E.fromstring(child.read('word/document.xml'))
                    self.assertEqual([node.text for node in document.iter(q('t'))], original_text)
                    self.assertEqual(facts['source_parent_sha256'], producer.PARENT_SHA256)

    def test_observations_retain_full_original_word_and_font_identity(self):
        mac = read_json(checked_file(ROOT, self.metadata['mac_parent_manifest']))
        self.assertEqual(mac['reference'], self.metadata['mac_parent_reference'])
        self.assertEqual(mac['reference']['platform'], 'mac')
        self.assertEqual(mac['sourceSha256'], producer.PARENT_SHA256)
        for row in self.rows:
            with self.subTest(case=row['name']):
                capture = read_json(checked_file(ROOT, row['capture']))
                self.assertEqual(capture['word'], row['word'])
                self.assertRegex(row['word']['product_version'], r'^\d+\.\d+\.\d+\.\d+$')
                self.assertRegex(row['word']['executable_sha256'], r'^[0-9a-f]{64}$')
                self.assertEqual(capture['capture_id'], row['capture_id'])
                self.assertTrue(row['original_word_font_programs'])
                for font in row['original_word_font_programs']:
                    self.assertRegex(font['program_sha256'], r'^[0-9a-f]{64}$')
                    self.assertTrue(font['family'])
                    self.assertTrue(font['version'])
                self.assertEqual(row['coordinate_gate_promotion'], 'required')


if __name__ == '__main__':
    unittest.main()
