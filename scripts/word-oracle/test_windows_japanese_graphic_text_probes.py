from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile
import xml.etree.ElementTree as ET

from generate_windows_japanese_graphic_text_probes import ROOT, TEXT, generate


class JapaneseGraphicTextProbeTests(unittest.TestCase):
    def test_reproduces_all_captured_input_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 15)
            for source in output.glob('*.docx'):
                with self.subTest(source=source.name):
                    self.assertEqual(source.read_bytes(),
                                     (ROOT / 'public/fixtures' / source.name).read_bytes())

    def test_only_body_text_and_explicit_font_slots_change(self):
        w = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            generate(output)
            for source in output.glob('*.docx'):
                _, effect_size = source.stem.split('ja-yu-mincho-')
                base = ROOT / 'public/fixtures' / (
                    'word-windows-edge-graphic-' + effect_size.replace('-compat15', '-colorauto-compat15') + '.docx')
                with self.subTest(source=source.name), ZipFile(source) as a, ZipFile(base) as b:
                    self.assertEqual(set(a.namelist()), set(b.namelist()))
                    for name in a.namelist():
                        if name != 'word/document.xml':
                            self.assertEqual(a.read(name), b.read(name))
                    body = ET.fromstring(a.read('word/document.xml'))
                    self.assertEqual([t.text for t in body.iter(w + 't')], [TEXT + ' ', TEXT])
                    fonts = list(body.iter(w + 'rFonts'))
                    self.assertEqual(len(fonts), 2)
                    self.assertTrue(all(set(font.attrib.values()) == {'Yu Mincho'} for font in fonts))


if __name__ == '__main__':
    unittest.main()
