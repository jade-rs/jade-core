from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile
import xml.etree.ElementTree as ET

from generate_windows_heading_break_probes import ROOT, generate


class HeadingBreakProbeTests(unittest.TestCase):
    def test_inputs_reproduce_original_word_source_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 52)
            sources = list(output.glob('*.docx'))
            self.assertEqual(len(sources), 52)
            for source in sources:
                with self.subTest(source=source.name):
                    self.assertEqual(source.read_bytes(),
                                     (ROOT / 'public/fixtures' / source.name).read_bytes())

    def test_accepted_controls_keep_paragraph_spacing_out_of_character_spacing(self):
        w = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            generate(output)
            sources = list(output.glob('*-ppr-only-compat*.docx'))
            self.assertEqual(len(sources), 26)
            for source in sources:
                with self.subTest(source=source.name), ZipFile(source) as archive:
                    document = ET.fromstring(archive.read('word/document.xml'))
                    for spacing in document.findall(f'.//{w}rPr/{w}spacing'):
                        self.assertEqual(spacing.attrib, {w + 'val': '0'})
                    headings = [p for p in document.iter(w + 'p')
                                if ''.join(p.itertext()) == 'Second page heading']
                    self.assertEqual(len(headings), 1)
                    self.assertIn(headings[0].find(f'{w}pPr/{w}spacing').get(w + 'before'),
                                  ('0', '480'))


if __name__ == '__main__':
    unittest.main()
