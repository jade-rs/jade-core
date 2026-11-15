"""Keep justification controls identical to the archived Word inputs."""
from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile
import xml.etree.ElementTree as ET

from generate_windows_justification_probes import ROOT, generate


class JustificationProbeTests(unittest.TestCase):
    def test_all_controls_reproduce_archived_originals_and_preserve_other_parts(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 36)
            sources = list(output.glob('*.docx'))
            self.assertEqual(len(sources), 36)
            with ZipFile(ROOT / 'public/fixtures/2col.docx') as archive:
                original = {name: archive.read(name) for name in archive.namelist()}
            for source in sources:
                with self.subTest(source=source.name):
                    archived = ROOT / 'tests/word-oracle/windows/observations/sources' / source.name
                    self.assertEqual(source.read_bytes(), archived.read_bytes())
                    with ZipFile(source) as archive:
                        self.assertEqual(set(archive.namelist()), set(original))
                        for name, data in original.items():
                            if name not in ('word/document.xml', 'word/settings.xml'):
                                self.assertEqual(archive.read(name), data)

    def test_controls_keep_explicit_kerning_and_manual_break_semantics(self):
        w = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            generate(output)
            for source in output.glob('*.docx'):
                with self.subTest(source=source.name), ZipFile(source) as archive:
                    document = ET.fromstring(archive.read('word/document.xml'))
                    settings = ET.fromstring(archive.read('word/settings.xml'))
                    kern = document.findall('.//' + w + 'kern')
                    breaks = document.findall('.//' + w + 'br')
                    if '-kern-yu20-' in source.name:
                        threshold = source.stem.split('-kern')[-1].split('-')[0]
                        self.assertEqual([node.get(w + 'val') for node in kern], [threshold])
                        self.assertEqual(breaks, [])
                    else:
                        self.assertEqual(kern, [])
                        self.assertEqual(len(breaks), 1)
                        self.assertEqual(settings.find('.//' + w + 'doNotExpandShiftReturn').get(w + 'val'), '0')


if __name__ == '__main__':
    unittest.main()
