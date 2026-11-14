from pathlib import Path
import json
import tempfile
import unittest
from zipfile import ZipFile

from generate_windows_dot_leader_probes import ROOT, generate


class WindowsDotLeaderTests(unittest.TestCase):
    def test_generated_inputs_match_original_word_capture_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 28)
            sources = list(output.glob('*.docx'))
            self.assertEqual(len(sources), 28)
            for source in sources:
                with self.subTest(source=source.name):
                    self.assertEqual(source.read_bytes(),
                                     (ROOT / 'public/fixtures' / source.name).read_bytes())
                    with ZipFile(source) as archive:
                        self.assertNotIn('word/settings.xml', archive.namelist())

    def test_font_facts_retain_original_period_advance_units(self):
        facts = json.loads((ROOT / 'tests/word-oracle/windows/dot-leader-font-facts.json').read_text())
        self.assertEqual(facts['word_product_version'], '16.0.19127.20936')
        observed = {entry['family']: entry['period_advance'] / entry['units_per_em']
                    for entry in facts['fonts']}
        self.assertEqual(observed, {'Arial': 569 / 2048, 'Century': 569 / 2048,
                                    'MS Mincho': 128 / 256, 'Yu Mincho': 559 / 2048})
        for entry in facts['fonts']:
            self.assertEqual(entry['style'], 'Regular')
            self.assertRegex(entry['font_program_sha256'], r'^[a-f0-9]{64}$')
