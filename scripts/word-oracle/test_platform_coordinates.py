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
    def test_five_output_controls_preserve_versioned_same_input_originals(self):
        from collections import Counter
        facts = [{'name': 'floating-inside-outside-mac', 'source_sha256': '7928b10c2849bac71a61b74b282cdc700afb1344e77dbc9f1134fdd6c7a79312', 'Mac_pdf_sha256': 'fb8f0d699f67a0d5bdb63ab5147d1daeafd273d54a7ef5d38cee6dfd4588e52e', 'capture_id': '20261114T004143028Z-b4cb9887', 'archive_sha256': 'd1cc6d692fca913877eca1615ee8dfedb9c2f51d83b3268a4c0e6f10c19e9913'}, {'name': 'header-footer-edge-cases-mac', 'source_sha256': 'd6c325b84a42e5df290543623f000e00eafdbdbe72322b2f56fd391ba7a37d69', 'Mac_pdf_sha256': '69c1531d3c0f13639328725bfeedccac891b0e2b8d92209bdcec1ac3fb1e35ce', 'capture_id': '20261114T004143028Z-b4cb9887', 'archive_sha256': 'd1cc6d692fca913877eca1615ee8dfedb9c2f51d83b3268a4c0e6f10c19e9913'}, {'name': 'textbox-body-properties-mac', 'source_sha256': 'a360a8668dc3d745f0d617be078ca16197ab6d58e7c1aee7687802d74e1182cc', 'Mac_pdf_sha256': '415f3a50685e702ef1efa03cb5811ff10172fe9a7773d6e70ee61ead3031c288', 'capture_id': '20261114T004202539Z-7cf29738', 'archive_sha256': '186eafa1c44e8f2baab5244f7a26a802b4b0c51870b4736e62ec45b6661e6e78'}, {'name': 'textbox-tall-header-overlap-mac', 'source_sha256': '544e9bedd0958929959c11e46a520afe98f534d39e8466ac9d4f0c5ec05b3898', 'Mac_pdf_sha256': '5c83c9c02678792f3d19083a5da182fcefa8b2e8767e462415b4ffa6a52abc47', 'capture_id': '20261114T004202539Z-7cf29738', 'archive_sha256': '186eafa1c44e8f2baab5244f7a26a802b4b0c51870b4736e62ec45b6661e6e78'}, {'name': 'toc-instrtext-edge-cases-mac', 'source_sha256': '3dbe67594235f371babbe632095e927e2d961a975d4830291dc60db57d1a3fba', 'Mac_pdf_sha256': 'baefbb18951e0d57d8080cd5a91199881e807041b9707a4ad97d333554bae746', 'capture_id': '20261114T004202539Z-7cf29738', 'archive_sha256': '186eafa1c44e8f2baab5244f7a26a802b4b0c51870b4736e62ec45b6661e6e78'}]
        verified = {row[0]: row for row in check.verify()}
        for fact in facts:
            with self.subTest(name=fact['name']):
                name = fact['name']
                self.assertIn(name, check.INDEPENDENT_OUTPUT_CASES)
                self.assertEqual(check.REQUIRED_CAPTURES[name], fact['capture_id'])
                _, source, archive, document, drawing, fonts = verified[name]
                raw = check.read_json(check.ROOT / (
                    'tests/word-oracle/mac-glyph-corpus-20261019/'
                    + name.removesuffix('-mac') + '.json'))
                manifest = check.read_json(source.parent / 'manifest.json')
                self.assertEqual(raw['docx_sha256'], fact['source_sha256'])
                self.assertEqual(document['sha256'], fact['source_sha256'])
                self.assertEqual(raw['word_version'], '16.114')
                self.assertEqual(raw['word_build'], '16.114.26101123')
                self.assertEqual(manifest['reference']['version'], raw['word_version'])
                self.assertEqual(manifest['reference']['build'], raw['word_build'])
                self.assertEqual(raw['pdf_sha256'], fact['Mac_pdf_sha256'])
                self.assertEqual(manifest['reference']['pdfSha256'], raw['pdf_sha256'])
                pdf = check.checked_file(check.ROOT, dict(file=(
                    'tests/word-oracle/mac-glyph-corpus-20261019/pdf/'
                    + name.removesuffix('-mac') + '.pdf'), sha256=raw['pdf_sha256']))
                pages = json.loads(gzip.decompress((source.parent / 'pages.json.gz').read_bytes()))
                golden = [(i, *g) for i, page in enumerate(pages, 1)
                          for story in ('header', 'body', 'footer') for g in page[story]]
                observed = [(g[0], g[1], round(g[2], 3), round(g[3], 3))
                            for g in check.pdf_origins(pdf)['glyphs'] if not g[1].isspace()]
                self.assertEqual(Counter(golden), Counter(observed))
                self.assertTrue(golden)
                self.assertEqual(fonts, 'original-word-pdf')
                check.checked_file(check.ROOT, dict(file=archive.relative_to(check.ROOT).as_posix(),
                                                   sha256=fact['archive_sha256']))

    def test_each_new_output_control_is_required(self):
        names = ['floating-inside-outside-mac', 'header-footer-edge-cases-mac', 'textbox-body-properties-mac', 'textbox-tall-header-overlap-mac', 'toc-instrtext-edge-cases-mac']
        original = check.read_json
        for name in names:
            with self.subTest(name=name):
                index = copy.deepcopy(original(check.ROOT / check.INDEX))
                index['fixtures'] = [c for c in index['fixtures'] if c['mac_case'] != name]
                with patch.object(check, 'read_json', side_effect=lambda path:
                                  index if path == check.ROOT / check.INDEX else original(path)):
                    with self.assertRaisesRegex(ValueError, 'Shared fixture coverage removed'):
                        check.verify()

    def test_tight_polygon_preserves_same_input_versions_and_original_mac_pdf_baselines(self):
        from collections import Counter
        name = 'floating-tight-polygon-mac'
        self.assertIn(name, check.INDEPENDENT_OUTPUT_CASES)
        self.assertEqual(check.REQUIRED_CAPTURES[name], '20261114T004143028Z-b4cb9887')
        _, source, archive, document, drawing, fonts = next(
            row for row in check.verify() if row[0] == name)
        original = check.read_json(check.ROOT / (
            'tests/word-oracle/mac-glyph-corpus-20261019/floating-tight-polygon.json'))
        manifest = check.read_json(source.parent / 'manifest.json')
        self.assertEqual(original['docx_sha256'], document['sha256'])
        self.assertEqual(original['word_version'], '16.114')
        self.assertEqual(original['word_build'], '16.114.26101123')
        self.assertEqual(manifest['reference']['version'], original['word_version'])
        self.assertEqual(manifest['reference']['build'], original['word_build'])
        self.assertEqual(manifest['reference']['pdfSha256'], original['pdf_sha256'])
        pdf = check.checked_file(check.ROOT, dict(file=(
            'tests/word-oracle/mac-glyph-corpus-20261019/pdf/floating-tight-polygon.pdf'),
            sha256=original['pdf_sha256']))
        pages = json.loads(gzip.decompress((source.parent / 'pages.json.gz').read_bytes()))
        golden = [(i, *g) for i, page in enumerate(pages, 1)
                  for story in ('header', 'body', 'footer') for g in page[story]]
        # An old JSON row groups two baselines only0.24pt apart. The unchanged
        # Mac golden preserves the PDF's actual per-character origins.
        observed = [(g[0], g[1], round(g[2], 3), round(g[3], 3))
                    for g in check.pdf_origins(pdf)['glyphs'] if not g[1].isspace()]
        self.assertEqual(Counter(golden), Counter(observed))
        self.assertEqual(len(golden), 432)
        self.assertEqual(fonts, 'original-word-pdf')
        self.assertEqual(document['pages'], 1)
        self.assertTrue(archive.is_file())

    def test_tight_polygon_version_expectation_cannot_be_removed(self):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        index['fixtures'] = [case for case in index['fixtures']
                             if case['mac_case'] != 'floating-tight-polygon-mac']
        with patch.object(check, 'read_json', side_effect=lambda path:
                          index if path == check.ROOT / check.INDEX else original(path)):
            with self.assertRaisesRegex(ValueError, 'Shared fixture coverage removed'):
                check.verify()

    def test_float_lookahead_keeps_same_input_originals_and_all_four_planes(self):
        name = 'word-mac-probe-float-lookahead'
        self.assertNotIn(name, check.INDEPENDENT_OUTPUT_CASES)
        self.assertEqual(check.REQUIRED_CAPTURES[name], '20261114T004202539Z-7cf29738')
        _, source, archive, document, drawing, fonts = next(
            row for row in check.verify() if row[0] == name)
        self.assertEqual(fonts, 'original-word-pdf')
        original = check.read_json(check.ROOT / (
            'tests/word-oracle/mac-probes-20261019/' + name + '.json'))
        manifest = check.read_json(source.parent / 'manifest.json')
        self.assertEqual(document['sha256'], original['docx_sha256'])
        self.assertEqual(manifest['reference']['version'], original['word_version'])
        self.assertEqual(manifest['reference']['build'], original['word_build'])
        self.assertEqual(original['word_build'], '16.114.26101123')
        self.assertEqual(manifest['reference']['pdfSha256'], original['pdf_sha256'])
        pages = json.loads(gzip.decompress((source.parent / 'pages.json.gz').read_bytes()))
        expected = [dict(size=[page['width_pt'], page['height_pt']],
                         body=[[g['text'], g['x_pt'], row['baseline_pt']]
                               for row in page['rows'] for g in row['characters']])
                    for page in original['pages']]
        self.assertEqual(len(pages), 5)
        for page, reference in zip(pages, expected):
            self.assertEqual(page['size'], reference['size'])
            self.assertEqual(page['body'], reference['body'])
            self.assertFalse(page['header'] or page['footer'])
        self.assertEqual(document['pages'], 5)
        self.assertEqual(len(drawing['glyphs']), 1930)
        self.assertTrue(archive.is_file())

    def test_float_lookahead_version_expectation_cannot_be_removed(self):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        index['fixtures'] = [case for case in index['fixtures']
                             if case['mac_case'] != 'word-mac-probe-float-lookahead']
        with patch.object(check, 'read_json', side_effect=lambda path:
                          index if path == check.ROOT / check.INDEX else original(path)):
            with self.assertRaisesRegex(ValueError, 'Shared fixture coverage removed'):
                check.verify()

    def test_registered_originals_and_both_platform_references(self):
        cases = check.verify()
        self.assertGreaterEqual(len(cases), 111)
        self.assertGreaterEqual(sum(case[2].suffix == '.zip' for case in cases), 90)

    def test_section_start_spacing_preserves_versioned_original_outputs(self):
        name = 'header-footer-sectionpages-mac'
        self.assertIn(name, check.INDEPENDENT_OUTPUT_CASES)
        self.assertEqual(check.REQUIRED_CAPTURES[name], '20261114T004143028Z-b4cb9887')
        index = check.read_json(check.ROOT / check.INDEX)
        case = next(case for case in index['fixtures'] if case['mac_case'] == name)
        windows, = case['windows']
        self.assertEqual(windows['word_product_version'], '16.0.19127.20936')
        self.assertEqual(windows['word_executable_sha256'],
                         '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
        self.assertEqual(windows['output_context'], 'independent-screen-print')
        manifest = check.read_json(check.ROOT / case['mac_manifest']['file'])
        self.assertEqual(manifest['reference']['version'], '16.114')
        self.assertEqual(manifest['reference']['build'], '16.114.26101123')
        self.assertEqual(manifest['coordinatesSha256'],
                         'af42cc67cd23b5846fcbd791c8dfccdf8c7219e8cb6a8c25703e344311f7993f')
        self.assertEqual(case['source']['sha256'],
                         '9904e5c52526c3a8fd897b78747c865a06d17d656f3e7c5a12c891c3d2cbd65f')
        self.assertEqual(len([case for case in check.verify() if case[0] == name]), 1)

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
