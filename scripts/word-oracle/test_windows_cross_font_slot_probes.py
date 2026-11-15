from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile

from generate_windows_cross_font_slot_probes import ROOT, generate, variants


class CrossFontSlotProbeTests(unittest.TestCase):
    def test_original_inputs_change_only_the_east_asian_font_slot(self):
        sources = ROOT / 'tests/word-oracle/windows/observations/sources'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 36)
            self.assertEqual(len(list(output.glob('*.docx'))), 36)
            self.assertEqual(len({name for name, _, _ in variants()}), 36)
            for name, parent, edits in variants():
                with self.subTest(name=name):
                    self.assertEqual((output / name).read_bytes(), (sources / name).read_bytes())
                    with ZipFile(sources / parent) as original, ZipFile(output / name) as generated:
                        self.assertEqual(original.namelist(), generated.namelist())
                        for part in original.namelist():
                            expected = original.read(part)
                            if part == 'word/document.xml':
                                for before, after in edits:
                                    expected = expected.replace(before.encode(), after.encode())
                            self.assertEqual(generated.read(part), expected)


if __name__ == '__main__':
    unittest.main()
