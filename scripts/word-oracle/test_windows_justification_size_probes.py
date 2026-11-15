from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile

from generate_windows_justification_size_probes import ROOT, generate


class JustificationSizeProbeTests(unittest.TestCase):
    def test_size_only_controls_match_original_word_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 12)
            sources = list(output.glob('*.docx'))
            self.assertEqual(len(sources), 12)
            archived = ROOT / 'tests/word-oracle/windows/observations/sources'
            for source in sources:
                with self.subTest(source=source.name):
                    self.assertEqual(source.read_bytes(), (archived / source.name).read_bytes())
                    size = source.name.split('-yu')[1].split('-')[0]
                    parent = source.name.replace('justify-size-yu' + size, 'justify-tiers-yu20')
                    with ZipFile(source) as actual, ZipFile(archived / parent) as original:
                        self.assertEqual(actual.namelist(), original.namelist())
                        for name in original.namelist():
                            expected = original.read(name)
                            if name == 'word/document.xml':
                                for tag in (b'sz', b'szCs'):
                                    expected = expected.replace(
                                        b'<w:' + tag + b' w:val="40"/>',
                                        b'<w:' + tag + b' w:val="' + str(int(size) * 2).encode() + b'"/>')
                            self.assertEqual(actual.read(name), expected)


if __name__ == '__main__':
    unittest.main()
