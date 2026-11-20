from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile
import xml.etree.ElementTree as ET

import check_windows_mincho_sizes as check


class WindowsMinchoSizesTests(unittest.TestCase):
    def test_original_word_matrix_has_all_96_distinct_sources(self):
        cases = check.verify()
        self.assertEqual(len(cases), 96)
        self.assertEqual(len({c['source']['sha256'] for c, _ in cases}), 96)

    def test_authored_size_must_match_the_index(self):
        case, paths = check.verify()[0]
        with tempfile.TemporaryDirectory() as directory:
            changed = Path(directory) / 'changed.docx'
            with ZipFile(paths['source']) as original, ZipFile(changed, 'w') as target:
                document = ET.fromstring(original.read('word/document.xml'))
                size = next(document.iter(check.W + 'sz'))
                size.set(check.W + 'val', str(case['font_size_pt'] * 2 + 1))
                for info in original.infolist():
                    target.writestr(info, ET.tostring(document) if info.filename == 'word/document.xml'
                                    else original.read(info.filename))
            with self.assertRaisesRegex(ValueError, 'authored size changed'):
                check.verify_source(changed, case)

    def test_coordinate_failures_are_required_and_cannot_be_pending(self):
        observed = dict(planes={'EMF': 'PASS', 'painted-PDF': 'coordinate drift'},
                        paragraph_background='PASS')
        with self.assertRaisesRegex(ValueError, 'original Word coordinate regression'):
            check.require_promoted_word(dict(name='size-control', required_word_coordinates=True),
                                        observed)


if __name__ == '__main__':
    unittest.main()
