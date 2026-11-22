import copy
import hashlib
import unittest
from unittest.mock import patch
import check_mac_captured_source as check


class CapturedMacOriginalTests(unittest.TestCase):
    def test_shared_mac_originals_keep_independent_sources_and_coverage(self):
        rows = check.shared_originals()
        self.assertEqual({r[0] for r in rows}, set(check.SHARED_ORIGINALS))
        self.assertEqual(len(rows), 98)
        self.assertEqual(sum(r[5] for r in rows), 31353)
        self.assertEqual([r[4]['page_count'] for r in rows], [4, 4, 2, 2, 3, 1, 8, 8, 8, 9, 9, 9, 10, 10, 10, 6, 6, 7, 12, 12, 12, 3, 3, 3, 3, 2, 2, 3, 8, 2, 1, 2, 2, 8, 1, 1, 1, 2, 2, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 2, 8, 2, 5, 1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 1, 2, 2, 3, 2, 2, 2, 2, 2])
        for name, case, source, manifest, expected, count in rows:
            with self.subTest(case=name):
                self.assertEqual(manifest['reference']['platform'], 'mac')
                self.assertEqual(case['windows'][0]['word_product_version'], '16.0.19127.20936')
                actual = dict(copy.deepcopy(expected), input_sha256=case['source']['sha256'])
                self.assertEqual(check.assert_layout(expected, actual, case['source']['sha256'], name), count)

    def test_missing_shared_original_cannot_be_silently_skipped(self):
        read = check.read_json
        path = check.ROOT/'tests/word-oracle/platform-coordinates.json'
        registry = read(path)
        for name in check.SHARED_ORIGINALS:
            changed = copy.deepcopy(registry)
            changed['fixtures'] = [r for r in changed['fixtures'] if r['mac_case'] != name]
            with self.subTest(case=name), patch.object(
                    check, 'read_json', side_effect=lambda p: changed if p == path else read(p)):
                with self.assertRaisesRegex(ValueError, 'missing or duplicate shared Mac original'):
                    check.shared_originals()

    def test_shared_mac_goldens_cannot_be_relabelled_to_accept_new_expectations(self):
        read = check.read_json
        path = check.ROOT/'tests/word-oracle/platform-coordinates.json'
        registry = copy.deepcopy(read(path))
        for row in registry['fixtures']:
            if row['mac_case'] in check.SHARED_ORIGINALS:
                row['mac_manifest']['sha256'] = '0' * 64
        with patch.object(check, 'read_json', side_effect=lambda p: registry if p == path else read(p)):
            with self.assertRaisesRegex(ValueError, 'source or manifest identity changed'):
                check.shared_originals()

    def test_shared_mac_originals_reject_motion_loss_repagination_and_substitution(self):
        for name, case, source, manifest, expected, count in check.shared_originals():
            good = dict(copy.deepcopy(expected), input_sha256=case['source']['sha256'])
            moved = copy.deepcopy(good)
            moved['glyphs'][0][2] += .376
            invalid = [moved, dict(good, glyphs=[]),
                       dict(good, page_count=good['page_count'] + 1),
                       dict(good, input_sha256=check.CONTROL_SHA256)]
            for actual in invalid:
                with self.subTest(case=name), self.assertRaises(ValueError):
                    check.assert_layout(expected, actual, case['source']['sha256'], name)

    def test_missing_shared_names_are_rejected_before_reading_original_files(self):
        registry = copy.deepcopy(check.read_json(
            check.ROOT/'tests/word-oracle/platform-coordinates.json'))
        missing = list(check.SHARED_ORIGINALS)[-1]
        registry['fixtures'] = [r for r in registry['fixtures'] if r['mac_case'] != missing]
        with patch.object(check, 'read_json', return_value=registry), patch.object(
                check, 'checked_file', side_effect=AssertionError('unexpected original read')):
            with self.assertRaisesRegex(ValueError, 'missing or duplicate shared Mac original'):
                check.shared_originals()

    def test_shared_original_rejects_a_different_captured_mac_input(self):
        read = check.read_json
        registry = read(check.ROOT/'tests/word-oracle/platform-coordinates.json')
        name = next(iter(check.SHARED_ORIGINALS))
        case = next(r for r in registry['fixtures'] if r['mac_case'] == name)
        target = check.checked_file(check.ROOT, case['mac_manifest'])
        changed = copy.deepcopy(read(target))
        changed['reference']['capturedDocxSha256'] = '0' * 64
        with patch.object(check, 'read_json', side_effect=lambda p: changed if p == target else read(p)):
            with self.assertRaisesRegex(ValueError, 'captured Mac input differs'):
                check.shared_originals()

    def test_normalized_control_cannot_be_promoted_to_a_shared_original(self):
        name = 'word-mac-probe-floating-order'
        prefix = 'tests/word-oracle/mac-elsa-synthetic/' + name + '/'
        source = check.ROOT/prefix/'source.docx'
        manifest_path = check.ROOT/prefix/'manifest.json'
        read = check.read_json
        manifest = read(manifest_path)
        source_sha = hashlib.sha256(source.read_bytes()).hexdigest()
        self.assertEqual(source_sha, check.CONTROL_SHA256)
        manifest_sha = hashlib.sha256(manifest_path.read_bytes()).hexdigest()
        case = dict(mac_case=name,
                    source=dict(file=prefix+'source.docx', sha256=source_sha),
                    mac_manifest=dict(file=prefix+'manifest.json', sha256=manifest_sha),
                    windows=[dict(font_source='original-word-pdf')])
        registry_path = check.ROOT/'tests/word-oracle/platform-coordinates.json'
        spec = (source_sha, manifest_sha, manifest['coordinatesSha256'], 1, 1070)
        with patch.object(check, 'SHARED_ORIGINALS', {name: spec}), patch.object(
                check, 'read_json', side_effect=lambda p: dict(fixtures=[case])
                if p == registry_path else read(p)):
            with self.assertRaisesRegex(ValueError, 'normalized control must remain separate'):
                check.shared_originals()

    def test_exact_original_and_distinct_normalized_input_are_retained(self):
        original, control, raw, expected = check.originals()
        self.assertNotEqual(original.read_bytes(), control.read_bytes())
        self.assertEqual(raw['docx_sha256'], check.ORIGINAL_SHA256)
        self.assertEqual(raw['word_build'], '16.114.26101123')
        self.assertEqual(expected['page_count'], 1)
        self.assertEqual(sum(not g[1].isspace() for g in expected['glyphs']), 1070)

    def test_native_result_cannot_substitute_the_normalized_input(self):
        _, _, _, expected = check.originals()
        actual=dict(expected,input_sha256=check.CONTROL_SHA256)
        with self.assertRaisesRegex(ValueError,'wrong rendered input'):
            check.assert_layout(expected,actual,check.ORIGINAL_SHA256,'original')

    def test_original_coordinate_gate_rejects_motion_loss_and_repagination(self):
        expected=dict(page_count=1,glyphs=[[1,'A',50.,80.]])
        good=dict(copy.deepcopy(expected),input_sha256=check.ORIGINAL_SHA256)
        self.assertEqual(check.assert_layout(expected,good,check.ORIGINAL_SHA256,'same'),1)
        cases=[dict(good,page_count=2),dict(good,glyphs=[]),
               dict(good,glyphs=[[1,'A',50.376,80.]]),dict(good,glyphs=[[1,'A',50.,80.376]])]
        for actual in cases:
            with self.subTest(actual=actual),self.assertRaises(ValueError):
                check.assert_layout(expected,actual,check.ORIGINAL_SHA256,'drift')

    def test_actual_macos_requirement_cannot_silently_skip_on_another_host(self):
        with patch.object(check.sys,'platform','win32'):
            with self.assertRaisesRegex(ValueError,'requires macOS'):
                check.require_mac_host(True)
            check.require_mac_host(False)
