from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile
import xml.etree.ElementTree as ET

from generate_windows_header_table_probes import ROOT, generate


class HeaderTableProbeTests(unittest.TestCase):
    def test_all_ordered_inputs_match_original_word_captures(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 10)
            files = list(output.glob('*.docx'))
            self.assertEqual(len(files), 10)
            for source in files:
                with self.subTest(source=source.name):
                    self.assertEqual(source.read_bytes(),
                                     (ROOT / 'public/fixtures' / source.name).read_bytes())

    def test_direct_properties_follow_the_ooxml_table_property_order(self):
        namespace = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
        # CT_TblPr: Word can disregard formatting placed before tblStyle.
        sequence = ['tblStyle', 'tblW', 'jc', 'tblInd', 'tblLook']
        for source in (ROOT / 'public/fixtures').glob('word-windows-edge-header-table-*-ordered-*.docx'):
            with ZipFile(source) as archive:
                for name in ('word/document.xml', 'word/header1.xml', 'word/footer1.xml'):
                    table, = ET.fromstring(archive.read(name)).iter(namespace + 'tblPr')
                    ranks = [sequence.index(child.tag.removeprefix(namespace)) for child in table]
                    with self.subTest(source=source.name, story=name):
                        self.assertEqual(ranks, sorted(ranks))
