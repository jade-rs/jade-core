"""Reproduce the captured controls without changing their original coordinates."""
import hashlib
import tempfile
import unittest
from pathlib import Path
from zipfile import ZipFile

from generate_windows_float_lookahead_probes import ROOT, PARENTS, generate


class FloatLookaheadProbes(unittest.TestCase):
    def test_captured_sources_are_reproducible_and_only_compatibility_changes(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'inputs'
            self.assertEqual(generate(output), 9)
            for label, (parent, digest) in PARENTS.items():
                source = ROOT / 'tests/word-oracle/mac-elsa-synthetic' / parent / 'source.docx'
                self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), digest)
                with ZipFile(source) as original:
                    for mode in (12, 14, 15):
                        name = f'word-windows-float-lookahead-{label}-compat{mode}.docx'
                        generated = output / name
                        captured = ROOT / 'tests/word-oracle/windows/observations/sources' / name
                        self.assertEqual(generated.read_bytes(), captured.read_bytes(), name)
                        with ZipFile(generated) as actual:
                            self.assertEqual(actual.namelist(), original.namelist())
                            for part in original.namelist():
                                expected = original.read(part)
                                if part == 'word/settings.xml':
                                    # Exactly one compatibility setting changes; all
                                    # document, formatting and drawing parts remain original.
                                    import re
                                    expected, count = re.subn(
                                        rb'(<w:compatSetting\b[^>]*w:name="compatibilityMode"[^>]*w:val=")14(")',
                                        lambda match: match[1] + str(mode).encode() + match[2],
                                        expected)
                                    self.assertEqual(count, 1)
                                self.assertEqual(actual.read(part), expected, (name, part))


if __name__ == '__main__':
    unittest.main()
