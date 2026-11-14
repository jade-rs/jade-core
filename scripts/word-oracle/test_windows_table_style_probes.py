from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile
import xml.etree.ElementTree as ET

from generate_windows_table_style_probes import ROOT, generate

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


class TableStyleProbeTests(unittest.TestCase):
    def test_all_inputs_reproduce_original_word_source_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 76)
            sources = list(output.glob('*.docx'))
            self.assertEqual(len(sources), 76)
            for source in sources:
                with self.subTest(source=source.name):
                    self.assertEqual(source.read_bytes(),
                                     (ROOT / 'public/fixtures' / source.name).read_bytes())

    def test_default_style_controls_preserve_the_distinction_from_missing_styles(self):
        fixtures = ROOT / 'public/fixtures'
        for mode in (12, 14, 15):
            for variant in ('default-zero', 'default-omitted', 'no-styles'):
                name = f'word-windows-edge-table-style-{variant}-directomitted-compat{mode}'
                with self.subTest(name=name), ZipFile(fixtures / (name + '.docx')) as archive:
                    document = ET.fromstring(archive.read('word/document.xml'))
                    self.assertIsNone(document.find(f'.//{W}tblStyle'))
                    settings = ET.fromstring(archive.read('word/settings.xml'))
                    compatibility = [e.get(W + 'val') for e in settings.iter(W + 'compatSetting')
                                     if e.get(W + 'name') == 'compatibilityMode']
                    self.assertEqual(compatibility, [str(mode)])
                    if variant == 'no-styles':
                        self.assertNotIn('word/styles.xml', archive.namelist())
                        self.assertNotIn(b'/styles"', archive.read('word/_rels/document.xml.rels'))
                        self.assertNotIn(b'/word/styles.xml', archive.read('[Content_Types].xml'))
                    else:
                        styles = ET.fromstring(archive.read('word/styles.xml'))
                        default, = [s for s in styles.findall(W + 'style')
                                    if s.get(W + 'type') == 'table' and s.get(W + 'default') == '1']
                        self.assertEqual(default.get(W + 'styleId'), 'TableNormal')
                        self.assertEqual(bool(list(styles.iter(W + 'tblInd'))),
                                         variant == 'default-zero')


if __name__ == '__main__':
    unittest.main()
