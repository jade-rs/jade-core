import copy
import unittest
from unittest.mock import patch
import check_mac_header_coordinates as check


class HeaderOriginals(unittest.TestCase):
    def test_both_original_versions_keep_all_header_and_body_origins(self):
        rows = check.verify()
        self.assertEqual([r[5] for r in rows], [55, 58])
        self.assertEqual([r[4]['page_count'] for r in rows], [1, 1])
        for name, case, source, reference, expected, glyphs in rows:
            with self.subTest(name=name):
                self.assertEqual(reference['version'], '16.114')
                self.assertEqual(reference['build'], '16.114.26101123')
                self.assertEqual(case['windows'][0]['word_product_version'], '16.0.19127.20936')
                self.assertEqual(check.compare(expected, expected, name), glyphs)
                missing = dict(expected, glyphs=[])
                with self.assertRaisesRegex(ValueError, 'missing/extra glyphs'):
                    check.compare(expected, missing, name)

    def test_removing_either_header_original_is_rejected(self):
        read = check.read_json
        for name in check.REFERENCES:
            registry = copy.deepcopy(read(check.ROOT/check.INDEX))
            registry['fixtures'] = [c for c in registry['fixtures'] if c['mac_case'] != name]
            with self.subTest(name=name), patch.object(check, 'read_json', side_effect=lambda p:
                    registry if p == check.ROOT/check.INDEX else read(p)):
                with self.assertRaisesRegex(ValueError, 'missing or duplicate header original'):
                    check.verify()


if __name__ == '__main__':
    unittest.main()
