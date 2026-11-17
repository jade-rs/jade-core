"""Verify independently captured Word subset identities and reproducible inputs."""
from collections import Counter
from io import BytesIO
import hashlib
import unittest
from zipfile import ZipFile

import pymupdf
import generate_windows_aptos_subset_probes as generate
from check_platform_coordinates import checked_file, compare, pdf_origins, read_json
from extract_windows_emf import extract


class WindowsAptosSubsetProbes(unittest.TestCase):
    def observations(self):
        path = generate.ROOT / 'tests/word-oracle/windows/aptos-subset-observations.json'
        self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), 'fc16f6d34a238642fb7affd88d52c39ab79ec382725f931ab3847ee7a43105f8')
        return read_json(path)

    def test_eight_captured_inputs_reproduce_with_only_document_xml_changed(self):
        cases = {c['name']: c for c in self.observations()['fixtures'] if 'parent_source' in c}
        generated = dict(generate.probes())
        self.assertEqual(len(cases), 8)
        self.assertEqual(set(cases), set(generated))
        self.assertEqual({c['size_pt'] for c in cases.values()}, {14, 40})
        with ZipFile(generate.ROOT / generate.PARENT) as parent:
            for name, raw in generated.items():
                with self.subTest(case=name), ZipFile(BytesIO(raw)) as actual:
                    case = cases[name]
                    self.assertEqual(raw, checked_file(generate.ROOT, case['source']).read_bytes())
                    self.assertEqual(case['parent_source'], generate.PARENT.as_posix())
                    self.assertEqual(case['parent_sha256'], hashlib.sha256(
                        (generate.ROOT / generate.PARENT).read_bytes()).hexdigest())
                    self.assertEqual(actual.namelist(), parent.namelist())
                    self.assertEqual([part for part in parent.namelist()
                                      if actual.read(part) != parent.read(part)], ['word/document.xml'])

    def test_nine_promoted_cases_retain_original_version_fonts_and_origins(self):
        cases = self.observations()['fixtures']
        self.assertEqual(len(cases), 9)
        for case in cases:
            with self.subTest(case=case['name']):
                archive = checked_file(generate.ROOT, case['archive'])
                manifest = checked_file(generate.ROOT, case['capture'])
                capture = read_json(manifest)
                self.assertEqual(capture['word'], case['word'])
                self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
                self.assertEqual(capture['word']['executable_sha256'],
                                 '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                document, = capture['documents']
                self.assertEqual(document['open_mode'], 'normal-read-only')
                self.assertEqual(document['compatibility_mode'], 14)
                with ZipFile(archive) as original:
                    self.assertEqual(original.read('capture.json'), manifest.read_bytes())
                    self.assertEqual(original.read('inputs/' + case['name'] + '.docx'),
                                     checked_file(generate.ROOT, case['source']).read_bytes())
                    self.assertEqual(original.read(document['pdf']),
                                     (manifest.parent / document['pdf']).read_bytes())
                reference = read_json(checked_file(generate.ROOT, case['drawing_reference']))
                self.assertEqual(reference, extract(manifest))
                drawing, = reference['fixtures']
                self.assertEqual(compare(pdf_origins(manifest.parent / document['pdf']), drawing,
                                         case['name'] + '/Word originals'), len(drawing['glyphs']))
                with pymupdf.open(manifest.parent / document['pdf']) as pdf:
                    for program in case['original_word_font_programs']:
                        name, extension, _, data = pdf.extract_font(program['xref'])
                        self.assertEqual((name, extension), (program['postscript_name'], program['extension']))
                        self.assertEqual(hashlib.sha256(data).hexdigest(), program['sha256'])

    def test_failures_retain_multiple_subsets_with_distinct_legacy_families(self):
        failures = [c for c in self.observations()['fixtures'] if not c['baseline_all_planes_pass']]
        self.assertEqual(len(failures), 8)
        for case in failures:
            with self.subTest(case=case['name']):
                programs = case['original_word_font_programs']
                counts = Counter(family for program in programs for family in program['legacy_family'])
                self.assertGreaterEqual(counts['Aptos'], 2)
                self.assertGreaterEqual(counts['Aptos Display'], 2)
                self.assertTrue(all('Aptos' in p['typographic_family'] for p in programs
                                    if 'Aptos Display' in p['legacy_family']))

    def test_same_word_build_font_change_preserves_historical_captures(self):
        controls = self.observations()['historical_controls']
        self.assertEqual(len(controls), 4)
        prior_ids = {c['capture_id'] for c in read_json(
            generate.ROOT / 'tests/word-oracle/windows/observations/index.json')['captures']}
        for case in controls:
            with self.subTest(case=case['name']):
                checked_file(generate.ROOT, case['archive'])
                self.assertIn(case['previous_capture_id'], prior_ids)
                self.assertEqual(case['word'], case['previous_word'])
                self.assertEqual(hashlib.sha256((generate.ROOT / case['original_mac_source']).read_bytes())
                                 .hexdigest(), case['source_sha256'])
                if case['promoted']:
                    self.assertEqual(case['name'], 'aptos-ascii-advances-mac')
                    self.assertNotEqual(case['old_new_word_coordinates'], 'PASS')
                    self.assertTrue(any('Calibri' in p['family'] for p in case['previous_fonts']))
                    self.assertFalse(any('Calibri' in p['family'] for p in case['current_fonts']))
                else:
                    self.assertEqual(case['old_new_word_coordinates'], 'PASS')


if __name__ == '__main__':
    unittest.main()
