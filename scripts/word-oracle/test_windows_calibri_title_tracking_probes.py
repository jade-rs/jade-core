"""Keep the original Windows Title tracking experiment reproducible."""
import hashlib
import json
from pathlib import Path
import re
import tempfile
import unittest
from zipfile import ZipFile

from generate_windows_calibri_title_tracking_probes import ROOT, PARENTS, SPACING_TWIPS, generate
from check_platform_coordinates import pdf_origins
from extract_windows_emf import extract


class CalibriTitleTracking(unittest.TestCase):
    def test_versioned_word_evidence_and_original_output_differences_are_preserved(self):
        read = lambda path: json.loads(path.read_text(encoding='utf-8-sig'))
        facts = read(ROOT / 'tests/word-oracle/windows/calibri-title-tracking-observations.json')
        self.assertEqual(facts['schema'], 1)
        self.assertEqual(len(facts['cases']), 16)
        self.assertEqual(sum(c['baseline_all_planes_pass'] for c in facts['cases']), 6)
        self.assertEqual(facts['baseline_native']['source_commit'],
                         'b59df21db4ab6aa8ac70fee8215d201de6af1f8a')
        registry = read(ROOT / 'tests/word-oracle/windows/observations/index.json')
        captures = {c['capture_id']: c for c in registry['captures']}
        for case in facts['cases']:
            with self.subTest(case=case['name']):
                source = ROOT / case['source']['file']
                self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(),
                                 case['source']['sha256'])
                self.assertEqual(case['word_product_version'], '16.0.19127.20936')
                record = captures[case['capture_id']]
                self.assertEqual(record['archive'], case['archive']['file'])
                self.assertEqual(record['archive_sha256'], case['archive']['sha256'])
                self.assertEqual(record['word']['executable_sha256'], case['word_executable_sha256'])
                archive = ROOT / record['archive']
                self.assertEqual(hashlib.sha256(archive.read_bytes()).hexdigest(),
                                 record['archive_sha256'])
                with tempfile.TemporaryDirectory() as temporary:
                    directory = Path(temporary)
                    with ZipFile(archive) as bundle:
                        bundle.extractall(directory)
                    capture = read(directory / 'capture.json')
                    document, = capture['documents']
                    self.assertEqual(document['sha256'], case['source']['sha256'])
                    self.assertEqual(document['open_mode'], 'normal-read-only')
                    drawing, = extract(directory / 'capture.json')['fixtures']
                    pdf = pdf_origins(directory / document['pdf'])
                    emf_title = [g for g in drawing['glyphs'] if g[0] == 1 and abs(g[3] - 71.4) < .3]
                    pdf_title = [g for g in pdf['glyphs'] if g[0] == 1 and abs(g[3] - 71.4) < .3]
                    self.assertEqual([g[1] for g in emf_title], [g[1] for g in pdf_title])
                    gap = max(abs(a[2] - b[2]) for a, b in zip(emf_title, pdf_title))
                    self.assertAlmostEqual(gap, case['original_title_max_pdf_emf_gap_pt'], places=5)

    def test_all_captured_inputs_reproduce_with_only_one_style_property_changed(self):
        sources = ROOT / 'tests/word-oracle/windows/observations/sources'
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'inputs'
            self.assertEqual(generate(output), 16)
            for label, digest in PARENTS.items():
                parent = sources / f'word-windows-float-lookahead-{label}-compat15.docx'
                self.assertEqual(hashlib.sha256(parent.read_bytes()).hexdigest(), digest)
                with ZipFile(parent) as original:
                    for spacing in SPACING_TWIPS:
                        name = f'word-windows-title-calibri22-{label}-spacing{spacing}-compat15.docx'
                        self.assertEqual((output / name).read_bytes(), (sources / name).read_bytes())
                        with ZipFile(output / name) as actual:
                            self.assertEqual(actual.namelist(), original.namelist())
                            for part in original.namelist():
                                expected = original.read(part)
                                if part == 'word/styles.xml':
                                    def replace_title(match):
                                        title = match[0]
                                        self.assertEqual(title.count(b'<w:spacing w:val="5"/>'), 1)
                                        return title.replace(b'<w:spacing w:val="5"/>',
                                                             f'<w:spacing w:val="{spacing}"/>'.encode())
                                    expected, count = re.subn(
                                        rb'<w:style\b[^>]*w:styleId="Title"[^>]*>.*?</w:style>',
                                        replace_title, expected, flags=re.S)
                                    self.assertEqual(count, 1)
                                self.assertEqual(actual.read(part), expected, (name, part))


if __name__ == '__main__':
    unittest.main()
