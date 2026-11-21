import unittest

import check_windows_font_spacing as check


class WindowsFontSpacingTests(unittest.TestCase):
    def test_all_original_contexts_and_sources_are_present(self):
        cases = check.verify()
        self.assertEqual(len(cases), 256)
        self.assertEqual(len({case['source']['sha256'] for case, _ in cases}), 256)

    def test_original_balance_cannot_be_relabelled(self):
        case, paths = check.verify()[0]
        with self.assertRaisesRegex(ValueError, 'authored balance changed'):
            check.verify_context(paths['source'], dict(case, balanced=not case['balanced']))

    def test_manual_break_cannot_be_relabelled_as_paragraph_end(self):
        case, paths = check.verify()[0]
        self.assertEqual(case['line_ending'], 'manual')
        with self.assertRaisesRegex(ValueError, 'authored break changed'):
            check.verify_context(paths['source'], dict(case, line_ending='paragraph'))


if __name__ == '__main__':
    unittest.main()
