from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile
import xml.etree.ElementTree as ET

from generate_windows_table_row_probes import ROOT, generate


class TableRowProbeTests(unittest.TestCase):
    def test_reproduces_original_word_input_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 90)
            for source in output.glob('*.docx'):
                with self.subTest(source=source.name):
                    self.assertEqual(source.read_bytes(),
                                     (ROOT / 'public/fixtures' / source.name).read_bytes())

    def test_row_exceptions_keep_their_scope_and_source_order(self):
        w = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            generate(output)
            sources = list(output.glob('*-row-indent-*.docx'))
            self.assertEqual(len(sources), 46)
            for source in sources:
                with self.subTest(source=source.name), ZipFile(source) as archive:
                    document = ET.fromstring(archive.read('word/document.xml'))
                    rows = document.findall(f'.//{w}tbl/{w}tr')
                    target = 1 if '-row2-' in source.name else 0
                    self.assertEqual(len(rows), 2)
                    self.assertEqual(rows[target][0].tag, w + 'tblPrEx')
                    self.assertIsNotNone(rows[target].find(f'{w}tblPrEx/{w}tblInd'))
                    self.assertIsNone(rows[1 - target].find(w + 'tblPrEx'))
                    for spacing in document.findall(f'.//{w}rPr/{w}spacing'):
                        self.assertNotIn(w + 'before', spacing.attrib)


if __name__ == '__main__':
    unittest.main()
