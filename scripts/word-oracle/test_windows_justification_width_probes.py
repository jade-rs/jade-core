from decimal import Decimal
from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile
import xml.etree.ElementTree as ET

from generate_windows_justification_width_probes import ROOT, WIDTHS, generate


class JustificationWidthProbeTests(unittest.TestCase):
    def test_controls_match_original_word_inputs_and_only_change_document(self):
        w = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 18)
            sources = list(output.glob('*.docx'))
            self.assertEqual(len(sources), 18)
            archived = ROOT / 'tests/word-oracle/windows/observations/sources'
            for source in sources:
                with self.subTest(source=source.name):
                    self.assertEqual(source.read_bytes(), (archived / source.name).read_bytes())
                    balance = source.stem.split('-balance')[1].split('-')[0]
                    parent = archived / f'word-windows-edge-justify-tiers-yu20-cjk3-space3-left-balance{balance}-compat15.docx'
                    with ZipFile(source) as actual, ZipFile(parent) as original:
                        self.assertEqual(actual.namelist(), original.namelist())
                        for name in original.namelist():
                            if name != 'word/document.xml':
                                self.assertEqual(actual.read(name), original.read(name))
                        document = ET.fromstring(actual.read('word/document.xml'))
                        self.assertEqual([t.text for t in document.iter(w + 't')], ['Ag日本語 ABC ABC', 'X'])
                        self.assertEqual(len(list(document.iter(w + 'br'))), 1)
                        indent = document.find('.//' + w + 'ind')
                        if '-left-' in source.name:
                            self.assertIsNone(indent)
                        else:
                            width = int(source.name.split('-width')[2].split('-')[0])
                            self.assertIn(width, WIDTHS)
                            self.assertEqual(indent.attrib, {
                                w + 'right': str(int((Decimal('425.2') - Decimal(width)) * 20))})


if __name__ == '__main__':
    unittest.main()
