import copy
import unittest
from unittest.mock import patch

import check_windows_script_metrics as check


class WindowsScriptMetricsTests(unittest.TestCase):
    def test_changed_move_cannot_be_relabelled_as_word_measurement(self):
        original = check.read_json
        reference = copy.deepcopy(original(check.ROOT / check.DIRECTORY / 'script-metrics.json'))
        reference['facts'][0]['move_pt'] += .5
        with patch.object(check, 'read_json', side_effect=lambda path:
                          reference if path.name == 'script-metrics.json' else original(path)):
            with self.assertRaisesRegex(ValueError, 'differ from original Word'):
                check.verify()
