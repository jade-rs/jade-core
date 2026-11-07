"""Negative cases for the strict vector border gate; no Word installation needed."""
import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock, patch

import pymupdf

import compare_windows_borders as b


class WindowsBorderTests(unittest.TestCase):
    def test_actual_painted_text_rejects_shifts_missing_glyphs_and_repagination(self):
        word = [[1,'A',72.,84.],[1,'B',80.,84.]]
        self.assertTrue(b.compare_visible_text(word, word)['passed'])
        for native in [[[1,'A',72.4,84.],[1,'B',80.,84.]],
                       [[1,'A',72.,84.]],
                       [[1,'B',72.,84.],[1,'A',80.,84.]],
                       [[2,'A',72.,84.],[2,'B',80.,84.]]]:
            with self.subTest(native=native):
                self.assertFalse(b.compare_visible_text(word, native)['passed'])

    def test_pdf_text_projection_retains_visible_text_and_rejects_hidden_ink(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)/'painted.pdf'
            doc = pymupdf.open()
            page = doc.new_page(width=100, height=100)
            page.insert_text((20,40), 'A', fontsize=12)
            page.insert_text((20,90), 'B', fontsize=12)
            stream = page.get_contents()[-1]
            doc.update_stream(stream, b'q\n0 40 100 60 re W n\n'+doc.xref_stream(stream)+b'\nQ\n')
            doc.save(path)
            doc.close()
            self.assertEqual(b.visible_pdf_glyphs(path), [[1,'A',20.,40.]])
            self.assertFalse(b.compare_visible_text([[1,'A',20.,40.],[1,'B',20.,90.]], b.visible_pdf_glyphs(path))['passed'])

    def test_fragmentation_does_not_change_painted_union(self):
        whole = [(10, 20, 100, 21)]
        fragments = [(10, 20, 40, 21), (40, 20, 70, 21), (70, 20, 100, 21)]
        self.assertFalse(b.uncovered(whole, fragments))
        self.assertFalse(b.uncovered(fragments, whole))

    def test_subpixel_shift_beyond_fixed_bound_fails(self):
        source = [(10, 20, 100, 21)]
        for dx, dy in [(0.4, 0), (0, 0.4), (-0.4, 0), (0, -0.4)]:
            moved = [(10 + dx, 20 + dy, 100 + dx, 21 + dy)]
            self.assertTrue(b.uncovered(source, moved))
            self.assertTrue(b.uncovered(moved, source))
        self.assertFalse(b.uncovered(source, [(10.375, 20.375, 100.375, 21.375)]))

    def test_missing_added_and_thinned_rules_fail(self):
        rules = [(10, 20, 100, 21), (10, 30, 100, 31)]
        self.assertTrue(b.uncovered(rules, rules[:1]))
        self.assertTrue(b.uncovered(rules + [(10, 40, 100, 41)], rules))
        self.assertTrue(b.uncovered([(10, 20, 100, 23)], [(10, 21, 100, 22)]))

    def test_equal_bounds_with_a_hole_fail(self):
        self.assertTrue(b.uncovered([(10, 20, 100, 21)],
                                    [(10, 20, 40, 21), (45, 20, 100, 21)]))

    def make_pdf(self, folder, name, *, shift=0, pages=1, erase=False, diagonal=False):
        path = folder / name
        with pymupdf.open() as document:
            for _ in range(pages):
                page = document.new_page(width=200, height=200)
                if diagonal:
                    page.draw_line((20, 20), (100, 100), color=(0, 0, 0), width=1)
                else:
                    page.draw_rect((20, 20 + shift, 100, 21 + shift),
                                   color=None, fill=(0, 0, 0))
                if erase:
                    page.draw_rect((40, 19, 50, 22), color=None, fill=(1, 1, 1))
            document.save(path)
        return path

    def test_real_pdf_equal_and_moved_geometry(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            word = self.make_pdf(root, 'word.pdf')
            native = self.make_pdf(root, 'native.pdf')
            moved = self.make_pdf(root, 'moved.pdf', shift=0.5)
            self.assertEqual(b.compare(word, native)[0]['word_uncovered_count'], 0)
            self.assertGreater(b.compare(word, moved)[0]['word_uncovered_count'], 0)
            self.assertGreater(b.compare(word, moved)[0]['native_uncovered_count'], 0)

    def test_white_overpaint_removes_previously_painted_black(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            word = self.make_pdf(root, 'word.pdf')
            erased = self.make_pdf(root, 'erased.pdf', erase=True)
            self.assertGreater(b.compare(word, erased)[0]['word_uncovered_count'], 0)
            self.assertEqual(b.compare(word, erased)[0]['native_uncovered_count'], 0)

    def test_page_loss_and_unsupported_diagonal_are_errors(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            word = self.make_pdf(root, 'word.pdf', pages=2)
            native = self.make_pdf(root, 'native.pdf')
            diagonal = self.make_pdf(root, 'diagonal.pdf', diagonal=True)
            with self.assertRaisesRegex(ValueError, 'Page count'):
                b.compare(word, native)
            with self.assertRaisesRegex(ValueError, 'Non-axis-aligned'):
                b.rectangles(diagonal)

    def test_source_checksum_and_path_escape_are_errors(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / 'source.pdf'
            source.write_bytes(b'immutable')
            digest = hashlib.sha256(source.read_bytes()).hexdigest()
            b.verify_hash(source, digest)
            source.write_bytes(b'changed')
            with self.assertRaisesRegex(ValueError, 'checksum'):
                b.verify_hash(source, digest)
            with self.assertRaisesRegex(ValueError, 'escapes'):
                b.checked_path(root, '../outside.pdf')

    def test_nonfinite_geometry_is_an_error(self):
        with self.assertRaisesRegex(ValueError, 'Nonfinite'):
            b.rect((0, float('nan'), 100, 10))

    def synthetic_page(self, drawings):
        page = MagicMock()
        page.rect = pymupdf.Rect(0, 0, 200, 200)
        page.get_images.return_value = []
        page.get_drawings.return_value = drawings
        document = MagicMock()
        document.__enter__.return_value = document
        document.__iter__.return_value = iter([page])
        return document

    def test_clip_scope_ends_before_following_sibling_paint(self):
        fill = dict(type='f', level=1, fill=(0, 0, 0), color=None,
                    fill_opacity=1, items=[('re', pymupdf.Rect(10, 20, 100, 21), 1)])
        draws = [dict(type='clip', level=0,
                      items=[('re', pymupdf.Rect(10, 20, 40, 21), 1)]),
                 fill, dict(fill, level=0)]
        with patch.object(b.pymupdf, 'open', return_value=self.synthetic_page(draws)):
            self.assertEqual(b.rectangles('synthetic'),
                             [[(10, 20, 40, 21), (10, 20, 100, 21)]])

    def test_unsupported_clipping_and_blending_fail_closed(self):
        for draw in [dict(type='clip', level=0, items=[('c',)]),
                     dict(type='group', level=0, blendmode='Multiply', opacity=1),
                     dict(type='group', level=0, blendmode='Normal', opacity=0.5),
                     dict(type='group', level=0, blendmode='Normal', opacity=1, knockout=True)]:
            with patch.object(b.pymupdf, 'open', return_value=self.synthetic_page([draw])):
                with self.assertRaises(ValueError):
                    b.rectangles('unsupported')


if __name__ == '__main__':
    unittest.main()
