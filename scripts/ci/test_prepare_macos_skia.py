import hashlib
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import prepare_macos_skia as skia


class PrepareMacosSkiaTests(unittest.TestCase):
    def test_verified_download_can_be_reused_without_network(self):
        data = b"verified fixture archive"
        digest = hashlib.sha256(data).hexdigest()
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "archive.tar.gz"
            with patch.object(skia.urllib.request, "urlopen", return_value=io.BytesIO(data)):
                self.assertEqual(skia.prepare_archive(target, digest), target.as_uri())
            with patch.object(skia.urllib.request, "urlopen", side_effect=AssertionError("network")):
                self.assertEqual(skia.prepare_archive(target, digest), target.as_uri())

    def test_bad_download_is_not_published_or_used(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "archive.tar.gz"
            with patch.object(skia.urllib.request, "urlopen", return_value=io.BytesIO(b"wrong")):
                with self.assertRaisesRegex(RuntimeError, "checksum mismatch"):
                    skia.prepare_archive(target)
            self.assertFalse(target.exists())

    def test_dependency_upgrade_requires_matching_archive(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for relative in skia.LOCKS:
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(
                    '[[package]]\nname="skia-bindings"\nversion="0.87.0"\n'
                    '[[package]]\nname="skia-safe"\nversion="0.87.0"\n', encoding="utf-8"
                )
            skia.verify_versions(root)
            path.write_text(path.read_text().replace("0.87.0", "0.88.0"), encoding="utf-8")
            with self.assertRaisesRegex(RuntimeError, "Update the pinned Skia archive"):
                skia.verify_versions(root)


if __name__ == "__main__":
    unittest.main()
