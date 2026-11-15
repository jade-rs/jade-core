from pathlib import Path
import tempfile
import unittest

import pymupdf

from check_platform_coordinates import compare
import check_windows_graphic_text as check
from pdf_actualtext_aliases import resolve_actualtext_aliases


class ActualTextAliasTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        case, source, manifest, drawing, word_pdf, font_pdf = next(
            row for row in check.verify()
            if row[0]['name'] == 'word-windows-edge-graphic-ja-yu-mincho-plain-size12-compat15')
        cls.program = None
        with pymupdf.open(word_pdf) as document:
            for reference in document[0].get_fonts(full=True):
                data = document.extract_font(reference[0])[3]
                if not data:
                    continue
                font = pymupdf.Font(fontbuffer=data)
                if font.has_glyph(0x65e5, fallback=False) == font.has_glyph(0x2f47, fallback=False) != 0:
                    cls.program = data
                    break
        if cls.program is None:
            raise AssertionError('Original Word font no longer contains the measured day/sun alias')

    def make_pdf(self, path, replacement=None, duplicate=False, shift=0):
        with pymupdf.open() as document:
            page = document.new_page(width=240, height=120)
            page.insert_font(fontname='OracleAlias', fontbuffer=self.program)
            page.insert_text((20, 40), '\u2f47', fontname='OracleAlias', fontsize=20)
            canonical = chr(page.get_texttrace()[0]['chars'][0][0])
            actual = replacement or ('\u65e5' if canonical != '\u65e5' else '\u2f47')
            stream, = page.get_contents()
            content = document.xref_stream(stream)
            self.assertNotIn(b'/ActualText', content)
            marked = (b'/Span << /ActualText <FEFF' + actual.encode('utf-16-be').hex().encode()
                      + b'> >> BDC\n' + content + b'\nEMC\n')
            if shift:
                marked = f'q\n1 0 0 1 {shift} 0 cm\n'.encode() + marked + b'\nQ\n'
            document.update_stream(stream, marked)
            if duplicate:
                page.insert_text((80, 40), '\u2f47', fontname='OracleAlias', fontsize=20)
            document.save(path)
        return canonical, actual

    def test_proven_font_alias_uses_explicit_unicode_without_moving_ink(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'alias.pdf'
            canonical, actual = self.make_pdf(path)
            with pymupdf.open(path) as document:
                glyph = document[0].get_texttrace()[0]['chars'][0]
            original = [1, chr(glyph[0]), *glyph[2]]
            painted = [original.copy()]
            self.assertEqual(resolve_actualtext_aliases(path, painted), 1)
            self.assertNotEqual(canonical, actual)
            self.assertEqual(painted, [[1, actual, *original[2:]]])
            all_glyphs, visible, colors, _, _ = check.pdf_paint(path)
            self.assertEqual(all_glyphs['glyphs'], visible['glyphs'])
            self.assertEqual(visible['glyphs'][0][1], actual)
            self.assertEqual(next(iter(colors.values()))[0][1], actual)

    def test_actualtext_cannot_disguise_a_different_painted_glyph(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'wrong-glyph.pdf'
            self.make_pdf(path, replacement='\u672c')
            with self.assertRaisesRegex(ValueError, 'changes the painted glyph'):
                check.pdf_paint(path)

    def test_unmarked_literal_alias_is_not_globally_renamed(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'mixed-meaning.pdf'
            self.make_pdf(path, duplicate=True)
            with self.assertRaisesRegex(ValueError, 'Cannot safely disambiguate'):
                check.pdf_paint(path)

    def test_alias_resolution_cannot_align_displaced_text(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'displaced.pdf'
            _, actual = self.make_pdf(path, shift=2)
            painted = check.pdf_paint(path)[0]
            self.assertAlmostEqual(painted['glyphs'][0][2], 22.)
            with self.assertRaises(ValueError):
                compare(dict(page_count=1, glyphs=[[1, actual, 20., 40.]]), painted, 'absolute alias')


if __name__ == '__main__':
    unittest.main()
