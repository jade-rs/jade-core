import copy
import unittest

import check_windows_column_tables as check


class ColumnTableGate(unittest.TestCase):
    def test_all_versioned_column_references_are_required(self):
        self.assertEqual(len(check.verify()), 54)

    def test_distinct_screen_and_print_origins_pass_only_in_their_own_context(self):
        screen = dict(page_count=1, glyphs=[[1, 'A', 72., 80.]])
        printed = dict(page_count=1, glyphs=[[1, 'A', 162., 80.]])
        check.independent_outputs(screen, printed, printed, screen, screen, printed, printed, 'own contexts')
        for wrong_screen, wrong_print in ((printed, printed), (screen, screen)):
            with self.assertRaisesRegex(ValueError, 'axis mismatch'):
                check.independent_outputs(screen, printed, printed, wrong_screen, wrong_screen,
                                          wrong_print, wrong_print, 'wrong context')

    def test_print_and_screen_each_reject_per_axis_drift(self):
        original = dict(page_count=1, glyphs=[[1, 'A', 72., 80.]])
        for axis in (2, 3):
            changed = copy.deepcopy(original)
            changed['glyphs'][0][axis] += .376
            for screen, printed in ((changed, original), (original, changed)):
                with self.subTest(axis=axis), self.assertRaisesRegex(ValueError, 'axis mismatch'):
                    check.independent_outputs(original, original, original, screen, screen,
                                              printed, printed, 'displaced')

    def test_empty_or_displaced_vector_rules_fail(self):
        page = dict(page=1, word_rectangles=4, native_rectangles=4,
                    word_uncovered_count=0, native_uncovered_count=0)
        check.require_nonempty_matching_rules([page], 'correct')
        for field in ('word_rectangles', 'native_rectangles', 'word_uncovered_count', 'native_uncovered_count'):
            changed = dict(page)
            changed[field] = 0 if 'rectangles' in field else 1
            with self.subTest(field=field), self.assertRaises(ValueError):
                check.require_nonempty_matching_rules([changed], 'missing/displaced')
        with self.assertRaises(ValueError):
            check.require_nonempty_matching_rules([], 'missing pages')


if __name__ == '__main__':
    unittest.main()
