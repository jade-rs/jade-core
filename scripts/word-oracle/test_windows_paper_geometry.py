import math
import unittest

import check_windows_paper_geometry as check


class WindowsPaperGeometryTests(unittest.TestCase):
    def test_all_original_paper_boundary_controls_are_present(self):
        cases = check.verify()
        self.assertEqual(len(cases), 17)
        self.assertEqual(len({case['source']['sha256'] for case, _ in cases}), 17)

    def test_authored_page_dimensions_cannot_be_relabelled(self):
        case, paths = check.verify()[0]
        with self.assertRaisesRegex(ValueError, 'Authored paper dimensions changed'):
            check.verify_page(paths['source'], dict(case, page_width_twips=case['page_width_twips'] + 1))

    def test_original_word_paper_rejects_the_previous_logical_page_size(self):
        case, paths = check.verify()[0]
        expected = check.page_bounds(paths['pdf'])
        old = [[0.0, 0.0, case['page_width_twips'] / 20, case['page_height_twips'] / 20]]
        with self.assertRaisesRegex(ValueError, 'original Word PDF paper bounds differ'):
            check.require_bounds(expected, old, case['name'])

    def test_invalid_or_missing_paper_pages_are_rejected(self):
        expected = [[0.0, 0.0, 595.32, 841.92]]
        for actual in [[], [[0.0, 0.0, math.nan, 841.92]], [[0.0, 0.0, -1.0, 841.92]]]:
            with self.assertRaises(ValueError):
                check.require_bounds(expected, actual, 'paper')


if __name__ == '__main__':
    unittest.main()
