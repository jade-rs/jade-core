from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile

from generate_windows_small_slack_probes import ROOT, generate, variants


class SmallSlackProbeTests(unittest.TestCase):
    def test_each_source_matches_the_original_capture_and_preserves_other_parts(self):
        sources = ROOT / 'tests/word-oracle/windows/observations/sources'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 12)
            self.assertEqual(len(list(output.glob('*.docx'))), 12)
            for name, parent, changes in variants():
                with self.subTest(name=name):
                    self.assertEqual((output / name).read_bytes(), (sources / name).read_bytes())
                    with ZipFile(sources / parent) as original, ZipFile(output / name) as generated:
                        self.assertEqual(original.namelist(), generated.namelist())
                        for part in original.namelist():
                            expected = original.read(part)
                            if part == 'word/document.xml':
                                for before, after in changes:
                                    expected = expected.replace(before.encode(), after.encode())
                            self.assertEqual(generated.read(part), expected)


if __name__ == '__main__':
    unittest.main()
