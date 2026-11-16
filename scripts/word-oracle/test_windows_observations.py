import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import zipfile
from unittest.mock import patch

import check_windows_observations as check


class WindowsObservationsTests(unittest.TestCase):
    def test_rejected_word_attempt_cannot_be_relabelled_as_success(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / 'word-16.0.1.2/capture-failed.zip'
            path.parent.mkdir()
            source, error = b'original input', b'original Word open failure'
            word = {'product_version': '16.0.1.2', 'executable_sha256': '0' * 64}
            digest = hashlib.sha256(source).hexdigest()
            for status, extra in [('failed', False), ('complete', False), ('failed', True)]:
                raw = json.dumps({'schema': 2, 'platform': 'windows', 'status': status,
                                  'capture_id': 'capture', 'word': word, 'documents': []}).encode()
                with zipfile.ZipFile(path, 'w') as bundle:
                    bundle.writestr('capture.json', raw)
                    bundle.writestr('inputs/source.docx', source)
                    bundle.writestr('capture-error.log', error)
                    if extra:
                        bundle.writestr('invented-coordinates.json', b'{}')
                attempt = dict(archive=path.relative_to(root).as_posix(),
                               archive_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                               capture_sha256=hashlib.sha256(raw).hexdigest(),
                               error_sha256=hashlib.sha256(error).hexdigest(),
                               source_sha256=digest, capture_id='capture', word=word,
                               open_mode='normal-read-only')
                if status == 'failed' and not extra:
                    check.verify_rejected_attempt(root, {'source_sha256': digest}, attempt)
                else:
                    with self.assertRaisesRegex(ValueError, 'drawing expectations|successful drawing'):
                        check.verify_rejected_attempt(root, {'source_sha256': digest}, attempt)

    def test_every_imported_mac_source_has_windows_evidence_or_explicit_refusal(self):
        index = check.read_json(check.ROOT / check.INDEX)
        self.assertEqual(check.verify_mac_input_coverage(index), (578, 1))
        for rejection in index['rejected_sources']:
            for attempt in rejection.get('attempts', []):
                check.verify_rejected_attempt(check.ROOT, rejection, attempt)

    def test_new_mac_input_cannot_silently_escape_windows_coverage(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            case = root / 'tests/word-oracle/mac-elsa-synthetic/new-case'
            case.mkdir(parents=True)
            data = b'unchanged source bytes'
            digest = hashlib.sha256(data).hexdigest()
            (case / 'source.docx').write_bytes(data)
            (case / 'manifest.json').write_text(json.dumps({'sourceSha256': digest}))
            index = {'captures': [], 'rejected_sources': []}
            with self.assertRaisesRegex(ValueError, 'Mac original missing Windows observation'):
                check.verify_mac_input_coverage(index, root)
            index['captures'] = [{'documents': [{'source_sha256': digest}]}]
            self.assertEqual(check.verify_mac_input_coverage(index, root), (1, 0))
            index['captures'] = []
            index['rejected_sources'] = [{'source_sha256': digest}]
            self.assertEqual(check.verify_mac_input_coverage(index, root), (0, 1))
            (case / 'source.docx').write_bytes(b'changed source')
            with self.assertRaisesRegex(ValueError, 'Imported Mac source identity changed'):
                check.verify_mac_input_coverage(index, root)

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
