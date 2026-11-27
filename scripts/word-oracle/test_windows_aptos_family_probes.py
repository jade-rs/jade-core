"""Preserve isolated Aptos/Display observations without accepting renderer drift."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

from check_platform_coordinates import ROOT, checked_file, compare, pdf_origins, read_json
from extract_windows_emf import emf_glyphs, extract


class WindowsAptosFamilyProbes(unittest.TestCase):
    def test_promoted_origins_keep_the_original_capture_name_and_actual_mode(self):
        cases = {c['name']: c for c in self.cases()}
        registry = read_json(ROOT / 'tests/word-oracle/windows/font-coordinates.json')
        promoted = [c for c in registry['fixtures'] if c['name'] in cases]
        self.assertEqual(len(promoted), 8)
        for case in promoted:
            fact = cases[case['name']]
            with self.subTest(case=case['name']):
                self.assertEqual(case['capture_document_name'], fact['original_capture_document_name'])
                source = checked_file(ROOT, case['source'])
                self.assertEqual(source.read_bytes(), (ROOT / fact['source']).read_bytes())
                manifest = checked_file(ROOT, case['capture'])
                capture = read_json(manifest)
                self.assertEqual(capture['word'], fact['word'])
                document, = capture['documents']
                self.assertEqual(document['name'], case['capture_document_name'])
                self.assertEqual(document['compatibility_mode'], 14)
                self.assertEqual((manifest.parent / 'inputs' / (document['name'] + '.docx')).read_bytes(),
                                 source.read_bytes())
                self.assertEqual(read_json(checked_file(ROOT, case['drawing_reference'])), extract(manifest))

    def cases(self):
        return read_json(ROOT / 'tests/word-oracle/windows/aptos-family-probes/observations.json')['fixtures']

    def test_eight_inputs_isolate_family_order_and_size_without_changing_settings(self):
        cases = self.cases()
        self.assertEqual(len(cases), 8)
        self.assertEqual({c['font_size_pt'] for c in cases}, {12, 20})
        self.assertEqual({tuple(c['paragraph_families']) for c in cases}, {
            ('Aptos',), ('Aptos Display',), ('Aptos', 'Aptos Display'), ('Aptos Display', 'Aptos')})
        ns = {'w': 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'}
        q = lambda name: '{' + ns['w'] + '}' + name
        for case in cases:
            source, parent = ROOT / case['source'], ROOT / case['parent_source']
            with self.subTest(case=case['name']), ZipFile(source) as actual, ZipFile(parent) as original:
                self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), case['source_sha256'])
                self.assertEqual(hashlib.sha256(parent.read_bytes()).hexdigest(), case['parent_sha256'])
                self.assertEqual(actual.namelist(), original.namelist())
                for part in actual.namelist():
                    if part != 'word/document.xml':
                        self.assertEqual(actual.read(part), original.read(part))
                document = ET.fromstring(actual.read('word/document.xml'))
                paragraphs = document.findall('w:body/w:p', ns)
                self.assertEqual(len(paragraphs), len(case['paragraph_families']))
                for paragraph, family in zip(paragraphs, case['paragraph_families']):
                    self.assertEqual(paragraph.find('w:r/w:t', ns).text, 'AV To 11x')
                    props = paragraph.find('w:r/w:rPr', ns)
                    for attribute in ('ascii', 'hAnsi'):
                        self.assertEqual(props.find('w:rFonts', ns).get(q(attribute)), family)
                    self.assertEqual(props.find('w:sz', ns).get(q('val')), str(case['font_size_pt'] * 2))

    def test_actual_word_version_mode_source_and_independent_drawing_planes(self):
        index = read_json(ROOT / 'tests/word-oracle/windows/observations/index.json')
        records = {c['capture_id']: c for c in index['captures']}
        for case in self.cases():
            record = records[case['capture_id']]
            archive = ROOT / record['archive']
            with self.subTest(case=case['name']), ZipFile(archive) as bundle:
                self.assertEqual(hashlib.sha256(archive.read_bytes()).hexdigest(), record['archive_sha256'])
                capture = json.loads(bundle.read('capture.json').decode('utf-8-sig'))
                self.assertEqual(capture['status'], 'complete')
                self.assertEqual(capture['word'], case['word'])
                self.assertEqual(case['word']['product_version'], '16.0.19127.20936')
                self.assertEqual(case['word']['executable_sha256'],
                                 '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                document, = capture['documents']
                self.assertEqual(document['name'], case['original_capture_document_name'])
                self.assertEqual(document['compatibility_mode'], case['actual_compatibility_mode'])
                self.assertEqual(document['compatibility_mode'], 14)
                self.assertEqual(document['open_mode'], 'normal-read-only')
                self.assertEqual(bundle.read('inputs/' + document['name'] + '.docx'),
                                 (ROOT / case['source']).read_bytes())
                glyphs = []
                for page in document['metafiles']:
                    raw = bundle.read(page['file'])
                    self.assertEqual(hashlib.sha256(raw).hexdigest(), page['sha256'])
                    glyphs.extend(emf_glyphs(raw, page['page'], page['width_pt'], page['height_pt']))
                expected = dict(page_count=document['pages'], glyphs=glyphs)
                with tempfile.TemporaryDirectory() as directory:
                    pdf = Path(directory) / 'original.pdf'
                    pdf.write_bytes(bundle.read(document['pdf']))
                    self.assertEqual(hashlib.sha256(pdf.read_bytes()).hexdigest(), document['pdf_sha256'])
                    self.assertEqual(compare(pdf_origins(pdf), expected, case['name']),
                                     7 * len(case['paragraph_families']))


if __name__ == '__main__':
    unittest.main()
