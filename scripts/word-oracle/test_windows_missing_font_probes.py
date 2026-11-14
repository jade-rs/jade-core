from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET
from zipfile import ZipFile

from generate_windows_missing_font_probes import CASES, ROOT, generate


class MissingFontProbeTests(unittest.TestCase):
    def test_reproduces_original_word_input_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 9)
            for source in output.glob('*.docx'):
                with self.subTest(source=source.name):
                    self.assertEqual(source.read_bytes(),
                                     (ROOT / 'public/fixtures' / source.name).read_bytes())

    def test_controls_keep_body_text_and_have_no_panose_override(self):
        w = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
        with ZipFile(ROOT / 'public/fixtures/hello.docx') as archive:
            document = archive.read('word/document.xml')
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            generate(output)
            for label, face, family in CASES:
                with self.subTest(label=label), ZipFile(output / (
                        f'word-windows-edge-font-independent-{label}-hello-compat15.docx')) as archive:
                    self.assertEqual(archive.read('word/document.xml'), document)
                    table = ET.fromstring(archive.read('word/fontTable.xml'))
                    font, = [f for f in table if f.get(w + 'name') == face]
                    self.assertEqual(font.find(w + 'family').get(w + 'val'), family)
                    self.assertIsNone(font.find(w + 'panose1'))


if __name__ == '__main__':
    unittest.main()
