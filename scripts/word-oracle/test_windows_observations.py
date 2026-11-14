import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import check_windows_observations as check


class WindowsObservationsTests(unittest.TestCase):
    def test_removed_capture_rejected_before_reading_archives(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / check.INDEX).parent.mkdir(parents=True)
            (root / check.INDEX).write_bytes(b'{"schema":1,"captures":[]}')
            with self.assertRaisesRegex(ValueError, 'coverage changed'):
                check.verify(root)

    def test_changed_archive_is_rejected(self):
        original = check.read_json
        index = copy.deepcopy(original(check.ROOT / check.INDEX))
        index['captures'][0]['archive_sha256'] = '0' * 64
        with patch.object(check, 'read_json', return_value=index):
            with self.assertRaisesRegex(ValueError, 'Evidence changed'):
                check.verify()
