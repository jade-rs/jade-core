"""Reject shading drift independently of otherwise matching text origins."""
from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile

import pymupdf

from paragraph_backgrounds import compare_paragraph_backgrounds


class ParagraphBackgrounds(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.source = self.root / 'source.docx'
        with ZipFile(self.source, 'w') as archive:
            archive.writestr('word/document.xml',
                '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
                '<w:body><w:p><w:pPr><w:shd w:fill="D9EAD3"/></w:pPr></w:p></w:body></w:document>')
        self.color = (217 / 255, 234 / 255, 211 / 255)
        self.boxes = [(80, 90, 300, 108), (80, 108, 300, 126)]
        self.word = self.pdf('word', self.boxes)

    def pdf(self, name, boxes, *, color=None, opacity=1, extra_page=False, erase=False):
        path = self.root / (name + '.pdf')
        with pymupdf.open() as document:
            page = document.new_page()
            for box in boxes:
                page.draw_rect(box, color=None, fill=color or self.color, fill_opacity=opacity)
            if erase:
                page.draw_rect((81, 91, 82, 92), color=None, fill=(1, 1, 1))
            if extra_page:
                document.new_page()
            document.save(path)
        return path

    def check(self, actual):
        return compare_paragraph_backgrounds(self.source, self.word, actual, 'test')

    def test_generated_controls_match_all_captured_inputs(self):
        from generate_windows_paragraph_shading_probes import ROOT, generate
        output = self.root / 'generated'
        self.assertEqual(generate(output), 45)
        files = list(output.glob('*.docx'))
        self.assertEqual(len(files), 45)
        with ZipFile(ROOT / 'public/fixtures/2col.docx') as archive:
            original = {name: archive.read(name) for name in archive.namelist()}
        for source in files:
            with self.subTest(source=source.name):
                captured = ROOT / 'tests/word-oracle/windows/observations/sources' / source.name
                self.assertEqual(source.read_bytes(), captured.read_bytes())
                with ZipFile(source) as archive:
                    self.assertEqual(set(archive.namelist()), set(original))
                    for name, data in original.items():
                        if name not in ('word/document.xml', 'word/styles.xml', 'word/settings.xml'):
                            self.assertEqual(archive.read(name), data)

    def test_original_and_outer_axis_boundary_pass(self):
        self.assertEqual(self.check(self.word), 2)
        self.assertEqual(self.check(self.pdf('boundary',
            [(x + .375, y, r + .375, b) for x, y, r, b in self.boxes])), 2)

    def test_every_edge_over_tolerance_fails(self):
        for axis in range(4):
            boxes = [list(box) for box in self.boxes]
            boxes[0][axis] += .376
            with self.subTest(axis=axis), self.assertRaises(ValueError):
                self.check(self.pdf('edge' + str(axis), boxes))

    def test_small_interior_gap_and_overlap_fail(self):
        for delta in (-.02, .02):
            boxes = [list(box) for box in self.boxes]
            boxes[1][1] += delta
            with self.subTest(delta=delta), self.assertRaisesRegex(ValueError, 'continuity'):
                self.check(self.pdf('seam' + str(delta), boxes))

    def test_missing_extra_page_color_translucency_and_erasure_fail(self):
        variants = [self.pdf('missing', self.boxes[:1]),
            self.pdf('extra', self.boxes + [(80, 126, 300, 144)]),
            self.pdf('page', self.boxes, extra_page=True),
            self.pdf('color', self.boxes, color=(1, 0, 0)),
            self.pdf('alpha', self.boxes, opacity=.5),
            self.pdf('erasure', self.boxes, erase=True)]
        for actual in variants:
            with self.subTest(actual=actual.name), self.assertRaises(ValueError):
                self.check(actual)


if __name__ == '__main__':
    unittest.main()
