"""Regression checks for platform selection, immutable inputs and strict XY gates."""
import copy
import unittest
from unittest.mock import patch

import check_platform_coordinates as check


class PlatformCoordinates(unittest.TestCase):
    def test_registered_originals_and_both_platform_references(self):
        self.assertGreaterEqual(len(check.verify()), 1)

    def reject_index_edit(self, edit, message):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        edit(index['fixtures'][0])
        with patch.object(check, 'read_json', side_effect=lambda path:
                          index if path == check.ROOT / check.INDEX else original(path)):
            with self.assertRaisesRegex(ValueError, message):
                check.verify()

    def test_windows_reference_cannot_replace_mac_reference(self):
        self.reject_index_edit(lambda c: c.update(mac_manifest=c['windows'][0]['capture']),
                               'Wrong Mac manifest')

    def test_word_version_cannot_be_relabelled(self):
        self.reject_index_edit(lambda c: c['windows'][0].update(word_product_version='0.0.0.0'),
                               'Windows Word version changed')

    def test_mac_source_cannot_be_swapped(self):
        self.reject_index_edit(lambda c: c['source'].update(sha256='0' * 64), 'Evidence changed')

    def test_windows_expectations_cannot_be_removed(self):
        self.reject_index_edit(lambda c: c.update(windows=[]), 'Missing Windows')

    def test_duplicate_platform_cases_are_rejected(self):
        self.reject_index_edit(lambda c: c['windows'].append(c['windows'][0]), 'Duplicate Word')

    def test_each_axis_is_checked_without_translation(self):
        reference = dict(page_count=1, glyphs=[[1, 'A', 72., 80.], [1, 'B', 80., 80.]])
        for axis in (2, 3):
            with self.subTest(axis=axis):
                actual = copy.deepcopy(reference)
                for glyph in actual['glyphs']:
                    glyph[axis] += .375
                self.assertEqual(check.compare(reference, actual, 'boundary'), 2)
                actual['glyphs'][0][axis] += .0001
                with self.assertRaisesRegex(ValueError, 'axis'):
                    check.compare(reference, actual, 'drift')

    def test_missing_text_and_page_changes_fail(self):
        reference = dict(page_count=1, glyphs=[[1, 'A', 72., 80.]])
        for actual in [dict(page_count=2, glyphs=reference['glyphs']),
                       dict(page_count=1, glyphs=[]),
                       dict(page_count=1, glyphs=[[1, 'B', 72., 80.]]),
                       dict(page_count=1, glyphs=[[1, 'A', float('nan'), 80.]])]:
            with self.subTest(actual=actual), self.assertRaises(ValueError):
                check.compare(reference, actual, 'negative')


if __name__ == '__main__':
    unittest.main()
