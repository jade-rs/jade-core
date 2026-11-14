"""Regression checks for platform selection, immutable inputs and strict XY gates."""
import copy
import gzip
import json
import unittest
from unittest.mock import patch

import check_platform_coordinates as check


class PlatformCoordinates(unittest.TestCase):
    def test_registered_originals_and_both_platform_references(self):
        self.assertGreaterEqual(len(check.verify()), 1)

    def test_original_word_platform_differences_are_not_regressions(self):
        # This identical DOCX has different origins in the two actual Word
        # exports. A shared golden would incorrectly fail one platform.
        name, _, _, _, windows, _ = next(
            case for case in check.verify() if case[0] == 'endnote-mac')
        directory = check.ROOT / 'tests/word-oracle/mac-elsa-synthetic' / name
        pages = json.loads(gzip.decompress((directory / 'pages.json.gz').read_bytes()))
        mac = dict(page_count=len(pages), glyphs=[
            [number, *glyph]
            for number, page in enumerate(pages, 1)
            for story in ('header', 'body', 'footer')
            for glyph in page[story]])
        self.assertEqual(check.compare(mac, mac, 'Mac original'), 12)
        self.assertEqual(check.compare(windows, windows, 'Windows original'), 12)
        for expected, actual in ((mac, windows), (windows, mac)):
            with self.subTest(expected=expected['glyphs'][0]), self.assertRaisesRegex(
                    ValueError, 'axis mismatch'):
                check.compare(expected, actual, 'wrong platform golden')

    def reject_index_edit(self, edit, message):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        edit(index['fixtures'][0])
        with patch.object(check, 'read_json', side_effect=lambda path:
                          index if path == check.ROOT / check.INDEX else original(path)):
            with self.assertRaisesRegex(ValueError, message):
                check.verify()

    def test_all_shared_originals_pass_independently_and_reject_xy_drift(self):
        # Exercise every imported pair, not just the example whose OS origins
        # differ. The native/Browser CI jobs render these same source bytes.
        for name, _, _, _, windows, _ in check.verify():
            directory = check.ROOT / 'tests/word-oracle/mac-elsa-synthetic' / name
            pages = json.loads(gzip.decompress((directory / 'pages.json.gz').read_bytes()))
            mac = dict(page_count=len(pages), glyphs=[
                [number, *glyph]
                for number, page in enumerate(pages, 1)
                for story in ('header', 'body', 'footer')
                for glyph in page[story]])
            for platform, reference in (('mac', mac), ('windows', windows)):
                with self.subTest(case=name, platform=platform):
                    self.assertGreater(check.compare(reference, reference, platform), 0)
                    for axis in (2, 3):
                        shifted = copy.deepcopy(reference)
                        for glyph in shifted['glyphs']:
                            glyph[axis] += 2
                        with self.assertRaisesRegex(ValueError, 'axis mismatch'):
                            check.compare(reference, shifted, f'{name}/{platform}/{axis}')

    def test_windows_reference_cannot_replace_mac_reference(self):
        self.reject_index_edit(lambda c: c.update(mac_manifest=c['windows'][0]['capture']),
                               'Wrong Mac manifest')

    def test_word_version_cannot_be_relabelled(self):
        self.reject_index_edit(lambda c: c['windows'][0].update(word_product_version='0.0.0.0'),
                               'Windows Word version changed')

    def test_unknown_font_source_is_rejected(self):
        self.reject_index_edit(lambda c: c['windows'][0].update(font_source='expected-positions'),
                               'Unknown font source')

    def test_mac_source_cannot_be_swapped(self):
        self.reject_index_edit(lambda c: c['source'].update(sha256='0' * 64), 'Evidence changed')

    def test_windows_expectations_cannot_be_removed(self):
        self.reject_index_edit(lambda c: c.update(windows=[]), 'Missing Windows')

    def test_duplicate_platform_cases_are_rejected(self):
        self.reject_index_edit(lambda c: c['windows'].append(c['windows'][0]), 'Duplicate Word')

    def test_registered_case_cannot_be_removed(self):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        index['fixtures'].pop()
        with patch.object(check, 'read_json', side_effect=lambda path:
                          index if path == check.ROOT / check.INDEX else original(path)):
            with self.assertRaisesRegex(ValueError, 'Shared fixture coverage removed'):
                check.verify()

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

    def test_floating_text_near_a_superscript_keeps_strict_origin_matching(self):
        # Word's 28pt exact textbox and adjacent body superscript straddle
        # the old 0.25pt reading-line grouping boundary after PDF rounding.
        word = dict(page_count=1, glyphs=[[1, 'B', 352.27, 137.18],
                                         [1, '7', 75.14, 137.42]])
        jade = dict(page_count=1, glyphs=[[1, 'B', 352.15, 137.14],
                                         [1, '7', 75.18, 137.43]])
        self.assertEqual(check.compare(word, jade, 'adjacent origins'), 2)
        jade['glyphs'][0][3] = 137.18 + .376
        with self.assertRaisesRegex(ValueError, 'axis'):
            check.compare(word, jade, 'real displacement')

    def test_overlapping_repeated_characters_need_distinct_partners(self):
        word = dict(page_count=1, glyphs=[[1, 'A', x, 80] for x in (0, .4, .8)])
        jade = dict(page_count=1, glyphs=[[1, 'A', x, 80] for x in (.2, .6, -.2)])
        self.assertEqual(check.compare(word, jade, 'ambiguous nearest origin'), 3)
        # Every reference has a nearby candidate, but two references would
        # have to reuse the same actual glyph. An existence-only check fails.
        word['glyphs'] = [[1, 'A', x, 80] for x in (0, .2, 1)]
        jade['glyphs'] = [[1, 'A', x, 80] for x in (0, 1, 1)]
        with self.assertRaisesRegex(ValueError, 'distinct glyph'):
            check.compare(word, jade, 'duplicate cannot replace missing paint')


if __name__ == '__main__':
    unittest.main()
