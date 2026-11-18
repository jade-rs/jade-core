"""Reproduce all original positive-kerning controls and protect their CI assertions."""
from io import BytesIO
from zipfile import ZipFile
import copy
import hashlib
import itertools
import unittest
from lxml import etree as E
import pymupdf
from check_platform_coordinates import ROOT, checked_file, read_json
import check_windows_positive_kerning as gate
import generate_windows_positive_kerning_probes as producer


class PositiveKerningProbes(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.metadata = read_json(ROOT / gate.INDEX)
        cls.rows = cls.metadata['fixtures']

    def test_all192_sources_are_byte_exact_and_cover_the_full_matrix(self):
        generated = {name: (data, spec) for name, data, spec in producer.generate()}
        self.assertEqual(len(generated), 192)
        self.assertEqual(set(generated), {r['name'] for r in self.rows})
        coverage = set()
        for row in self.rows:
            data, spec = generated[row['name']]
            self.assertEqual(data, checked_file(ROOT, row['source']).read_bytes())
            self.assertEqual(spec, row['input_specification'])
            coverage.add((spec['style'], spec['font_size_pt'], spec['character'], spec['compatibility_mode']))
        self.assertEqual(coverage, set(itertools.product(
            ('bold', 'bolditalic', 'italic', 'regular'), (13, 13.5, 14, 14.5, 15, 15.5, 16, 20), 'XVZ', (14, 15))))

    def test_only_declared_text_and_page_width_orientation_change(self):
        q = producer.q
        for row in self.rows:
            spec = row['input_specification']
            with self.subTest(case=row['name']), ZipFile(checked_file(ROOT, spec['derived_parent'])) as before, \
                    ZipFile(checked_file(ROOT, row['source'])) as after:
                self.assertEqual(set(before.namelist()), set(after.namelist()))
                for part in before.namelist():
                    if part != 'word/document.xml':
                        self.assertEqual(before.read(part), after.read(part))
                old, new = [E.fromstring(z.read('word/document.xml')) for z in (before, after)]
                old_text, new_text = [list(root.iter(q('t'))) for root in (old, new)]
                self.assertEqual(len(old_text), 4)
                self.assertEqual(len(new_text), 4)
                for a, b in zip(old_text, new_text, strict=True):
                    self.assertEqual(b.text, spec['character'] * 48)
                    b.text = a.text
                old_page, = old.iter(q('pgSz')); new_page, = new.iter(q('pgSz'))
                self.assertEqual(new_page.get(q('w')), '17280')
                self.assertEqual(new_page.get(q('h')), old_page.get(q('h')))
                self.assertEqual(new_page.get(q('orient')), 'landscape')
                new_page.attrib.clear(); new_page.attrib.update(old_page.attrib)
                self.assertEqual(E.tostring(old), E.tostring(new))
                self.assertEqual([int(k.get(q('val'))) for k in new.iter(q('kern'))], [200, 200, 2, 2])

    def test_original_capture_versions_executable_fonts_and_mac_parent_are_preserved(self):
        mac = read_json(checked_file(ROOT, self.metadata['mac_parent_manifest']))
        self.assertEqual(mac['reference'], self.metadata['mac_parent_reference'])
        self.assertEqual(mac['reference']['platform'], 'mac')
        self.assertEqual(mac['sourceSha256'], producer.parentgen.PARENT_SHA256)
        captures = {}
        for row in self.rows:
            path = checked_file(ROOT, row['capture'])
            if path not in captures:
                captures[path] = read_json(path)
            capture = captures[path]
            self.assertEqual(capture['word'], row['word'])
            self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
            self.assertEqual(capture['word']['executable_sha256'],
                '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
            self.assertEqual(capture['print_environment'], self.metadata['print_environment'])
            self.assertTrue(capture['print_environment']['active_printer'])
            doc, = [d for d in capture['documents'] if d['name'] == row['name']]
            self.assertEqual(doc['open_mode'], 'normal-read-only')
            self.assertEqual(doc['sha256'], row['source']['sha256'])
            self.assertEqual(doc['compatibility_mode'], row['input_specification']['compatibility_mode'])
            with ZipFile(checked_file(ROOT, row['archive'])) as archive:
                self.assertEqual(archive.read('capture.json'), path.read_bytes())
                self.assertEqual(archive.read('inputs/' + row['name'] + '.docx'), checked_file(ROOT, row['source']).read_bytes())
                for filename, digest in [(doc['pdf'], doc['pdf_sha256'])] + [
                        (page['file'], page['sha256']) for page in doc['metafiles']]:
                    data = (path.parent / filename).read_bytes()
                    self.assertEqual(data, archive.read(filename))
                    self.assertEqual(hashlib.sha256(data).hexdigest(), digest)
            with pymupdf.open(path.parent / doc['pdf']) as pdf:
                programs = {hashlib.sha256(pdf.extract_font(font[0])[3]).hexdigest()
                    for page in pdf for font in page.get_fonts(full=True) if pdf.extract_font(font[0])[3]}
            self.assertEqual(programs, {f['program_sha256'] for f in row['original_word_font_programs']})

    def test_screen_and_validated_print_assertions_cannot_be_removed_or_swapped(self):
        pending = gate.locked_promotions(self.metadata)
        self.assertEqual(len(pending), 100)
        required = next(r['name'] for r in self.rows if r['name'] not in pending)
        def swap(index):
            a = next(r for r in index['fixtures'] if r['name'] == required)
            b = next(r for r in index['fixtures'] if r['name'] in pending)
            a['coordinate_gate_promotion'], b['coordinate_gate_promotion'] = 'pending', 'aptos-positive-kerning'
        for edit in [lambda index: index['fixtures'].pop(),
                     lambda index: index['fixtures'][0].update(screen_gate_promotion='pending'),
                     lambda index: next(r for r in index['fixtures'] if r['name'] == required).update(coordinate_gate_promotion='pending'), swap]:
            changed = copy.deepcopy(self.metadata); edit(changed)
            with self.assertRaises(ValueError):
                gate.locked_promotions(changed)


if __name__ == '__main__':
    unittest.main()
