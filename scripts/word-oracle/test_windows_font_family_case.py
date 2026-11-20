import unittest
from copy import deepcopy

import check_windows_font_family_case as check


class FontFamilyCaseTests(unittest.TestCase):
    def test_all_originals_have_unchanged_word_case_controls(self):
        self.assertEqual(len(check.verify()), 48)

    def test_vertical_drift_is_rejected(self):
        word = dict(page_count=1, glyphs=[[1, '@', 72.0 + i, 84.0] for i in range(240)])
        shifted = deepcopy(word)
        for glyph in shifted['glyphs']:
            glyph[3] += .5
        with self.assertRaisesRegex(ValueError, 'vertical origin'):
            check.assert_vertical_origins(word, shifted, 'moved')


if __name__ == '__main__':
    unittest.main()
