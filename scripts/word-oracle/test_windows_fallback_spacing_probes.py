from pathlib import Path
import hashlib
import json
import struct
import tempfile
import unittest
from zipfile import ZipFile

from generate_windows_fallback_spacing_probes import ROOT, generate, variants


class FallbackSpacingProbeTests(unittest.TestCase):
    def test_calibri_average_width_comes_from_the_original_word_font_program(self):
        import pymupdf
        name = 'word-windows-edge-calibri-auto-size12-left-balance0-compat15'
        index = json.loads((ROOT / 'tests/word-oracle/windows/observations/index.json').read_text(encoding='utf-8'))
        record, = [c for c in index['captures'] if any(d['name'] == name for d in c['documents'])]
        archive = ROOT / record['archive']
        self.assertEqual(hashlib.sha256(archive.read_bytes()).hexdigest(), record['archive_sha256'])
        with ZipFile(archive) as bundle:
            capture = json.loads(bundle.read('capture.json').decode('utf-8-sig'))
            self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
            document, = capture['documents']
            original_pdf = bundle.read(document['pdf'])
            self.assertEqual(hashlib.sha256(original_pdf).hexdigest(), document['pdf_sha256'])
        programs = {}
        with pymupdf.open(stream=original_pdf, filetype='pdf') as pdf:
            for xref in {face[0] for page in pdf for face in page.get_fonts(full=True)}:
                font = pdf.extract_font(xref)[3]
                if font:
                    programs[hashlib.sha256(font).hexdigest()] = font
        font = programs['bb3b401f7ccf6e1f9dbf79ef080f04be6affba7f3e804a616accc6641ceadff7']
        count, = struct.unpack_from('>H', font, 4)
        offsets = {font[12 + 16 * i:16 + 16 * i]: struct.unpack_from('>I', font, 20 + 16 * i)[0]
                   for i in range(count)}
        self.assertEqual(struct.unpack_from('>h', font, offsets[b'OS/2'] + 2)[0], 1067)
        self.assertEqual(struct.unpack_from('>H', font, offsets[b'head'] + 18)[0], 2048)

    def test_all_original_inputs_and_unrelated_package_parts_are_preserved(self):
        sources = ROOT / 'tests/word-oracle/windows/observations/sources'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 36)
            self.assertEqual(len(list(output.glob('*.docx'))), 36)
            self.assertEqual(len({name for name, _, _ in variants()}), 36)
            for name, source, edits in variants():
                with self.subTest(name=name):
                    self.assertEqual((output / name).read_bytes(), (sources / name).read_bytes())
                    with ZipFile(sources / source) as original, ZipFile(output / name) as generated:
                        self.assertEqual(original.namelist(), generated.namelist())
                        for part in original.namelist():
                            expected = original.read(part)
                            if part == 'word/document.xml':
                                for before, after in edits:
                                    expected = expected.replace(before.encode(), after.encode())
                            self.assertEqual(generated.read(part), expected)


if __name__ == '__main__':
    unittest.main()
