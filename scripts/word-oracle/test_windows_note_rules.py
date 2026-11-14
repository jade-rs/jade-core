"""Reject absent/moved separator paint and unsupported original EMF geometry."""
import copy
import struct
import unittest

import check_windows_note_rules as check
from test_windows_emf import metafile, record


def rule_metafile(operation=0x00f00021, bitmap=0, color=0x02000000, prefix=()):
    paint = bytearray(100)
    struct.pack_into('<II', paint, 0, 76, 100)
    struct.pack_into('<iiiiI', paint, 24, 600, 700, 1200, 4, operation)
    struct.pack_into('<I', paint, 84, bitmap)
    added = [record(39, struct.pack('<IIII', 4, 0, color, 0)),
             record(37, struct.pack('<I', 4)), *prefix, bytes(paint)]
    original = metafile()
    data = bytearray(original[:-20] + b''.join(added) + original[-20:])
    struct.pack_into('<I', data, 48, len(data))
    struct.pack_into('<I', data, 52, 5 + len(added))
    return bytes(data)


class NoteRuleTests(unittest.TestCase):
    def assert_rule(self, actual, expected):
        self.assertEqual(len(actual), 1)
        self.assertEqual(actual[0][0], expected[0])
        for found, edge in zip(actual[0][1:], expected[1:]):
            self.assertAlmostEqual(found, edge, places=12)

    def test_versioned_originals_and_all_twenty_two_cases(self):
        self.assertEqual(len(check.verify()), 22)

    def test_patcopy_uses_logical_destination_edges(self):
        self.assert_rule(check.emf_rules(rule_metafile(), 1, 612, 792),
                         [1, 72., 84., 216., 84.48])

    def test_raster_operations_bitmaps_transforms_and_colored_brushes_fail(self):
        for data in (rule_metafile(operation=0x00cc0020), rule_metafile(bitmap=100),
                     rule_metafile(color=0xff),
                     rule_metafile(prefix=[record(10, struct.pack('<ii', 1, 0))]),
                     rule_metafile(prefix=[record(35, struct.pack('<6f', 1, 0, 0, 1, 0, 0))]),
                     rule_metafile(prefix=[record(37, struct.pack('<I', 0x80000003))])):
            with self.subTest(data=data[-124:]), self.assertRaises(ValueError):
                check.emf_rules(data, 1, 612, 792)

    def test_rule_clip_and_saved_dc_are_obeyed(self):
        prefix = [record(33, b''), record(30, struct.pack('<iiii', 900, 0, 1800, 6600)),
                  record(34, struct.pack('<i', -1))]
        self.assert_rule(check.emf_rules(rule_metafile(prefix=prefix), 1, 612, 792),
                         [1, 72., 84., 216., 84.48])
        self.assert_rule(check.emf_rules(rule_metafile(prefix=prefix[1:2]), 1, 612, 792),
                         [1, 108., 84., 216., 84.48])

    def test_each_edge_is_asserted_without_translation(self):
        word = [[1, 72., 84., 216., 84.48]]
        for axis in range(1, 5):
            actual = copy.deepcopy(word)
            actual[0][axis] += .375
            self.assertEqual(check.compare_rules(word, actual, 'boundary'), 1)
            actual[0][axis] += .0001
            with self.subTest(axis=axis), self.assertRaisesRegex(ValueError, 'rule axis'):
                check.compare_rules(word, actual, 'displacement')

    def test_missing_extra_wrong_page_and_nonfinite_paint_fail(self):
        word = [[1, 72., 84., 216., 84.48]]
        for actual in ([], word + word, [[2, *word[0][1:]]],
                       [[1, float('nan'), 84., 216., 84.48]]):
            with self.subTest(actual=actual), self.assertRaises(ValueError):
                check.compare_rules(word, actual, 'paint regression')
        with self.assertRaises(ValueError):
            check.compare_rules([], [], 'empty assertion')


if __name__ == '__main__':
    unittest.main()
