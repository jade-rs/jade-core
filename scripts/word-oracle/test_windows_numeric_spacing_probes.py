"""Preserve original Word numeric-spacing evidence and reproducible inputs."""
from collections import Counter
from io import BytesIO
import hashlib
import unittest
from zipfile import ZipFile

import pymupdf
import generate_windows_numeric_spacing_probes as generate
from check_platform_coordinates import checked_file, read_json
from extract_windows_emf import extract

INDEX = 'tests/word-oracle/windows/numeric-spacing-observations.json'
INDEX_SHA256 = 'bae14fdcf77d358862d7a637f02f4199a2ead1ed76fc382c8e89e4bc87edaa4e'


class WindowsNumericSpacingProbes(unittest.TestCase):
    def cases(self):
        path = generate.ROOT / INDEX
        self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), INDEX_SHA256)
        return read_json(path)['fixtures']

    def test_all_100_actual_word_inputs_reproduce_from_the_unchanged_mac_parent(self):
        cases = {case['name']: case for case in self.cases()}
        generated = dict(generate.probes())
        self.assertEqual(len(cases), 100)
        self.assertEqual(set(cases), set(generated))
        self.assertEqual(Counter(c['probe_group'] for c in cases.values()),
                         dict(hints=32, styles=36, cjk=16, kerning=16))
        self.assertEqual(sum(not c['baseline_all_planes_pass'] for c in cases.values()), 84)
        parent = generate.ROOT / generate.PARENT
        self.assertEqual(hashlib.sha256(parent.read_bytes()).hexdigest(),
                         '24393143f3def21b3ad253dedf6d642e68d0f0087f9bb770726bf0d7a237bdf1')
        with ZipFile(parent) as original:
            for name, raw in generated.items():
                with self.subTest(case=name), ZipFile(BytesIO(raw)) as actual:
                    case = cases[name]
                    self.assertEqual(raw, checked_file(generate.ROOT, case['source']).read_bytes())
                    self.assertEqual(actual.namelist(), original.namelist())
                    changed = {part for part in actual.namelist()
                               if actual.read(part) != original.read(part)}
                    expected = {'word/document.xml'}
                    if case['input_specification']['compatibility_mode'] == 15:
                        expected.add('word/settings.xml')
                    self.assertEqual(changed, expected)

    def test_original_word_version_executable_inputs_fonts_and_exports_are_immutable(self):
        for case in self.cases():
            with self.subTest(case=case['name']):
                archive = checked_file(generate.ROOT, case['archive'])
                manifest = checked_file(generate.ROOT, case['capture'])
                source = checked_file(generate.ROOT, case['source'])
                capture = read_json(manifest)
                self.assertEqual(capture['word'], case['word'])
                self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
                self.assertEqual(capture['word']['executable_sha256'],
                                 '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                self.assertEqual(capture['capture_id'], case['capture_id'])
                document, = capture['documents']
                self.assertEqual(document['open_mode'], 'normal-read-only')
                self.assertEqual(document['name'], case['capture_document_name'])
                self.assertEqual(document['compatibility_mode'],
                                 case['input_specification']['compatibility_mode'])
                self.assertIn('word-' + capture['word']['product_version'], archive.parts)
                with ZipFile(archive) as bundle:
                    self.assertEqual(bundle.read('capture.json'), manifest.read_bytes())
                    self.assertEqual(bundle.read('inputs/' + document['name'] + '.docx'), source.read_bytes())
                    self.assertEqual(bundle.read(document['pdf']), (manifest.parent / document['pdf']).read_bytes())
                reference = checked_file(generate.ROOT, case['drawing_reference'])
                self.assertEqual(read_json(reference), extract(manifest))
                with pymupdf.open(manifest.parent / document['pdf']) as pdf:
                    for program in case['original_word_font_programs']:
                        name, extension, _, raw = pdf.extract_font(program['xref'])
                        self.assertEqual((name, extension),
                                         (program['postscript_name'], program['extension']))
                        self.assertEqual(hashlib.sha256(raw).hexdigest(), program['sha256'])

    def test_missing_subset_tables_use_versioned_font_units_with_original_program_provenance(self):
        metadata = read_json(generate.ROOT / 'tests/word-oracle/windows/aptos-short-shaping-metrics.json')
        data = checked_file(generate.ROOT, metadata['data'])
        checked_file(generate.ROOT, metadata['font_provenance'])
        self.assertEqual(hashlib.sha256(data.read_bytes()).hexdigest(), metadata['data_sha256'])
        self.assertEqual(metadata['uharfbuzz'], '0.57.0')
        self.assertEqual(metadata['pair_rows'], 8507)
        self.assertEqual([p['face'] for p in metadata['programs']], list(range(8)))
        self.assertTrue(all(p['version'] == ['Version 2.01;O365'] and p['upem'] == 2048
                            for p in metadata['programs']))
        originals = read_json(checked_file(generate.ROOT, metadata['font_provenance']))
        self.assertEqual({p['sha256'] for p in metadata['programs']},
                         {p['sha256'] for p in originals['actual_system_full_font_programs']})
        programs = [p for c in self.cases() if c['probe_group'] == 'kerning'
                    for p in c['original_word_font_programs']
                    if any(f in ('Aptos', 'Aptos Display') for f in p['legacy_family'])]
        self.assertTrue(programs)
        self.assertTrue(all(p['version'] == ['Version 2.01;O365']
                            and not p['has_GPOS'] and not p['has_GSUB'] for p in programs))


if __name__ == '__main__':
    unittest.main()
