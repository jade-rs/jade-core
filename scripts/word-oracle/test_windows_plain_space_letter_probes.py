from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile
from generate_windows_plain_space_letter_probes import ROOT, generate, variants


class PlainSpaceLetterProbeTests(unittest.TestCase):
    def test_all_originals_and_unrelated_package_parts_are_preserved(self):
        sources = ROOT / 'tests/word-oracle/windows/observations/sources'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 16)
            self.assertEqual(len(list(output.glob('*.docx'))), 16)
            for name, parent, letter, disable_auto in variants():
                with self.subTest(name=name):
                    self.assertEqual((output / name).read_bytes(), (sources / name).read_bytes())
                    with ZipFile(sources / parent) as original, ZipFile(output / name) as generated:
                        self.assertEqual(original.namelist(), generated.namelist())
                        for part in original.namelist():
                            expected = original.read(part)
                            if part == 'word/document.xml':
                                text = expected.decode()
                                if disable_auto:
                                    text = text.replace('<w:jc w:val="left"/>', '<w:autoSpaceDE w:val="0"/><w:jc w:val="left"/>')
                                expected = text.replace('Ag日本語 ABCABC', f'Ag日本語 {letter}BCABC').encode()
                            self.assertEqual(generated.read(part), expected)


if __name__ == '__main__':
    unittest.main()
