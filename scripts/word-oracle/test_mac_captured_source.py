import copy
import unittest
from unittest.mock import patch
import check_mac_captured_source as check


class CapturedMacOriginalTests(unittest.TestCase):
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
