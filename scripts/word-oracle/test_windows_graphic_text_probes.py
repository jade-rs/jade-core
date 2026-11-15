from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile

from generate_windows_graphic_text_probes import ROOT, generate
from check_windows_graphic_text import _font_control


class GraphicTextProbeTests(unittest.TestCase):
    def test_reproduces_all_original_word_input_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 46)
            for source in output.glob('*.docx'):
                with self.subTest(source=source.name):
                    self.assertEqual(source.read_bytes(),
                                     (ROOT / 'public/fixtures' / source.name).read_bytes())

    def test_font_control_only_removes_four_legacy_effect_flags(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            generate(output)
            original = ROOT / 'public/fixtures/char-effects.docx'
            control = output / 'word-windows-edge-graphic-original-plain-font-control.docx'
            _font_control(original, control)
            with ZipFile(original) as a, ZipFile(control) as b:
                self.assertIn(b'<w:vanish/>', a.read('word/document.xml'))
                self.assertIn(b'<w:vanish/>', b.read('word/document.xml'))
                self.assertEqual(a.read('word/fontTable.xml'), b.read('word/fontTable.xml'))


if __name__ == '__main__':
    unittest.main()
