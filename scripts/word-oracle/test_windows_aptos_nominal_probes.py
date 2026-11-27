"""Reproduce nominal controls and verify the original versioned Word evidence."""
from io import BytesIO
from zipfile import ZipFile
import hashlib
import itertools
import unittest
from unittest.mock import patch
import copy
import check_windows_aptos_nominal as gate
from lxml import etree as E
import pymupdf
from check_platform_coordinates import ROOT, checked_file, read_json
import generate_windows_aptos_nominal_probes as producer


class AptosNominalProbes(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.metadata = read_json(ROOT / 'tests/word-oracle/windows/aptos-nominal-observations.json')
        cls.rows = cls.metadata['fixtures']

    def test_all24_captured_inputs_are_reproducible_byte_for_byte(self):
        generated = {name: (data, spec) for name, data, spec in producer.generate()}
        self.assertEqual(set(generated), {r['name'] for r in self.rows})
        self.assertEqual(len(generated), 24)
        coverage = set()
        for row in self.rows:
            data, spec = generated[row['name']]
            with self.subTest(case=row['name']):
                self.assertEqual(data, checked_file(ROOT, row['source']).read_bytes())
                self.assertEqual(spec, row['input_specification'])
                coverage.add((spec['style'], spec['font_size_pt'], spec['character'], spec['compatibility_mode']))
        self.assertEqual(coverage, {(style, size, char, mode)
            for (style, size), char, mode in itertools.product(
                [('bold', 14), ('bolditalic', 15), ('regular', 16)], 'Xa1-', (14, 15))})

    def test_only_four_text_nodes_change_with_identical_parent_formatting(self):
        q = producer.parentgen.q
        for row in self.rows:
            spec = row['input_specification']
            parent = checked_file(ROOT, spec['derived_parent'])
            with self.subTest(case=row['name']), ZipFile(parent) as before, ZipFile(
                    checked_file(ROOT, row['source'])) as after:
                self.assertEqual(set(before.namelist()), set(after.namelist()))
                for part in before.namelist():
                    if part != 'word/document.xml':
                        self.assertEqual(before.read(part), after.read(part))
                old, new = [E.fromstring(z.read('word/document.xml')) for z in (before, after)]
                old_text, new_text = [list(doc.iter(q('t'))) for doc in (old, new)]
                self.assertEqual(len(old_text), 4)
                self.assertEqual(len(new_text), 4)
                for a, b in zip(old_text, new_text, strict=True):
                    self.assertEqual(b.text, spec['character'] * 48)
                    b.text = a.text
                self.assertEqual(E.tostring(old), E.tostring(new))
                self.assertEqual(spec['retained_kerning_thresholds_halfpoints'], [200, 200, 2, 2])

    def test_original_word_pdf_emf_fonts_and_mac_reference_are_immutable(self):
        mac = read_json(checked_file(ROOT, self.metadata['mac_parent_manifest']))
        self.assertEqual(mac['reference'], self.metadata['mac_parent_reference'])
        self.assertEqual(mac['reference']['platform'], 'mac')
        self.assertEqual(mac['sourceSha256'], producer.parentgen.PARENT_SHA256)
        for row in self.rows:
            path = checked_file(ROOT, row['capture'])
            capture = read_json(path)
            self.assertEqual(capture['word'], row['word'])
            self.assertEqual(row['word']['product_version'], '16.0.19127.20936')
            self.assertEqual(row['word']['executable_sha256'],
                '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
            doc, = [d for d in capture['documents'] if d['name'] == row['name']]
            self.assertEqual(doc['open_mode'], 'normal-read-only')
            self.assertEqual(doc['sha256'], row['source']['sha256'])
            self.assertEqual(doc['compatibility_mode'], row['input_specification']['compatibility_mode'])
            with ZipFile(checked_file(ROOT, row['archive'])) as archive:
                self.assertEqual(archive.read('capture.json'), path.read_bytes())
                self.assertEqual(archive.read('inputs/' + row['name'] + '.docx'),
                    checked_file(ROOT, row['source']).read_bytes())
                for filename, digest in [(doc['pdf'], doc['pdf_sha256'])] + [
                        (p['file'], p['sha256']) for p in doc['metafiles']]:
                    data = (path.parent / filename).read_bytes()
                    self.assertEqual(archive.read(filename), data)
                    self.assertEqual(hashlib.sha256(data).hexdigest(), digest)
            with pymupdf.open(path.parent / doc['pdf']) as pdf:
                programs = {hashlib.sha256(pdf.extract_font(font[0])[3]).hexdigest()
                    for page in pdf for font in page.get_fonts(full=True)
                    if pdf.extract_font(font[0])[3]}
            self.assertEqual(programs, {p['program_sha256'] for p in row['original_word_font_programs']})
            self.assertEqual(row['coordinate_gate_promotion'],
                'pending' if row['name'] in gate.PENDING_PDF else 'aptos-nominal')
            self.assertEqual(row['screen_gate_promotion'], 'aptos-nominal-screen')

    def test_required_case_and_screen_assertions_cannot_be_removed(self):
        original = gate.read_json
        for edit in [lambda index: index['fixtures'].pop(),
                     lambda index: index['fixtures'][0].update(screen_gate_promotion='pending')]:
            index = copy.deepcopy(self.metadata); edit(index)
            with patch.object(gate, 'read_json', side_effect=lambda path:
                    index if path == ROOT / gate.INDEX else original(path)):
                with self.assertRaises(ValueError):
                    gate.verify()

    def test_each_required_output_plane_rejects_horizontal_and_vertical_drift(self):
        expected = dict(page_count=1, glyphs=[[1, 'X', 72., 80.]])
        gate.assert_outputs(*[expected for _ in range(7)], 'positive')
        for plane in range(3, 7):
            for axis in (2, 3):
                models = [copy.deepcopy(expected) for _ in range(7)]
                models[plane]['glyphs'][0][axis] += .376
                with self.subTest(plane=plane, axis=axis), self.assertRaises(ValueError):
                    gate.assert_outputs(*models, 'drift')


if __name__ == '__main__':
    unittest.main()
