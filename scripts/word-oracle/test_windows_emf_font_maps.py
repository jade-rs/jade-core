"""Original Thai EMF glyph indexes remain font-bound, versioned and asserted."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from zipfile import ZipFile
import check_platform_coordinates as check
import check_mac_captured_source as mac
from windows_emf_font_maps import original_pdf_glyph_index_resolver

CASES = {'probe-complex-font-metrics-thonburi': (3, 75),
         'probe-complex-font-sweep-thonburi': (6, 270)}


class OriginalGlyphIndexFonts(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        registry = check.read_json(check.ROOT/check.INDEX)
        cls.cases = [c for c in registry['fixtures'] if c['mac_case'] in CASES]
        if len(cls.cases) != 2:
            raise AssertionError('Two original Thai fixture gates required')

    def context(self, case):
        w = case['windows'][0]
        archive = check.checked_file(check.ROOT, w['archive'])
        with ZipFile(archive) as bundle:
            raw = bundle.read('capture.json')
            self.assertEqual(hashlib.sha256(raw).hexdigest(), w['capture_sha256'])
            capture = json.loads(raw.decode('utf-8-sig'))
            document, = [d for d in capture['documents'] if d['name'] == w['fixture']]
            data = bundle.read(document['pdf'])
        identity = check.read_json(check.checked_file(check.ROOT, w['glyph_index_fonts']))
        return data, identity, dict(source_sha256=case['source']['sha256'],
                                   capture_sha256=w['capture_sha256'], word=capture['word'])

    def test_actual_original_emf_matches_independent_word_pdf_on_all_nine_pages(self):
        pages = glyphs = 0
        for case in self.cases:
            with self.subTest(name=case['mac_case']):
                _, capture, document, drawing = check.archive_reference(check.ROOT, case, case['windows'][0])
                expected_pages, expected_glyphs = CASES[case['mac_case']]
                self.assertEqual(drawing['page_count'], expected_pages)
                self.assertEqual(len(drawing['glyphs']), expected_glyphs)
                self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
                data, _, _ = self.context(case)
                with tempfile.TemporaryDirectory() as directory:
                    pdf = Path(directory)/'original.pdf'
                    pdf.write_bytes(data)
                    self.assertEqual(check.compare(check.pdf_origins(pdf, True), drawing,
                                                  case['mac_case']+'/original-EMF-vs-PDF'), expected_glyphs)
                pages += expected_pages
                glyphs += expected_glyphs
        self.assertEqual((pages, glyphs), (9, 345))

    def test_provenance_face_style_cmap_outline_and_advance_tampering_fail(self):
        mutations = [
            lambda r:r.update(source_sha256='0'*64),
            lambda r:r.update(capture_sha256='0'*64),
            lambda r:r['word'].update(product_version='0.0.0.0'),
            lambda r:r.update(original_pdf_sha256='0'*64),
            lambda r:r['fonts'][0].update(original_pdf_font_sha256='0'*64),
            lambda r:r['fonts'][0].update(family='Arial'),
            lambda r:r['fonts'][0].update(weight=700),
            lambda r:r['fonts'][0].update(italic=True),
            lambda r:r['fonts'][0].update(glyph_count=1),
            lambda r:r['fonts'][0].update(font_revision=0),
            lambda r:r['fonts'][0]['glyphs'][0].update(outline_sha256='0'*64),
            lambda r:r['fonts'][0]['glyphs'][0].update(advance_and_bearing=[0,0]),
            lambda r:r['fonts'][0]['glyphs'][0].update(unicode_scalar=65),
            lambda r:r['fonts'][0]['glyphs'].append(copy.deepcopy(r['fonts'][0]['glyphs'][0])),
            lambda r:r['fonts'].append(copy.deepcopy(r['fonts'][0])),
            lambda r:r['fonts'][0].update(inspected_full_font_sha256=None),
        ]
        for case in self.cases:
            data, identity, kwargs = self.context(case)
            for i, mutate in enumerate(mutations):
                changed = copy.deepcopy(identity)
                mutate(changed)
                with self.subTest(name=case['mac_case'], mutation=i), self.assertRaises(ValueError):
                    original_pdf_glyph_index_resolver(data, changed, **kwargs)

    def test_malformed_binding_objects_and_unbound_indexes_fail(self):
        for case in self.cases:
            data, identity, kwargs = self.context(case)
            for changed in [None, dict(identity, fonts=[None]),
                            dict(identity, fonts=[dict(identity['fonts'][0], glyphs=[None])])]:
                with self.subTest(name=case['mac_case'], malformed=repr(changed)[:40]), self.assertRaises(ValueError):
                    original_pdf_glyph_index_resolver(data, changed, **kwargs)
            resolve = original_pdf_glyph_index_resolver(data, identity, **kwargs)
            for face, gid in [(('Arial',400,False),251), (('Leelawadee UI',700,False),251),
                              (('Leelawadee UI',400,False),0)]:
                with self.subTest(name=case['mac_case'], font=face, gid=gid), self.assertRaises(ValueError):
                    resolve(face, gid)

    def test_new_cases_cannot_drop_font_program_or_mac_geometry_gates(self):
        rows = mac.shared_originals(check.ROOT, mac.THAI_LAYOUT_ORIGINALS)
        self.assertEqual({r[0] for r in rows}, set(CASES))
        self.assertEqual(sum(r[-1] for r in rows), 345)
        for case in self.cases:
            name = case['mac_case']
            self.assertIn(name, check.REQUIRED_CAPTURES)
            self.assertIn(name, check.PHYSICAL_FONT_CASES)
            manifest = check.read_json(check.checked_file(check.ROOT, case['mac_manifest']))
            self.assertNotIn('version', manifest['reference'])
            changed = copy.deepcopy(case)
            changed['windows'][0].pop('glyph_index_fonts')
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'Unsupported text options'):
                check.archive_reference(check.ROOT, changed, changed['windows'][0])

    def test_new_mac_originals_reject_motion_loss_repagination_and_source_changes(self):
        for name, case, _, _, expected, count in mac.shared_originals(check.ROOT, mac.THAI_LAYOUT_ORIGINALS):
            good = dict(copy.deepcopy(expected), input_sha256=case['source']['sha256'])
            self.assertEqual(mac.assert_layout(expected, good, case['source']['sha256'], name), count)
            moved = copy.deepcopy(good)
            moved['glyphs'][0][2] += .376
            for actual in [moved, dict(good, glyphs=[]), dict(good, page_count=good['page_count']+1),
                           dict(good, input_sha256='0'*64)]:
                with self.subTest(name=name), self.assertRaises(ValueError):
                    mac.assert_layout(expected, actual, case['source']['sha256'], name)


if __name__ == '__main__':
    unittest.main()
