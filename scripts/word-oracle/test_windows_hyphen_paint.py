from copy import deepcopy
from unittest import mock
from pathlib import Path
import tempfile
import unittest

import pymupdf

import check_windows_hyphen_paint as check
from check_platform_coordinates import compare
import word_painted_control_origins as paint


class WindowsHyphenPaint(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.cases=check.verify()

    def test_all88_actual_originals_keep_every_visible_glyph(self):
        self.assertEqual(len(self.cases),88)
        self.assertEqual(len({row['source']['sha256'] for row,_,_,_ in self.cases}),88)
        aliases=0
        for row,_,pdf,expected in self.cases:
            with self.subTest(case=row['name']),pymupdf.open(pdf) as original:
                raw=[char for page in original for span in page.get_texttrace()
                     if span['type']!=3 and span.get('opacity',1)>0
                     for char,_,_,_ in span['chars'] if not chr(char).isspace()]
                self.assertEqual(len(raw),len(expected['glyphs']))
                self.assertEqual(compare(expected,expected,row['name']),len(raw))
                aliases+=len(expected['verified_painted_aliases'])
                for alias in expected['verified_painted_aliases']:
                    self.assertEqual(alias['source_char'],'\u00ad')
                    self.assertEqual(alias['canonical_char'],'-')
                    self.assertTrue(alias['equivalent_font_candidates'])
                    for font in alias['equivalent_font_candidates']:
                        self.assertRegex(font['program_sha256'],r'^[a-f0-9]{64}$')
                        self.assertEqual(font['glyph_name'],font['hyphen_name'])
                self.assertEqual(row['coordinate_gate_promotion'],'pending')
        self.assertGreater(aliases,0,'Original visible U+00AD font proofs must stay covered')

    def test_both_axes_counts_text_pages_and_duplicate_ink_are_rejected_for_every_case(self):
        row,_,word_pdf,expected=self.cases[0]
        with tempfile.TemporaryDirectory() as temporary:
            copied=Path(temporary)/'copied-reference.pdf'
            copied.write_bytes(word_pdf.read_bytes())
            # Coordinate equality alone would accept this. The renderer must
            # produce its own output rather than copy the original reference.
            compare(expected,paint.painted_origins(copied),row['name'])
            with self.assertRaisesRegex(ValueError,'Word PDF self-comparison'):
                check.require_independent_paint(word_pdf,copied,row['name'])
        for row,_,_,expected in self.cases:
            for defect in ('x','y','missing','duplicate','text','page'):
                with self.subTest(case=row['name'],defect=defect):
                    actual=deepcopy(expected)
                    if defect in ('x','y'):
                        actual['glyphs'][0][2 if defect=='x' else 3]+=.5
                    elif defect=='missing':actual['glyphs'].pop()
                    elif defect=='duplicate':actual['glyphs'].append(actual['glyphs'][0][:])
                    elif defect=='text':actual['glyphs'][0][1]='?'
                    else:actual['page_count']+=1
                    with self.assertRaises(ValueError):compare(expected,actual,row['name'])

    def test_unproven_font_or_wrong_actual_glyph_cannot_be_aliased_to_a_hyphen(self):
        _,_,pdf,_=next(case for case in self.cases if case[3]['verified_painted_aliases'])
        for defect in ('font','glyph'):
            with self.subTest(defect=defect),pymupdf.open(pdf) as original:
                class Page:
                    def __init__(self,page):self.page=page
                    def get_fonts(self,**kwargs):return self.page.get_fonts(**kwargs)
                    def get_texttrace(self):
                        spans=deepcopy(self.page.get_texttrace())
                        for span in spans:
                            if any(char==173 for char,_,_,_ in span['chars']):
                                if defect=='font':span['font']='UnidentifiedFont'
                                else:
                                    span['chars']=[(char,0 if char==173 else gid,origin,bounds)
                                                   for char,gid,origin,bounds in span['chars']]
                        return spans
                class Document:
                    def __enter__(self):return self
                    def __exit__(self,*args):return False
                    def __len__(self):return len(original)
                    def __iter__(self):return (Page(page) for page in original)
                    def extract_font(self,xref):return original.extract_font(xref)
                with mock.patch.object(paint.pymupdf,'open',return_value=Document()):
                    with self.assertRaises(ValueError):paint.painted_origins(pdf)


if __name__=='__main__':
    unittest.main()
