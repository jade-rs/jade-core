import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from zipfile import ZipFile
import xml.etree.ElementTree as ET

import check_windows_font_family_extended_case as check
from check_windows_font_family_cjk_case import W


class ExtendedFontFamilyCaseTests(unittest.TestCase):
    def test_all_families_styles_and_promoted_word_coordinates_are_present(self):
        cases = check.verify()
        self.assertEqual(len(cases), 384)
        self.assertEqual(len({c['canonical']['source']['sha256'] for c, _, _ in cases}), 128)
        self.assertEqual(sum(c['required_word_coordinates'] for c, _, _ in cases), 384)

    def test_word_assertion_policy_cannot_be_changed_in_the_index(self):
        index = check.read_json(check.ROOT / check.INDEX)
        index['fixtures'][0]['required_word_coordinates'] = False
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / check.INDEX
            target.parent.mkdir(parents=True)
            raw = json.dumps(index).encode()
            target.write_bytes(raw)
            with patch.object(check, 'INDEX_SHA256', hashlib.sha256(raw).hexdigest()):
                with self.assertRaisesRegex(ValueError, 'Absolute Word requirement changed'):
                    check.verify(root)

    def test_case_comparison_cannot_hide_a_changed_bold_property(self):
        case, alias, canonical = check.verify()[0]
        with tempfile.TemporaryDirectory() as directory:
            changed = Path(directory) / 'changed.docx'
            with ZipFile(alias['source']) as source, ZipFile(changed, 'w') as target:
                document = ET.fromstring(source.read('word/document.xml'))
                run = next(r for r in document.iter(W + 'r')
                           if r.find(W + 'rPr/' + W + 'rFonts') is not None)
                properties = run.find(W + 'rPr')
                bold = properties.find(W + 'b')
                if bold is None:
                    bold = ET.SubElement(properties, W + 'b')
                bold.set(W + 'val', '1')
                for info in source.infolist():
                    target.writestr(info, ET.tostring(document) if info.filename == 'word/document.xml'
                                    else source.read(info.filename))
            with self.assertRaisesRegex(ValueError, 'other source properties'):
                check.verify_source_pair(changed, canonical['source'], case['family'],
                                         case['canonical_family'])


if __name__ == '__main__':
    unittest.main()
