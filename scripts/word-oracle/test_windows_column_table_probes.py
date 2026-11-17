"""Assert original inputs, font/Word provenance and independent output contexts."""
import hashlib
from io import BytesIO
import itertools
import json
from pathlib import Path
import unittest
from zipfile import ZipFile

from lxml import etree as ET

from check_platform_coordinates import checked_file, compare, pdf_origins
from extract_windows_emf import extract
import generate_windows_column_table_probes as generator

ROOT = Path(__file__).resolve().parents[2]
INDEX = ROOT / 'tests/word-oracle/windows/column-table-observations.json'
read = lambda p: json.loads(p.read_text(encoding='utf-8-sig'))
sha = lambda data: hashlib.sha256(data).hexdigest()


class ColumnTableProbes(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.metadata = read(INDEX)
        cls.rows = cls.metadata['fixtures']
        cls.parent = (ROOT / generator.PARENT).read_bytes()
        assert sha(cls.parent) == generator.PARENT_SHA256

    def test_all_54_inputs_reproduce_exact_original_capture_bytes(self):
        generated = {name: (data, facts) for name, data, facts in generator.generate(self.parent)}
        self.assertEqual(len(generated), 54)
        self.assertEqual(set(generated), {row['name'] for row in self.rows})
        combinations = set()
        for row in self.rows:
            with self.subTest(case=row['name']):
                source = checked_file(ROOT, row['source']).read_bytes()
                data, facts = generated[row['name']]
                self.assertEqual(data, source)
                self.assertEqual(row['input_specification'], dict(name=row['name'], source_sha256=sha(data), **facts))
                combinations.add((facts['frame'], facts['preferred_width_pt'], facts['alignment'], facts['compatibility_mode']))
        self.assertEqual(combinations, set(itertools.product(
            ('single', 'equal', 'unequal'), (80, 100, 120), ('left', 'center', 'right'), (14, 15))))
        with self.assertRaisesRegex(ValueError, 'Original Mac table source changed'):
            list(generator.generate(self.parent + b'changed'))

    def test_mac_parts_text_and_existing_cell_geometry_are_preserved(self):
        parent_manifest = read(checked_file(ROOT, self.metadata['mac_parent_manifest']))
        self.assertEqual(parent_manifest['reference'], self.metadata['mac_parent_reference'])
        self.assertEqual(parent_manifest['reference']['platform'], 'mac')
        self.assertNotIn('version', parent_manifest['reference'])
        with ZipFile(BytesIO(self.parent)) as original:
            original_document = ET.fromstring(original.read('word/document.xml'))
            original_rows = [ET.tostring(row) for row in original_document.iter(generator.q('tr'))]
            original_text = [t.text for t in original_document.iter(generator.q('t'))]
            for row in self.rows:
                with self.subTest(case=row['name']), ZipFile(checked_file(ROOT, row['source'])) as package:
                    self.assertEqual(package.namelist(), original.namelist())
                    for part in original.namelist():
                        if part not in ('word/document.xml', 'word/settings.xml'):
                            self.assertEqual(package.read(part), original.read(part), part)
                    document = ET.fromstring(package.read('word/document.xml'))
                    self.assertEqual([t.text for t in document.iter(generator.q('t'))], original_text)
                    self.assertEqual([ET.tostring(r) for r in document.iter(generator.q('tr'))], original_rows)

    def test_actual_word_version_executable_and_original_font_versions_are_retained(self):
        for row in self.rows:
            with self.subTest(case=row['name']):
                capture = read(checked_file(ROOT, row['capture']))
                document, = capture['documents']
                self.assertEqual(capture['word'], row['word'])
                self.assertEqual(row['word']['product_version'], '16.0.19127.20936')
                self.assertEqual(row['word']['executable_sha256'], '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                self.assertEqual(document['open_mode'], 'normal-read-only')
                self.assertEqual(document['compatibility_mode'], row['input_specification']['compatibility_mode'])
                self.assertEqual(document['sha256'], row['source']['sha256'])
                programs = row['original_word_font_programs']
                arial, = [p for p in programs if 'Arial' in p['family']]
                yu, = [p for p in programs if 'Yu Mincho' in p['family']]
                self.assertEqual(arial['version'], ['Version 7.07'])
                self.assertEqual(yu['version'], ['Version 1.92'])
                self.assertEqual(arial['upem'], 2048)
                self.assertEqual(yu['upem'], 2048)

    def test_six_original_word_screen_pdf_differences_remain_independent(self):
        differences = 0
        for row in self.rows:
            manifest = checked_file(ROOT, row['capture'])
            document, = read(manifest)['documents']
            emf, = extract(manifest)['fixtures']
            pdf = pdf_origins(manifest.parent / document['pdf'], True)
            facts = row['input_specification']
            distinct = facts['frame'] == 'unequal' and facts['alignment'] in ('center', 'right') and facts['compatibility_mode'] == 14
            with self.subTest(case=row['name']):
                if distinct:
                    with self.assertRaisesRegex(ValueError, 'axis mismatch'):
                        compare(pdf, emf, row['name'])
                    differences += 1
                else:
                    self.assertGreater(compare(pdf, emf, row['name']), 0)
        self.assertEqual(differences, 6)


if __name__ == '__main__':
    unittest.main()
