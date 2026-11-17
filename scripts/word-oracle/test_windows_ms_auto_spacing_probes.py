"""Check reproduced input bytes and the independent Word spacing measurements."""
from io import BytesIO
import hashlib
import json
import unittest
from zipfile import ZipFile

import generate_windows_ms_auto_spacing_probes as generate
from check_platform_coordinates import checked_file, read_json, compare
from extract_windows_emf import extract


class WindowsMsAutoSpacingProbes(unittest.TestCase):
    def cases(self):
        return read_json(generate.ROOT / 'tests/word-oracle/windows/ms-fixed-auto-spacing-observations.json')['cases']

    def test_all_actual_word_inputs_reproduce_with_only_the_document_part_changed(self):
        cases = {c['name']: c for c in self.cases()}
        generated = dict(generate.probes())
        self.assertEqual(len(cases), 48)
        self.assertEqual(set(cases), set(generated))
        parent_path = generate.ROOT / generate.PARENT
        with ZipFile(parent_path) as parent:
            for name, data in generated.items():
                with self.subTest(case=name), ZipFile(BytesIO(data)) as actual:
                    case = cases[name]
                    self.assertEqual(case['parent_source'], generate.PARENT.as_posix())
                    self.assertEqual(case['parent_sha256'], hashlib.sha256(parent_path.read_bytes()).hexdigest())
                    self.assertEqual(data, checked_file(generate.ROOT, case['source']).read_bytes())
                    self.assertEqual(actual.namelist(), parent.namelist())
                    for part in parent.namelist():
                        if part != 'word/document.xml':
                            self.assertEqual(actual.read(part), parent.read(part))

    def test_original_word_versions_sources_and_quarter_em_boundaries(self):
        originals = {}
        cases = self.cases()
        self.assertEqual(sum(not c['baseline_all_planes_pass'] for c in cases), 16)
        for case in cases:
            with self.subTest(case=case['name']):
                archive = checked_file(generate.ROOT, case['archive'])
                with ZipFile(archive) as bundle:
                    capture = json.loads(bundle.read('capture.json').decode('utf-8-sig'))
                    self.assertEqual(capture['platform'], 'windows')
                    self.assertEqual(capture['status'], 'complete')
                    self.assertEqual(capture['capture_id'], case['capture_id'])
                    self.assertEqual(capture['word']['product_version'], case['word_product_version'])
                    self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
                    self.assertEqual(capture['word']['executable_sha256'], case['word_executable_sha256'])
                    self.assertEqual(capture['word']['executable_sha256'],
                                     '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                    document, = capture['documents']
                    self.assertEqual(document['open_mode'], 'normal-read-only')
                    self.assertEqual(document['sha256'], case['source']['sha256'])
                    self.assertEqual(bundle.read('inputs/' + case['name'] + '.docx'),
                                     checked_file(generate.ROOT, case['source']).read_bytes())
                reference_path = checked_file(generate.ROOT, case['drawing_reference'])
                original = extract(reference_path.parent / 'capture.json')
                self.assertEqual(original, read_json(reference_path))
                drawing, = original['fixtures']
                self.assertEqual(drawing['page_count'], 1)
                self.assertEqual(''.join(g[1] for g in drawing['glyphs']), case['text'])
                originals[(case['font_family'], case['font_size_pt'], case['text'], case['auto_space_de'])] = drawing
        for family in ['ＭＳ 明朝', 'ＭＳ ゴシック']:
            for size in [10.5, 11, 20]:
                for text in ['亜A', 'A亜']:
                    enabled = originals[(family, size, text, True)]
                    implicit = originals[(family, size, text, None)]
                    disabled = originals[(family, size, text, False)]
                    with self.subTest(family=family, size=size, text=text):
                        compare(enabled, implicit, 'original implicit/enabled Word boundary')
                        delta = enabled['glyphs'][1][2] - disabled['glyphs'][1][2]
                        # One original 600dpi printer pixel permits only the
                        # Word output's rounding, with no renderer positions.
                        self.assertLessEqual(abs(delta - size / 4), .12)


if __name__ == '__main__':
    unittest.main()
