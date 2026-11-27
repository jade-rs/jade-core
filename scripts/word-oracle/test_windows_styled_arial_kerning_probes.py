import hashlib
import json
import tempfile
from pathlib import Path
import unittest

from generate_windows_styled_arial_kerning_probes import ROOT, generate


class StyledArialProbes(unittest.TestCase):
    def test_all_original_word_sources_reproduce_byte_for_byte(self):
        facts = json.loads((ROOT / 'tests/word-oracle/windows/styled-arial-kerning-font-facts.json').read_text(encoding='utf8'))
        self.assertEqual(len(facts['faces']), 3)
        self.assertEqual(len({key for face in facts['faces'] for key, _ in face['pairs']}), 97)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 12)
            self.assertEqual(len(facts['word_pair_probes']), 12)
            for record in facts['word_pair_probes']:
                with self.subTest(name=record['name']):
                    source = output / (record['name'] + '.docx')
                    self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), record['source_sha256'])


if __name__ == '__main__':
    unittest.main()
