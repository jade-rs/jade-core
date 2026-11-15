from decimal import Decimal
from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile

from generate_windows_manual_size_threshold_probes import ROOT, PARENT, generate


class ManualSizeThresholdProbeTests(unittest.TestCase):
    def test_original_inputs_and_all_unrelated_package_parts_are_preserved(self):
        sources = ROOT / 'tests/word-oracle/windows/observations/sources'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 12)
            files = list(output.glob('*.docx'))
            self.assertEqual(len(files), 12)
            for source in files:
                with self.subTest(source=source.name):
                    self.assertEqual(source.read_bytes(), (sources / source.name).read_bytes())
                    size = int(source.name.split('-yu')[1].split('-')[0])
                    width = Decimal(source.name.split('-width')[1].split('-')[0].replace('p', '.'))
                    indent = int((Decimal('425.2') - width) * 20)
                    with ZipFile(sources / PARENT) as parent, ZipFile(source) as generated:
                        self.assertEqual(parent.namelist(), generated.namelist())
                        for name in parent.namelist():
                            expected = parent.read(name)
                            if name == 'word/document.xml':
                                for before, after in (
                                    (b'<w:sz w:val="40"/>', f'<w:sz w:val="{size * 2}"/>'.encode()),
                                    (b'<w:szCs w:val="40"/>', f'<w:szCs w:val="{size * 2}"/>'.encode()),
                                    (b'<w:ind w:right="4744"/>', f'<w:ind w:right="{indent}"/>'.encode()),
                                ):
                                    expected = expected.replace(before, after)
                            self.assertEqual(generated.read(name), expected)


if __name__ == '__main__':
    unittest.main()
