import json
from pathlib import Path
import re
import tempfile
import unittest

from generate_windows_calibri_probes import ROOT, generate


class CalibriProbeTests(unittest.TestCase):
    def test_twelve_inputs_match_original_word_capture_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 12)
            sources = list(output.glob('*.docx'))
            self.assertEqual(len(sources), 12)
            for source in sources:
                with self.subTest(source=source.name):
                    self.assertEqual(source.read_bytes(),
                                     (ROOT / 'public/fixtures' / source.name).read_bytes())

    def test_windows_pairs_match_independent_original_font_gpos_facts(self):
        facts = json.loads((ROOT / 'tests/word-oracle/windows/calibri-kerning-font-facts.json')
                           .read_text())
        self.assertEqual(facts['font_version'], 'Version 6.27')
        self.assertEqual(facts['units_per_em'], 2048)
        self.assertEqual(facts['font_sha256'],
                         '57e93ff143572bcc092c1f56202d43ddc07bb119aa104c6d7af163470fb551f6')
        expected = [(ord(p['left']) << 8 | ord(p['right']), p['units'])
                    for p in facts['pairs']]
        source = (ROOT / 'crates/jade-docx-layout/src/line_metrics/windows_calibri_kern.rs')
        actual = [(int(pair, 16), int(units)) for pair, units in
                  re.findall(r'\(0x([0-9a-f]+), (-?\d+)\)', source.read_text())]
        self.assertEqual(len(expected), 611)
        self.assertEqual(expected, sorted(set(expected)))
        self.assertEqual(actual, expected)
