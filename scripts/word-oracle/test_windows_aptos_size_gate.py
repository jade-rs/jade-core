"""Reject moved, missing, reordered and repaginated Aptos-size glyphs."""
from copy import deepcopy
import unittest
import check_windows_aptos_sizes as gate


class AptosSizeGate(unittest.TestCase):
    def test_all64_screen_and49_pdf_cases_are_required(self):
        cases=gate.verify()
        self.assertEqual(len(cases),64)
        self.assertEqual(sum(c[0]['glyph_count'] for c in cases),17920)
        full=[c for c in cases if c[0]['name'] in gate.REQUIRED_CASE_NAMES]
        self.assertEqual(len(full),49)
        self.assertEqual(sum(c[0]['glyph_count'] for c in full),13720)
        self.assertTrue(all(c[0]['coordinate_gate_promotion']=='aptos-sizes' for c in full))
        self.assertTrue(all(c[0]['screen_gate_promotion']=='aptos-size-screen' for c in cases))

    def models(self):
        return [deepcopy(dict(page_count=1,glyphs=[[1,'A',72.,86.25],[1,'B',80.,86.25]])) for _ in range(7)]

    def test_both_screen_planes_reject_movement_missing_text_and_page_changes(self):
        for plane in (1,2):
            for mutation in ('x','y','missing','text','page'):
                with self.subTest(plane=plane,mutation=mutation):
                    models=self.models()[:3]
                    if mutation in ('x','y'):models[plane]['glyphs'][0][2 if mutation=='x' else 3]+=.5
                    elif mutation=='missing':models[plane]['glyphs'].pop()
                    elif mutation=='text':models[plane]['glyphs'][0][1]='Z'
                    else:models[plane]['page_count']=2
                    with self.assertRaises(ValueError):gate.assert_screen_outputs(*models,'invalid-screen')

    def test_all_independent_coordinate_planes_pass_without_adjustment(self):
        gate.assert_outputs(*self.models(),'same-original-origins')

    def test_each_native_plane_rejects_horizontal_and_vertical_displacement(self):
        for plane in range(3,7):
            for axis in (2,3):
                with self.subTest(plane=plane,axis=axis):
                    models=self.models();models[plane]['glyphs'][0][axis]+=.5
                    with self.assertRaisesRegex(ValueError,'axis mismatch'):
                        gate.assert_outputs(*models,'moved-origin')

    def test_each_native_plane_rejects_missing_reordered_text_and_page_changes(self):
        for plane in range(3,7):
            for mutation in ('missing','reordered','text','page'):
                with self.subTest(plane=plane,mutation=mutation):
                    models=self.models();model=models[plane]
                    if mutation=='missing':model['glyphs'].pop()
                    elif mutation=='reordered':
                        model['glyphs'][0][1],model['glyphs'][1][1]='B','A'
                    elif mutation=='text':model['glyphs'][0][1]='Z'
                    else:model['page_count']=2
                    with self.assertRaises(ValueError):gate.assert_outputs(*models,'invalid-origin-set')


if __name__=='__main__':unittest.main()
