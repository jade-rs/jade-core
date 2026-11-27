"""Reproduce all inputs and verify Word's independent style/family evidence."""
from io import BytesIO
import hashlib
import json
import unittest
from zipfile import ZipFile

import pymupdf
import generate_windows_aptos_style_probes as generate
from check_platform_coordinates import checked_file, compare, pdf_origins, read_json
from extract_windows_emf import extract


class WindowsAptosStyleProbes(unittest.TestCase):
    def cases(self):
        path = generate.ROOT / 'tests/word-oracle/windows/aptos-style-observations.json'
        self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(),
                         'a32c25eeab42c8b73d5c3f0f2463fa17bda740cb05555549ca2bf8d8d86b8515')
        return read_json(path)['fixtures']

    def test_all_48_original_inputs_reproduce_without_changing_other_parts(self):
        cases = {c['name']: c for c in self.cases()}
        generated = dict(generate.probes())
        self.assertEqual(len(cases), 48)
        self.assertEqual(set(cases), set(generated))
        self.assertEqual({c['font_size_pt'] for c in cases.values()}, {12, 20, 40})
        with ZipFile(generate.ROOT / generate.PARENT) as parent:
            for name, raw in generated.items():
                with self.subTest(case=name), ZipFile(BytesIO(raw)) as actual:
                    case = cases[name]
                    self.assertEqual(raw, checked_file(generate.ROOT, case['source']).read_bytes())
                    self.assertEqual(case['parent_source'], generate.PARENT.as_posix())
                    self.assertEqual(case['parent_sha256'], hashlib.sha256(
                        (generate.ROOT / generate.PARENT).read_bytes()).hexdigest())
                    self.assertEqual(actual.namelist(), parent.namelist())
                    for part in parent.namelist():
                        if part != 'word/document.xml':
                            self.assertEqual(actual.read(part), parent.read(part))

    def test_word_version_fonts_and_both_original_coordinate_planes_are_immutable(self):
        for case in self.cases():
            with self.subTest(case=case['name']):
                archive = checked_file(generate.ROOT, case['archive'])
                manifest = checked_file(generate.ROOT, case['capture'])
                capture = read_json(manifest)
                with ZipFile(archive) as original:
                    self.assertEqual(original.read('capture.json'), manifest.read_bytes())
                    self.assertEqual(capture['word'], case['word'])
                    self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
                    self.assertEqual(capture['word']['executable_sha256'],
                                     '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                    self.assertEqual(capture['capture_id'], case['capture_id'])
                    document, = capture['documents']
                    self.assertEqual(document['compatibility_mode'], 14)
                    self.assertEqual(document['open_mode'], 'normal-read-only')
                    self.assertEqual(original.read('inputs/' + case['name'] + '.docx'),
                                     checked_file(generate.ROOT, case['source']).read_bytes())
                    self.assertEqual(original.read(document['pdf']),
                                     (manifest.parent / document['pdf']).read_bytes())
                reference = read_json(checked_file(generate.ROOT, case['drawing_reference']))
                self.assertEqual(reference, extract(manifest))
                drawing, = reference['fixtures']
                self.assertEqual(len(drawing['glyphs']), 7 * case['paragraph_count'])
                self.assertEqual(compare(pdf_origins(manifest.parent / document['pdf']), drawing,
                                         case['name'] + '/original Word planes'), len(drawing['glyphs']))
                with pymupdf.open(manifest.parent / document['pdf']) as pdf:
                    for program in case['original_word_font_programs']:
                        name, extension, _, data = pdf.extract_font(program['xref'])
                        self.assertEqual(extension, program['extension'])
                        self.assertEqual(name, program['postscript_name'])
                        self.assertEqual(hashlib.sha256(data).hexdigest(), program['sha256'])

    def test_six_original_failures_keep_distinct_legacy_and_typographic_families(self):
        failures = [c for c in self.cases() if not c['baseline_all_planes_pass']]
        self.assertEqual(len(failures), 6)
        for case in failures:
            with self.subTest(case=case['name']):
                self.assertEqual(case['paragraph_count'], 3)
                self.assertEqual(case['font_family'], 'Aptos Display')
                self.assertTrue(case['bold'])
                display = [p for p in case['original_word_font_programs']
                           if 'Aptos Display' in p['legacy_family']]
                self.assertEqual({p['weight'] for p in display}, {400, 700})
                self.assertTrue(all('Aptos' in p['typographic_family'] for p in display))


if __name__ == '__main__':
    unittest.main()
