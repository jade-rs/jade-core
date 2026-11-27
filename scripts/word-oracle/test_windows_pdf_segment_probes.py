"""Keep original PDF segmentation evidence separate from renderer acceptance."""
from io import BytesIO
from zipfile import ZipFile
import hashlib
import unittest
from lxml import etree as E
import pymupdf
from check_platform_coordinates import ROOT, checked_file, read_json
from generate_windows_top_bottom_probes import generate as float_controls, q
from generate_windows_pdf_segment_probes import generate


class PDFSegmentProbes(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.metadata = read_json(ROOT / 'tests/word-oracle/windows/pdf-segment-observations.json')
        cls.rows = cls.metadata['fixtures']

    def test_reproduces_four_captured_sources_byte_for_byte(self):
        generated = dict(generate())
        self.assertEqual(len(generated), 4)
        self.assertEqual(set(generated), {r['name'] for r in self.rows})
        for row in self.rows:
            self.assertEqual(generated[row['name']], checked_file(ROOT, row['source']).read_bytes())
            self.assertEqual(row['coordinate_gate_promotion'], 'required')
            self.assertEqual(row['physical_pdf_gate'], 'required')

    def test_only_declared_parts_change_and_body_text_survives_splitting(self):
        original, = [data for name, data, _ in float_controls()
                     if name == 'word-windows-top-bottom-s11-o52p5-b0-m15']
        with ZipFile(BytesIO(original)) as parent:
            text = list(E.fromstring(parent.read('word/document.xml')).iter(q('t')))[-1].text
            for row in self.rows:
                with ZipFile(checked_file(ROOT, row['source'])) as probe:
                    self.assertEqual(set(probe.namelist()), set(parent.namelist()))
                    for name in probe.namelist():
                        if name not in ('word/document.xml', 'word/settings.xml'):
                            self.assertEqual(probe.read(name), parent.read(name))
                    root = E.fromstring(probe.read('word/document.xml'))
                    self.assertEqual(len(list(root.iter(q('drawing')))), 0)
                    body = root.find(q('body'))
                    self.assertEqual(''.join(t.text for p in body.findall(q('p'))
                        if p.find(q('pPr') + '/' + q('pStyle')) is None
                        for t in p.iter(q('t'))), text)

    def test_original_word_mode15_resets_at_256_characters_per_paint_run(self):
        for row in self.rows:
            capture_path = checked_file(ROOT, row['capture'])
            capture = read_json(capture_path)
            self.assertEqual(capture['word'], row['word'])
            self.assertRegex(row['word']['product_version'], r'^\d+\.\d+\.\d+\.\d+$')
            self.assertRegex(row['word']['executable_sha256'], r'^[a-f0-9]{64}$')
            doc, = [d for d in capture['documents'] if d['name'] == row['name']]
            pdf = capture_path.parent / doc['pdf']
            self.assertEqual(hashlib.sha256(pdf.read_bytes()).hexdigest(), doc['pdf_sha256'])
            with pymupdf.open(pdf) as original:
                offset = 0
                starts = []
                for span in original[0].get_texttrace():
                    if span['font'] == 'ArialMT':
                        starts.append(offset - 31)
                    offset += len(span['chars'])
            spec = row['input_specification']
            resets = {256, 512} if spec['paint_run_split_at'] is None else {356}
            if spec['compatibility_mode'] == 15:
                self.assertTrue(resets <= set(starts), (row['name'], starts))
            else:
                self.assertFalse(resets & set(starts), (row['name'], starts))


if __name__ == '__main__':
    unittest.main()
