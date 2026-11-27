import tempfile
import unittest
from pathlib import Path
from zipfile import ZipFile
import xml.etree.ElementTree as ET

import check_windows_font_family_cjk_case as check


class MixedFontFamilyCaseTests(unittest.TestCase):
    def test_original_word_case_coordinates_and_complete_coverage(self):
        self.assertEqual(len(check.verify()), 96)

    def test_source_geometry_change_cannot_be_accepted_as_font_case(self):
        case, alias, canonical = check.verify()[0]
        with tempfile.TemporaryDirectory() as temp:
            changed = Path(temp) / 'changed.docx'
            with ZipFile(alias['source']) as source, ZipFile(changed, 'w') as dest:
                for item in source.infolist():
                    data = source.read(item.filename)
                    if item.filename == 'word/document.xml':
                        tree = ET.fromstring(data)
                        indent = tree.find('.//' + check.W + 'ind')
                        self.assertIsNotNone(indent)
                        indent.set(check.W + 'right', '5404')
                        data = ET.tostring(tree)
                    dest.writestr(item, data)
            with self.assertRaisesRegex(ValueError, 'other source properties'):
                check.verify_source_pair(changed, canonical['source'], case['family'])


if __name__ == '__main__':
    unittest.main()
