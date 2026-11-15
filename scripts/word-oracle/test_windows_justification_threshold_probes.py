from decimal import Decimal
from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile

from generate_windows_justification_threshold_probes import ROOT, WIDTHS, generate


class JustificationThresholdProbeTests(unittest.TestCase):
    def test_all_inputs_match_word_archives_and_only_change_the_right_indent(self):
        sources = ROOT / 'tests/word-oracle/windows/observations/sources'
        parent = sources / 'word-windows-edge-justify-width-yu20-both-width188-balance1-compat15.docx'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 4)
            files = list(output.glob('*.docx'))
            self.assertEqual(len(files), 4)
            for source in files:
                with self.subTest(source=source.name):
                    self.assertEqual(source.read_bytes(), (sources / source.name).read_bytes())
                    width = source.name.split('-width')[1].split('-')[0].replace('p', '.')
                    self.assertIn(width, WIDTHS)
                    indent = int((Decimal('425.2') - Decimal(width)) * 20)
                    with ZipFile(parent) as original, ZipFile(source) as generated:
                        self.assertEqual(original.namelist(), generated.namelist())
                        for name in original.namelist():
                            expected = original.read(name)
                            if name == 'word/document.xml':
                                expected = expected.replace(b'<w:ind w:right="4744"/>',
                                    f'<w:ind w:right="{indent}"/>'.encode())
                            self.assertEqual(generated.read(name), expected)


if __name__ == '__main__':
    unittest.main()
