"""Require full Word screen/PDF coverage and reject altered original geometry."""
import copy
import unittest
from unittest.mock import patch
import check_windows_top_bottom_clearance as check


class TopBottomClearance(unittest.TestCase):
    def test_requires_every_original_float_and_segment_parameter_combination(self):
        cases = check.verify()
        self.assertEqual(len(cases), 28)
        floats = [r for r, _, _, _ in cases if 'font_size_pt' in r['input_specification']]
        self.assertEqual(len(floats), 24)
        combinations = {tuple(row['input_specification'][key] for key in (
            'font_size_pt', 'vertical_offset_pt', 'distance_bottom_pt', 'compatibility_mode'))
            for row in floats}
        expected = {(size, offset, bottom, mode)
                    for size in (11, 14) for offset in (26.25, 52.5, 78.75)
                    for bottom in (0, 7.5) for mode in (14, 15)}
        self.assertEqual(combinations, expected)
        segments = [r['input_specification'] for r, _, _, _ in cases
                    if 'paint_run_split_at' in r['input_specification']]
        self.assertEqual({(s['compatibility_mode'], s['paint_run_split_at']) for s in segments},
                         {(14, None), (15, None), (14, 100), (15, 100)})
        self.assertTrue(all(model['glyphs'] for _, _, _, model in cases))

    def test_original_coverage_and_both_coordinate_gates_cannot_be_removed(self):
        original = check.read_json
        for relative in (check.METADATA, check.SEGMENT_METADATA):
            for mutation, message in (
                    (lambda data: data['fixtures'].pop(), 'source coverage changed'),
                    (lambda data: data['fixtures'][0].update(coordinate_gate_promotion='pending'),
                     'Required screen or physical PDF gate removed'),
                    (lambda data: data['fixtures'][0].update(physical_pdf_gate='pending'),
                     'Required screen or physical PDF gate removed')):
                path = check.ROOT / relative
                data = copy.deepcopy(original(path))
                mutation(data)
                with self.subTest(dataset=relative, error=message), patch.object(
                        check, 'read_json', side_effect=lambda candidate:
                        data if candidate == path else original(candidate)):
                    with self.assertRaisesRegex(ValueError, message):
                        check.verify()

    def test_original_word_screen_and_print_origins_require_separate_contexts(self):
        row, _, _, printed = check.verify()[0]
        capture_path = check.checked_file(check.ROOT, row['capture'])
        screen = next(d for d in check.extract(capture_path)['fixtures']
                      if d['name'] == row['name'])
        self.assertEqual(row['output_context'], 'independent-screen-print')
        with self.assertRaises(ValueError):
            check.compare(screen, printed, 'original-Word-screen-versus-print')

    def test_original_coordinate_displacement_and_missing_ink_are_rejected(self):
        _, _, _, reference = check.verify()[0]
        moved = copy.deepcopy(reference)
        for glyph in moved['glyphs']:
            glyph[2] += 0.5
        with self.assertRaises(ValueError):
            check.compare(reference, moved, 'displaced-original')
        missing = copy.deepcopy(reference)
        missing['glyphs'].pop(0)
        with self.assertRaisesRegex(ValueError, 'missing/extra glyphs'):
            check.compare(reference, missing, 'missing-original-ink')


if __name__ == '__main__':
    unittest.main()
