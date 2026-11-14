import copy
import unittest
from unittest.mock import patch

import check_windows_print_coordinates as check


class WindowsPrintCoordinatesTests(unittest.TestCase):
    def test_original_print_references_and_actual_emf_disagreements(self):
        self.assertEqual(len(check.verify()), 2)

    def test_emf_disagreement_cannot_be_relabelled(self):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        index['fixtures'][0]['emf_disagreement'] = 'invented mismatch'
        with patch.object(check, 'read_json', side_effect=lambda path:
                          index if path == check.ROOT / check.INDEX else original(path)):
            with self.assertRaisesRegex(ValueError, 'disagreement changed'):
                check.verify()

    def test_word_version_cannot_be_relabelled(self):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        index['fixtures'][0]['word_product_version'] = '0.0.0.0'
        with patch.object(check, 'read_json', side_effect=lambda path:
                          index if path == check.ROOT / check.INDEX else original(path)):
            with self.assertRaisesRegex(ValueError, 'Wrong print Word version'):
                check.verify()
