"""Reject interchanged output references, missing evidence and absolute XY drift."""
import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import check_windows_output_contexts as check


class WindowsOutputContexts(unittest.TestCase):
    def test_all_originals_keep_independent_absolute_screen_and_print_references(self):
        cases = check.verify()
        self.assertEqual(len(cases), 18)
        for case in cases:
            with self.subTest(case=case['name']), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                _, pdf = check.original(case, directory)
                screen, = check.extract(directory / 'capture.json')['fixtures']
                printed = check.pdf_origins(pdf)
                for label, reference in [('screen', screen), ('print', printed)]:
                    self.assertGreater(check.compare(reference, reference, label), 0)
                    for axis in (2, 3):
                        shifted = copy.deepcopy(reference)
                        for glyph in shifted['glyphs']:
                            glyph[axis] += 2
                        with self.assertRaisesRegex(ValueError, 'axis mismatch'):
                            check.compare(reference, shifted, f'{label}/{axis}')

    def test_original_word_output_differences_are_not_renderer_regressions(self):
        case = next(c for c in check.verify()
                    if c['name'] == 'word-windows-title-calibri22-sides-spacing10-compat15')
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            _, pdf = check.original(case, directory)
            screen, = check.extract(directory / 'capture.json')['fixtures']
            printed = check.pdf_origins(pdf)
            for expected, actual in [(screen, printed), (printed, screen)]:
                with self.assertRaisesRegex(ValueError, 'axis mismatch'):
                    check.compare(expected, actual, 'wrong Word output context')

    def reject_registry_edit(self, edit, message):
        read = check.read_json
        index = copy.deepcopy(read(check.ROOT / check.INDEX))
        edit(index['fixtures'])
        with patch.object(check, 'read_json', side_effect=lambda path:
                          index if path == check.ROOT / check.INDEX else read(path)):
            with self.assertRaisesRegex(ValueError, message):
                check.verify()

    def test_output_contexts_cannot_be_interchanged(self):
        self.reject_registry_edit(lambda rows: rows[0].update(screen_reference='PDF'),
                                  'contexts were interchanged')

    def test_coverage_cannot_be_removed(self):
        self.reject_registry_edit(lambda rows: rows.pop(), 'coverage removed')

    def test_duplicate_cases_are_rejected(self):
        self.reject_registry_edit(lambda rows: rows.__setitem__(1, copy.deepcopy(rows[0])),
                                  'Duplicate output-context')

    def test_established_word_version_cannot_be_relabelled(self):
        self.reject_registry_edit(lambda rows: rows[0].update(word_product_version='0.0.0.0'),
                                  'Word version was removed')

    def test_original_capture_checks_word_version_executable_and_source_identity(self):
        case = check.verify()[0]
        for field, replacement, message in [
            ('word_product_version', '0.0.0.0', 'Word output version changed'),
            ('word_executable_sha256', '0' * 64, 'Word executable changed'),
            ('source', dict(file=case['source']['file'], sha256='0' * 64),
             'Wrong output source digest'),
        ]:
            changed = copy.deepcopy(case)
            changed[field] = replacement
            with self.subTest(field=field), tempfile.TemporaryDirectory() as temporary:
                with self.assertRaisesRegex(ValueError, message):
                    check.original(changed, Path(temporary))


if __name__ == '__main__':
    unittest.main()
