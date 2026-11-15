from pathlib import Path
import hashlib
import json
import struct
import re
import tempfile
import unittest
from zipfile import ZipFile

from generate_windows_calibri_fallback_style_probes import ROOT, generate, variants


class CalibriFallbackStyleProbeTests(unittest.TestCase):
    def test_style_averages_match_versioned_original_word_font_programs(self):
        import pymupdf
        cases = [
            ('arial', 'bold', 'f60d1cd44efe1577ae3c3aa38cbe321332e21631d2e7ac2571dd3498d587b6e0', 980, 700, 32),
            ('arial', 'bolditalic', 'e2170b90573df8adb63ba464a7eb54cbd002f5d3d7ae80667fbd3576e58c285f', 980, 700, 33),
            ('arial', 'italic', '4690e9a06d95de55f0c1d1bfeaa551306f34c32f6f83f685c23d42c250e6fb99', 904, 400, 1),
            ('calibri', 'bold', 'e5238336d8278b7e843fbc1eaad135e2f212523db43bfa5ab800554725d2ae97', 1098, 700, 32),
            ('calibri', 'bolditalic', '16d1b47eaa69efa0191756b99d60f63e61e3908c987b9486e821f4a801cc8a28', 1099, 700, 33),
            ('calibri', 'italic', 'd1201a4be01c41a0f8d61f5b1a0a408f7a4e2a1d1717781f8ff178e7a29bf8e6', 1067, 400, 1),
        ]
        index = json.loads((ROOT / 'tests/word-oracle/windows/observations/index.json').read_text(encoding='utf-8'))
        for face, style, font_sha, average, weight, selection in cases:
            name = f'word-windows-edge-calibri-fallback-style-{face}-ea-calibri-size12-left-{style}-balance0-compat15'
            with self.subTest(name=name):
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
                    for xref in {f[0] for page in pdf for f in page.get_fonts(full=True)}:
                        font = pdf.extract_font(xref)[3]
                        if font:
                            programs[hashlib.sha256(font).hexdigest()] = font
                font = programs[font_sha]
                count, = struct.unpack_from('>H', font, 4)
                offsets = {font[12 + 16 * i:16 + 16 * i]: struct.unpack_from('>I', font, 20 + 16 * i)[0]
                           for i in range(count)}
                self.assertEqual(struct.unpack_from('>H', font, offsets[b'head'] + 18)[0], 2048)
                self.assertEqual(struct.unpack_from('>h', font, offsets[b'OS/2'] + 2)[0], average)
                self.assertEqual(struct.unpack_from('>H', font, offsets[b'OS/2'] + 4)[0], weight)
                self.assertEqual(struct.unpack_from('>H', font, offsets[b'OS/2'] + 62)[0], selection)

    def test_original_inputs_change_only_run_bold_and_italic(self):
        sources = ROOT / 'tests/word-oracle/windows/observations/sources'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'probes'
            self.assertEqual(generate(output), 36)
            self.assertEqual(len(list(output.glob('*.docx'))), 36)
            self.assertEqual(len({name for name, _, _ in variants()}), 36)
            for name, parent, tags in variants():
                with self.subTest(name=name):
                    self.assertEqual((output / name).read_bytes(), (sources / name).read_bytes())
                    with ZipFile(sources / parent) as original, ZipFile(output / name) as generated:
                        self.assertEqual(original.namelist(), generated.namelist())
                        for part in original.namelist():
                            expected = original.read(part)
                            if part == 'word/document.xml':
                                fonts, = re.findall(rb'<w:rFonts\b[^>]*/>', expected)
                                expected = expected.replace(fonts, fonts + tags.encode())
                            self.assertEqual(generated.read(part), expected)


if __name__ == '__main__':
    unittest.main()
