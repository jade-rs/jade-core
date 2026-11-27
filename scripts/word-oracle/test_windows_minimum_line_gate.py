"""Reject moved, missing, reordered and repaginated minimum-line glyphs."""
from copy import deepcopy
import unittest
import check_windows_minimum_lines as gate


class MinimumLineGate(unittest.TestCase):
    def test_original_108_cases_and6264_origins_are_required(self):
        cases=gate.verify()
        self.assertEqual(len(cases),108)
        self.assertEqual(sum(c[0]['glyph_count'] for c in cases),6264)
        self.assertTrue(all(c[0]['coordinate_gate_promotion']=='minimum-lines' for c in cases))

    def models(self):
        return [deepcopy(dict(page_count=1,glyphs=[[1,'A',72.,86.25],[1,'B',80.,86.25]])) for _ in range(7)]

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
