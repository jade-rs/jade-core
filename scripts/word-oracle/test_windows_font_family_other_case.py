import unittest

import check_windows_font_family_other_case as check


class OtherFontFamilyCaseTests(unittest.TestCase):
    def test_all_four_families_retain_actual_word_case_coordinates(self):
        self.assertEqual(len(check.verify()), 96)

    def test_wrong_canonical_face_cannot_be_accepted_as_a_case_variant(self):
        case, alias, canonical = check.verify()[0]
        self.assertEqual(case['canonical_family'], 'Arial')
        with self.assertRaisesRegex(ValueError, 'requested font changed'):
            check.verify_source_pair(alias['source'], canonical['source'], case['family'], 'Calibri')


if __name__ == '__main__':
    unittest.main()
