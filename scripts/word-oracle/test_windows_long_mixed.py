import copy
from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile
import xml.etree.ElementTree as ET

import check_windows_long_mixed as check


class WindowsMixedLongRunsTests(unittest.TestCase):
    def test_original_eight_cover_both_sizes_styles_and_text_patterns(self):
        cases = check.verify()
        self.assertEqual(len(cases), 8)
        self.assertEqual({(c['style'], c['font_size_pt'], c['pattern']) for c, _ in cases},
                         {(s, z, p) for s in ('regular', 'bold') for z in (17, 19)
                          for p in ('@m', '@@@@mmmm')})

    def test_pdf_holdouts_never_excuse_screen_or_xps_regressions(self):
        cases = check.verify()
        for case, _ in cases:
            for plane in check.SCREEN_PLANES:
                observed = dict(planes=dict.fromkeys(check.SCREEN_PLANES | check.PDF_PLANES, 'PASS'),
                                paragraph_background='PASS')
                observed['planes'][plane] = 'actual original coordinate mismatch'
                with self.subTest(case=case['name'], plane=plane), self.assertRaises(ValueError):
                    check.require_origins(case, observed)

    def test_seventeen_point_pdf_origins_remain_required(self):
        cases = check.verify()
        for case, _ in cases:
            observed = dict(planes=dict.fromkeys(check.SCREEN_PLANES | check.PDF_PLANES, 'PASS'),
                            paragraph_background='PASS')
            for plane in check.PDF_PLANES:
                altered = copy.deepcopy(observed)
                altered['planes'][plane] = 'actual original coordinate mismatch'
                with self.subTest(case=case['name'], plane=plane):
                    if case['font_size_pt'] == 17:
                        with self.assertRaises(ValueError):
                            check.require_origins(case, altered)
                    else:
                        self.assertEqual(set(check.require_origins(case, altered)), check.SCREEN_PLANES)

    def test_source_cannot_enable_kerning_or_add_shading_to_plain_controls(self):
        case, paths = check.verify()[0]
        with ZipFile(paths['source']) as package:
            members = {name: package.read(name) for name in package.namelist()}
        for change in ('kerning', 'shading'):
            tree = ET.fromstring(members['word/document.xml'])
            if change == 'kerning':
                tree.find('.//' + check.W + 'kern').set(check.W + 'val', '0')
            else:
                ET.SubElement(tree.find('.//' + check.W + 'rPr'), check.W + 'shd')
            with tempfile.TemporaryDirectory() as directory:
                source = Path(directory) / 'altered.docx'
                with ZipFile(source, 'w') as package:
                    for name, data in members.items():
                        package.writestr(name, ET.tostring(tree) if name == 'word/document.xml' else data)
                with self.subTest(change=change), self.assertRaises(ValueError):
                    check.verify_source(source, case)


if __name__ == '__main__':
    unittest.main()
