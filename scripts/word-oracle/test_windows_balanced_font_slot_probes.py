from pathlib import Path
import hashlib
import json
import tempfile
import unittest
from zipfile import ZipFile

from generate_windows_balanced_font_slot_probes import ROOT, generate, variants


class BalancedFontSlotProbeTests(unittest.TestCase):
    def test_original_word_distinguishes_font_slots_but_not_east_asia_hint(self):
        import pymupdf
        registry = json.loads((ROOT / 'tests/word-oracle/windows/observations/index.json').read_text(encoding='utf-8'))
        captures = {document['name']: capture for capture in registry['captures']
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
                return len(pdf), glyphs

        for family in ('arial', 'calibri'):
            for size in (12, 20, 24):
                for alignment in ('left', 'both'):
                    name = (f'word-windows-edge-default-hint-{family}-size{size}-'
                            f'{alignment}-hintnone-balance1-compat15')
                    with self.subTest(name=name):
                        self.assertEqual(origins(name), origins(name.replace('hintnone', 'hinteastAsia')))
        separate = origins('word-windows-edge-arial-balanced-auto-size20-left-balance1-compat15')
        same = origins('word-windows-edge-default-hint-arial-size20-left-hintnone-balance1-compat15')
        self.assertEqual(separate[0], same[0])
        self.assertEqual([g[:2] for g in separate[1]], [g[:2] for g in same[1]])
        self.assertGreater(max(abs(a[2] - b[2]) for a, b in zip(separate[1], same[1])), .375)

    def test_original_inputs_and_other_package_parts_are_preserved(self):
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
