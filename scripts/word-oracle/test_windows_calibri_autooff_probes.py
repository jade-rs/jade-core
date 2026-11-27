from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile

from generate_windows_calibri_autooff_probes import ROOT, generate, variants


class CalibriAutooffProbeTests(unittest.TestCase):
    def test_original_inputs_change_only_the_auto_spacing_switch(self):
        sources = ROOT / 'tests/word-oracle/windows/observations/sources'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 36)
            self.assertEqual(len(list(output.glob('*.docx'))), 36)
            for name, parent in variants():
                with self.subTest(name=name):
                    self.assertEqual((output / name).read_bytes(), (sources / name).read_bytes())
                    with ZipFile(sources / parent) as original, ZipFile(output / name) as generated:
                        self.assertEqual(original.namelist(), generated.namelist())
                        for part in original.namelist():
                            expected = original.read(part)
                            if part == 'word/document.xml':
                                expected = expected.replace(b'<w:jc w:val="left"/>',
                                    b'<w:autoSpaceDE w:val="0"/><w:jc w:val="left"/>')
                            self.assertEqual(generated.read(part), expected)


if __name__ == '__main__':
    unittest.main()
