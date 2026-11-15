import copy
from pathlib import Path
import tempfile
import unittest

import pymupdf

import check_windows_graphic_text as check


def rectangle(offset=0):
    return dict(page=1, type='s', color=(0., 0., 0.), stroke_opacity=1., width=.14,
                lineCap=(0, 0, 0), lineJoin=1., dashes='[] 0', closePath=False,
                items=[('re', pymupdf.Rect(10 + offset, 20, 30 + offset, 40), 1)])


class GraphicTextTests(unittest.TestCase):
    def test_registered_originals_retain_all_graphic_cases(self):
        self.assertEqual(len(check.verify()), 62)

    def test_missing_registry_or_changed_evidence_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / check.INDEX).parent.mkdir(parents=True)
            (root / check.INDEX).write_bytes(b'{"schema":1,"fixtures":[]}')
            with self.assertRaisesRegex(ValueError, 'coverage/expectations'):
                check.verify(root)

    def test_outline_positions_are_absolute_and_paths_cannot_disappear(self):
        check.compare_outlines([rectangle()], [rectangle()])
        for native in [[], [rectangle(1.)]]:
            with self.subTest(native=native), self.assertRaises(ValueError):
                check.compare_outlines([rectangle()], native)

    def test_outline_style_and_width_are_asserted(self):
        for key, value in [('width', 0.), ('width', .75), ('color', (1., 0., 0.)),
                           ('lineJoin', 0.), ('stroke_opacity', .5), ('type', 'f')]:
            native = rectangle()
            native[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                check.compare_outlines([rectangle()], [native])

    def test_curve_interiors_are_checked_even_with_identical_endpoints(self):
        word = rectangle()
        word['items'] = [('c', pymupdf.Point(10, 20), pymupdf.Point(10, 40),
                          pymupdf.Point(30, 40), pymupdf.Point(30, 20))]
        native = copy.deepcopy(word)
        native['items'] = [('c', pymupdf.Point(10, 20), pymupdf.Point(10, 50),
                            pymupdf.Point(30, 50), pymupdf.Point(30, 20))]
        with self.assertRaisesRegex(ValueError, 'geometry'):
            check.compare_outlines([word], [native])

    def test_invisible_search_text_is_separate_from_visible_paint(self):
        with tempfile.TemporaryDirectory() as directory, pymupdf.open() as document:
            page = document.new_page()
            page.insert_text((20, 40), 'A')
            page.insert_text((20, 60), 'B', render_mode=3)
            pdf = Path(directory) / 'search-layer.pdf'
            document.save(pdf)
            all_glyphs, visible, colors, _, images = check.pdf_paint(pdf)
            self.assertEqual(''.join(g[1] for g in all_glyphs['glyphs']), 'AB')
            self.assertEqual(''.join(g[1] for g in visible['glyphs']), 'A')
            self.assertEqual(sum(len(group) for group in colors.values()), 1)
            self.assertEqual(images, 0)

    def test_text_paint_trace_does_not_drop_nearby_shadow_copies(self):
        with tempfile.TemporaryDirectory() as directory, pymupdf.open() as document:
            page = document.new_page()
            page.insert_text((20, 40), 'A', fontsize=40, color=(.75, .75, .75))
            page.insert_text((20.5, 40.5), 'A', fontsize=40)
            pdf = Path(directory) / 'two-painted-origins.pdf'
            document.save(pdf)
            all_glyphs, visible, colors, _, _ = check.pdf_paint(pdf)
            self.assertEqual(len(all_glyphs['glyphs']), 2)
            self.assertEqual(len(visible['glyphs']), 2)
            self.assertEqual(len(colors), 2)


if __name__ == '__main__':
    unittest.main()
