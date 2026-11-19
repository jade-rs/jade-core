"""Keep original Word identity, isolated inputs and boundary assertions intact."""
import copy
import itertools
import unittest
from zipfile import ZipFile

from lxml import etree as E
import check_windows_word_boundaries as check
import generate_windows_word_boundary_probes as producer
from check_platform_coordinates import ROOT, checked_file, compare, pdf_origins, read_json


class WordBoundaries(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.index = read_json(ROOT / check.INDEX)

    def test_all16_sources_reproduce_original_bytes_and_full_matrix(self):
        cases, pending, xps = check.verify()
        generated = {n: (data, spec) for n, data, spec in producer.generate()}
        self.assertEqual(len(cases), 16)
        self.assertEqual(len(pending), 12)
        self.assertEqual(set(xps), set(generated))
        coverage = set()
        for row, source, *_ in cases:
            data, spec = generated[row['name']]
            self.assertEqual(source.read_bytes(), data)
            self.assertEqual(row['input_specification'], spec)
            coverage.add((spec['style'], spec['font_size_pt'], spec['compatibility_mode']))
        self.assertEqual(coverage, set(itertools.product(
            ('bold', 'bolditalic', 'italic', 'regular'), (14, 15), (14, 15))))

    def test_every_paragraph_retains_parent_formatting_and_other_package_parts(self):
        q = producer.q
        for row in self.index['fixtures']:
            spec = row['input_specification']
            with self.subTest(name=row['name']), \
                    ZipFile(checked_file(ROOT, spec['derived_parent'])) as old, \
                    ZipFile(checked_file(ROOT, row['source'])) as new:
                self.assertEqual(old.namelist(), new.namelist())
                for name in old.namelist():
                    if name != 'word/document.xml':
                        self.assertEqual(old.read(name), new.read(name))
                before, after = [E.fromstring(z.read('word/document.xml')) for z in (old, new)]
                ob, = before.iter(q('body')); nb, = after.iter(q('body'))
                originals = ob.findall(q('p')); paragraphs = nb.findall(q('p'))
                self.assertEqual(len(paragraphs), 24)
                for i, paragraph in enumerate(paragraphs):
                    template = originals[0 if i < 12 else 2]
                    text, = paragraph.iter(q('t')); original, = template.iter(q('t'))
                    self.assertEqual(text.text, producer.PATTERNS[i % 12][1])
                    self.assertEqual(text.get('{http://www.w3.org/XML/1998/namespace}space'), 'preserve')
                    text.text = original.text
                    text.attrib.clear(); text.attrib.update(original.attrib)
                    self.assertEqual(E.tostring(paragraph), E.tostring(template))
                self.assertEqual([c.tag for c in nb], [q('p')] * 24 + [q('sectPr')])
                a, = before.iter(q('pgSz')); b, = after.iter(q('pgSz'))
                self.assertEqual(b.get(q('w')), '17280')
                self.assertEqual(b.get(q('h')), a.get(q('h')))
                self.assertEqual(b.get(q('orient')), 'landscape')

    def test_required_pdf_and_all_screen_xps_assertions_cannot_be_removed_or_swapped(self):
        for field, promotion in [('coordinate_gate_promotion', 'word-boundary-print'),
                                 ('screen_gate_promotion', 'word-boundary-screen'),
                                 ('xps_gate_promotion', 'word-boundary-xps')]:
            index = copy.deepcopy(self.index)
            row = next(r for r in index['fixtures'] if r[field] == promotion)
            row[field] = 'pending'
            with self.subTest(field=field), self.assertRaises(ValueError):
                check.locked_promotions(index)
        index = copy.deepcopy(self.index)
        a = next(r for r in index['fixtures'] if r['coordinate_gate_promotion'] == 'pending')
        b = next(r for r in index['fixtures'] if r['coordinate_gate_promotion'] != 'pending')
        a['coordinate_gate_promotion'], b['coordinate_gate_promotion'] = \
            b['coordinate_gate_promotion'], a['coordinate_gate_promotion']
        with self.assertRaises(ValueError):
            check.locked_promotions(index)

    def test_pending_pdf_still_rejects_missing_reordered_text_and_changed_pages(self):
        expected = dict(page_count=1, glyphs=[[1, 'r', 72, 85], [1, 'y', 77, 85]])
        shifted = dict(page_count=1, glyphs=[[1, 'r', 73, 85], [1, 'y', 78, 85]])
        check.assert_pending_pdf_text(expected, shifted, 'pending coordinates')
        for actual in (dict(page_count=2, glyphs=expected['glyphs']),
                       dict(page_count=1, glyphs=expected['glyphs'][:1]),
                       dict(page_count=1, glyphs=expected['glyphs'][::-1])):
            with self.subTest(actual=actual), self.assertRaises(ValueError):
                check.assert_pending_pdf_text(expected, actual, 'pending')

    def test_pending_pdf_rejects_nonfinite_origins_on_both_axes(self):
        expected = dict(page_count=1, glyphs=[[1, 'y', 72., 85.]])
        for axis in (2, 3):
            for value in (float('nan'), float('inf'), -float('inf')):
                actual = copy.deepcopy(expected)
                actual['glyphs'][0][axis] = value
                with self.subTest(axis=axis, value=value), self.assertRaisesRegex(
                        ValueError, 'nonfinite glyph origin'):
                    check.assert_pending_pdf_text(expected, actual, 'pending')

    def test_original_word_pdf_and_xps_are_distinct_output_contexts(self):
        cases, _, xps = check.verify()
        for case, _, pdf, *_ in cases:
            with self.subTest(name=case['name']), self.assertRaisesRegex(ValueError, 'axis mismatch'):
                compare(xps[case['name']], pdf_origins(pdf, True), 'Word original PDF/XPS')


if __name__ == '__main__':
    unittest.main()
