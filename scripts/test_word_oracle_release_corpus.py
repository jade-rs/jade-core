"""Keep the SDK's Word input-byte gate complete at the source repository."""

import hashlib
import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]


class WordOracleReleaseCorpusTests(unittest.TestCase):
    def test_every_docx_has_its_exact_input_bytes_pinned(self) -> None:
        manifest = json.loads(
            (ROOT / "tests/word-oracle/release-corpus.json").read_text()
        )
        self.assertEqual(manifest["schema_version"], 1)
        rows = manifest["fixtures"]
        pinned = {row["fixture"]: row["docx_sha256"] for row in rows}
        self.assertEqual(len(pinned), len(rows), "duplicate fixture names")
        observed = {
            path.stem: hashlib.sha256(path.read_bytes()).hexdigest()
            for path in (ROOT / "public/fixtures").glob("*.docx")
            if not path.name.startswith(("._", "~$"))
        }
        self.assertGreaterEqual(len(observed), 114, "fixture corpus shrank")
        self.assertEqual(
            set(pinned), set(observed),
            "update release-corpus.json when adding or removing DOCX fixtures",
        )
        for name, digest in observed.items():
            with self.subTest(fixture=name):
                self.assertEqual(pinned[name], digest, "DOCX input bytes changed")


if __name__ == "__main__":
    unittest.main()
