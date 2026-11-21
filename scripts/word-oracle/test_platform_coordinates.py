"""Regression checks for platform selection, immutable inputs and strict XY gates."""
import copy
import gzip
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from zipfile import ZipFile

import check_platform_coordinates as check


class PlatformCoordinates(unittest.TestCase):
    def test_registered_originals_and_both_platform_references(self):
        cases = check.verify()
        self.assertGreaterEqual(len(cases), 103)
        self.assertGreaterEqual(sum(case[2].suffix == '.zip' for case in cases), 82)

    def test_legacy_floating_text_edges_keep_original_platform_exports(self):
        name = 'word-mac-probe-floating-horizontal-14'
        self.assertNotIn(name, check.INDEPENDENT_OUTPUT_CASES)
        self.assertEqual(check.REQUIRED_CAPTURES[name], '20261114T004202539Z-7cf29738')
        _, source, archive, document, drawing, fonts = next(
            case for case in check.verify() if case[0] == name)
        self.assertEqual(fonts, 'original-word-pdf')
        original = check.read_json(check.ROOT / (
            'tests/word-oracle/mac-floating-horizontal-20261030/' + name + '.json'))
        directory = check.ROOT / 'tests/word-oracle/mac-elsa-synthetic' / name
        manifest = check.read_json(directory / 'manifest.json')
        self.assertEqual(document['sha256'], original['docx_sha256'])
        self.assertEqual(manifest['reference']['version'], original['word_version'])
        self.assertNotIn('build', manifest['reference'])  # Preserve the existing manifest.
        self.assertEqual(original['word_build'], '16.114.26102345')
        self.assertEqual(manifest['reference']['pdfSha256'], original['pdf_sha256'])
        pages = json.loads(gzip.decompress((directory / 'pages.json.gz').read_bytes()))
        expected = [dict(size=[page['width_pt'], page['height_pt']], header=[],
                         body=[[g['text'], g['x_pt'], g['baseline_pt']]
                               for row in page['rows'] for g in row['characters']], footer=[], images=[])
                    for page in original['pages']]
        self.assertEqual(pages, expected)
        with ZipFile(archive) as bundle, tempfile.TemporaryDirectory() as temporary:
            pdf = Path(temporary) / 'original-word.pdf'
            pdf.write_bytes(bundle.read(document['pdf']))
            check.compare(drawing, check.pdf_origins(pdf), name + '/original-EMF-PDF')
        self.assertTrue(source.is_file())

    def test_grid_line_units_preserve_original_mac_and_windows_exports(self):
        cases = {case[0]: case for case in check.verify()}
        for name in ('word-mac-probe-grid-line-unit',
                     'word-mac-probe-grid-line-unit-table'):
            with self.subTest(name=name):
                self.assertNotIn(name, check.INDEPENDENT_OUTPUT_CASES)
                self.assertEqual(check.REQUIRED_CAPTURES[name],
                                 '20261114T004214564Z-4ebc9049')
                _, source, archive, document, drawing, fonts = cases[name]
                self.assertEqual(fonts, 'original-word-pdf')
                original = check.read_json(check.ROOT / (
                    'tests/word-oracle/mac-probes-20261107/' + name + '.json'))
                directory = check.ROOT / 'tests/word-oracle/mac-elsa-synthetic' / name
                manifest = check.read_json(directory / 'manifest.json')
                self.assertEqual(document['sha256'], original['docx_sha256'])
                self.assertEqual(manifest['reference']['version'], original['word_version'])
                self.assertEqual(manifest['reference']['build'], original['word_build'])
                self.assertEqual(manifest['reference']['pdfSha256'], original['pdf_sha256'])
                pages = json.loads(gzip.decompress((directory / 'pages.json.gz').read_bytes()))
                expected = [dict(size=[page['width_pt'], page['height_pt']], header=[],
                                 body=[[glyph['text'], glyph['x_pt'], glyph['baseline_pt']]
                                       for row in page['rows'] for glyph in row['characters']],
                                 footer=[]) for page in original['pages']]
                self.assertEqual(pages, expected)  # No rounded or native-derived golden.
                self.assertEqual(len(pages), original['page_count'])
                with ZipFile(archive) as bundle, tempfile.TemporaryDirectory() as temporary:
                    pdf = Path(temporary) / 'original-word.pdf'
                    pdf.write_bytes(bundle.read(document['pdf']))
                    check.compare(drawing, check.pdf_origins(pdf), name + '/original-EMF-PDF')
                self.assertTrue(source.is_file())

    def test_grid_line_unit_version_expectations_cannot_be_removed(self):
        original = check.read_json
        for name in ('word-mac-probe-grid-line-unit',
                     'word-mac-probe-grid-line-unit-table'):
            index = copy.deepcopy(original(check.ROOT / check.INDEX))
            index['fixtures'] = [case for case in index['fixtures'] if case['mac_case'] != name]
            with self.subTest(name=name), patch.object(
                    check, 'read_json', side_effect=lambda path:
                    index if path == check.ROOT / check.INDEX else original(path)):
                with self.assertRaisesRegex(ValueError, 'Shared fixture coverage removed'):
                    check.verify()

    def test_windows_kerned_punctuation_keeps_shared_four_plane_assertions(self):
        name = 'word-mac-probe-pairs-kerned'
        self.assertNotIn(name, check.INDEPENDENT_OUTPUT_CASES)
        self.assertEqual(check.REQUIRED_CAPTURES[name], '20261114T004214564Z-4ebc9049')
        cases = [case for case in check.verify() if case[0] == name]
        self.assertEqual(len(cases), 1)
        _, source, archive, document, drawing, font_source = cases[0]
        self.assertEqual(font_source, 'original-word-pdf')
        self.assertEqual(document['sha256'],
                         'f6bc744c18f1098e7945c231a42b87af37335461eab8a42b96eb5132d41a4562')
        self.assertEqual(len(drawing['glyphs']), 96)
        with ZipFile(archive) as bundle, tempfile.TemporaryDirectory() as temporary:
            pdf = Path(temporary) / 'original-word.pdf'
            pdf.write_bytes(bundle.read(document['pdf']))
            check.compare(drawing, check.pdf_origins(pdf), name + '/original-EMF-PDF')
        self.assertTrue(source.is_file())

    def test_windows_kerned_punctuation_reference_cannot_be_removed(self):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        index['fixtures'] = [case for case in index['fixtures']
                             if case['mac_case'] != 'word-mac-probe-pairs-kerned']
        with patch.object(check, 'read_json', side_effect=lambda path:
                          index if path == check.ROOT / check.INDEX else original(path)):
            with self.assertRaisesRegex(ValueError, 'Shared fixture coverage removed'):
                check.verify()

    def test_existing_four_plane_assertions_cannot_be_changed_to_independent_outputs(self):
        self.reject_index_edit(lambda c: c['windows'][0].update(
            output_context='independent-screen-print'), 'coordinate assertions changed')

    def test_original_independent_output_cases_cannot_lose_their_context(self):
        original = check.read_json
        for name in check.INDEPENDENT_OUTPUT_CASES:
            for value in (None, 'shared-screen-pdf', 'unknown'):
                index = copy.deepcopy(original(check.ROOT / check.INDEX))
                windows, = next(c for c in index['fixtures'] if c['mac_case'] == name)['windows']
                if value is None:
                    windows.pop('output_context')
                else:
                    windows['output_context'] = value
                with self.subTest(case=name, context=value), patch.object(
                        check, 'read_json', side_effect=lambda path:
                        index if path == check.ROOT / check.INDEX else original(path)):
                    with self.assertRaisesRegex(ValueError, 'coordinate assertions changed'):
                        check.verify()

    def assert_output_models(self, *, screen=None, snapshot=None, physical=None,
                             unclipped=None, changed_pixels=False, self_comparison=False):
        drawing = dict(page_count=1, glyphs=[[1, 'A', 72., 80.]])
        word_pdf = dict(page_count=1, glyphs=[[1, 'A', 72.8, 80.]])
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            before, after, word, painted = [directory / name for name in
                                           ('screen.png', 'snapshot.png', 'word.pdf', 'native.pdf')]
            before.write_bytes(b'screen-pixels')
            after.write_bytes(b'changed' if changed_pixels else b'screen-pixels')
            word.write_bytes(b'original-word-pdf')
            painted.write_bytes(b'original-word-pdf' if self_comparison else b'independent-native-pdf')
            def origins(path, all_glyphs=False):
                if path == word:
                    return word_pdf
                return unclipped if all_glyphs and unclipped is not None else (
                    physical if physical is not None else word_pdf)
            with patch.object(check, 'pdf_origins', side_effect=origins):
                check.assert_independent_outputs(
                    drawing, screen if screen is not None else drawing,
                    snapshot if snapshot is not None else drawing, before, after, word, painted, 'outputs')

    def test_genuine_screen_print_differences_pass_their_own_output_assertions(self):
        self.assert_output_models()
        with self.assertRaisesRegex(ValueError, 'axis mismatch'):
            check.compare(dict(page_count=1, glyphs=[[1, 'A', 72.8, 80.]]),
                          dict(page_count=1, glyphs=[[1, 'A', 72., 80.]]), 'wrong output golden')

    def test_independent_contexts_are_supported_by_the_original_word_outputs(self):
        index = check.read_json(check.ROOT / check.INDEX)
        for case in index['fixtures']:
            if case['mac_case'] not in check.INDEPENDENT_OUTPUT_CASES:
                continue
            windows, = case['windows']
            archive, _, document, drawing = check.archive_reference(check.ROOT, case, windows)
            with self.subTest(case=case['mac_case']), tempfile.TemporaryDirectory() as temporary:
                path = Path(temporary) / 'word.pdf'
                with ZipFile(archive) as bundle:
                    path.write_bytes(bundle.read(document['pdf']))
                original_pdf = check.pdf_origins(path)
                # Word itself produces distinct origins for these outputs.
                # No native renderer is used to manufacture either expectation.
                for expected, actual in ((drawing, original_pdf), (original_pdf, drawing)):
                    with self.assertRaisesRegex(ValueError, 'axis mismatch'):
                        check.compare(expected, actual, 'original Word screen/print difference')

    def test_screen_and_exported_snapshot_must_both_match_word_emf(self):
        shifted = dict(page_count=1, glyphs=[[1, 'A', 73., 80.]])
        for models in ({'screen': shifted}, {'snapshot': shifted}):
            with self.subTest(models=models), self.assertRaisesRegex(ValueError, 'axis mismatch'):
                self.assert_output_models(**models)
        # Export must retain exact live origins even within the Word tolerance.
        with self.assertRaisesRegex(ValueError, 'changed live logical origins'):
            self.assert_output_models(snapshot=dict(page_count=1, glyphs=[[1, 'A', 72.1, 80.]]))

    def test_vector_pdf_checks_each_axis_missing_glyphs_pages_and_unclipped_origins(self):
        bad = [dict(page_count=1, glyphs=[[1, 'A', 73.176, 80.]]),
               dict(page_count=1, glyphs=[[1, 'A', 72.8, 80.376]]),
               dict(page_count=1, glyphs=[]),
               dict(page_count=2, glyphs=[[1, 'A', 72.8, 80.]])]
        for model in bad:
            for plane in ('physical', 'unclipped'):
                with self.subTest(plane=plane, model=model), self.assertRaises(ValueError):
                    self.assert_output_models(**{plane: model})

    def test_independent_outputs_reject_changed_live_pixels_and_word_self_comparison(self):
        with self.assertRaisesRegex(ValueError, 'changed live screen pixels'):
            self.assert_output_models(changed_pixels=True)
        with self.assertRaisesRegex(ValueError, 'Word PDF self-comparison'):
            self.assert_output_models(self_comparison=True)

    def reject_archive_edit(self, edit, message):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        case = next(c for c in index['fixtures'] if 'archive' in c['windows'][0])
        edit(case['windows'][0])
        with patch.object(check, 'read_json', side_effect=lambda path:
                          index if path == check.ROOT / check.INDEX else original(path)):
            with self.assertRaisesRegex(ValueError, message):
                check.verify()

    def test_archive_bytes_and_original_metadata_are_required(self):
        self.reject_archive_edit(lambda w: w['archive'].update(sha256='0' * 64),
                                 'Evidence changed')
        self.reject_archive_edit(lambda w: w.update(capture_sha256='0' * 64),
                                 'Original archived capture metadata changed')

    def test_archived_word_version_executable_and_capture_cannot_be_relabelled(self):
        self.reject_archive_edit(lambda w: w.update(word_product_version='0.0.0.0'),
                                 'Windows Word version changed')
        self.reject_archive_edit(lambda w: w.update(word_executable_sha256='0' * 64),
                                 'Windows Word executable changed')
        self.reject_archive_edit(lambda w: w.update(capture_id='other-capture'),
                                 'Archived capture identity changed')

    def test_archived_input_and_original_font_programs_cannot_be_substituted(self):
        self.reject_archive_edit(lambda w: w.update(fixture='other-input'),
                                 'Missing or duplicate archived Windows input')
        self.reject_archive_edit(lambda w: w.update(font_source='expected-positions'),
                                 'Unknown archived font source')

    def test_archive_reference_cannot_silently_choose_a_second_origin(self):
        self.reject_archive_edit(lambda w: w.update(drawing_reference=w['archive']),
                                 'Ambiguous original Windows reference')

    def test_old_mac_version_gaps_remain_original_provenance(self):
        index = check.read_json(check.ROOT / check.INDEX)
        archived = [c for c in index['fixtures'] if 'archive' in c['windows'][0]]
        missing = []
        for case in archived:
            manifest = check.read_json(check.checked_file(check.ROOT, case['mac_manifest']))
            if 'version' not in manifest['reference']:
                missing.append(case['mac_case'])
        self.assertEqual(len(missing), 19)

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
