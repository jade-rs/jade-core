from pathlib import Path
import hashlib
import json
import tempfile
import unittest
from zipfile import ZipFile

from generate_windows_cross_font_slot_probes import ROOT, generate, variants


class CrossFontSlotProbeTests(unittest.TestCase):
    def test_original_word_arial_and_calibri_east_asia_fallbacks_are_identical(self):
        import pymupdf
        index = json.loads((ROOT / 'tests/word-oracle/windows/observations/index.json').read_text(encoding='utf-8'))
        captures = {document['name']: capture for capture in index['captures']
                    for document in capture['documents']}

        def origins(name):
            capture = captures[name]
            archive = ROOT / capture['archive']
            self.assertEqual(hashlib.sha256(archive.read_bytes()).hexdigest(), capture['archive_sha256'])
            with ZipFile(archive) as bundle:
                manifest = json.loads(bundle.read('capture.json').decode('utf-8-sig'))
                self.assertEqual(manifest['word']['product_version'], '16.0.19127.20936')
                document, = manifest['documents']
                data = bundle.read(document['pdf'])
                self.assertEqual(hashlib.sha256(data).hexdigest(), document['pdf_sha256'])
            with pymupdf.open(stream=data, filetype='pdf') as pdf:
                glyphs = [[number, char['c'], *char['origin']]
                          for number, page in enumerate(pdf, 1)
                          for block in page.get_text('rawdict')['blocks']
                          for line in block.get('lines', []) for span in line['spans']
                          for char in span['chars'] if not char['c'].isspace()]
                self.assertTrue(glyphs)
                return len(pdf), glyphs

        count = 0
        for latin, alternate in [('arial', 'calibri'), ('calibri', 'arial')]:
            for size in (12, 20, 24):
                for balance in (0, 1):
                    for alignment in ('left', 'both'):
                        same = (f'word-windows-edge-cross-slot-{latin}-{latin}-size{size}-{alignment}-balance0-compat15'
                                if balance == 0 else
                                f'word-windows-edge-default-hint-{latin}-size{size}-{alignment}-hintnone-balance1-compat15')
                        other = (f'word-windows-edge-cross-slot-{latin}-{alternate}-size{size}-{alignment}-'
                                 f'balance{balance}-compat15')
                        with self.subTest(first=same, second=other):
                            self.assertEqual(origins(same), origins(other))
                        count += 1
        self.assertEqual(count, 24)

    def test_original_inputs_change_only_the_east_asian_font_slot(self):
        sources = ROOT / 'tests/word-oracle/windows/observations/sources'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 36)
            self.assertEqual(len(list(output.glob('*.docx'))), 36)
            self.assertEqual(len({name for name, _, _ in variants()}), 36)
            for name, parent, edits in variants():
                with self.subTest(name=name):
                    self.assertEqual((output / name).read_bytes(), (sources / name).read_bytes())
                    with ZipFile(sources / parent) as original, ZipFile(output / name) as generated:
                        self.assertEqual(original.namelist(), generated.namelist())
                        for part in original.namelist():
                            expected = original.read(part)
                            if part == 'word/document.xml':
                                for before, after in edits:
                                    expected = expected.replace(before.encode(), after.encode())
                            self.assertEqual(generated.read(part), expected)


if __name__ == '__main__':
    unittest.main()
