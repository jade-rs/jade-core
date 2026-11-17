"""Protect reproducible minimum-line inputs and real Word/font provenance."""
import hashlib
from io import BytesIO
import itertools
import json
from pathlib import Path
import unittest
from zipfile import ZipFile
from lxml import etree as ET
from check_platform_coordinates import checked_file, compare, pdf_origins
from extract_windows_emf import extract
import generate_windows_minimum_line_probes as generator

ROOT=Path(__file__).resolve().parents[2]
read=lambda p:json.loads(p.read_text(encoding='utf-8-sig'))
sha=lambda raw:hashlib.sha256(raw).hexdigest()

class MinimumLineProbes(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.metadata=read(ROOT/'tests/word-oracle/windows/minimum-line-observations.json')
        cls.rows=cls.metadata['fixtures']
        cls.parent=(ROOT/generator.PARENT).read_bytes()

    def test_all108_inputs_and_explicit_minimum_auto_pairs_reproduce(self):
        generated={name:(data,facts) for name,data,facts in generator.generate(self.parent)}
        self.assertEqual(len(generated),108)
        self.assertEqual(set(generated),{row['name'] for row in self.rows})
        combinations=set()
        for row in self.rows:
            with self.subTest(case=row['name']):
                data,facts=generated[row['name']]
                self.assertEqual(data,checked_file(ROOT,row['source']).read_bytes())
                self.assertEqual(dict(name=row['name'],source_sha256=sha(data),**facts),row['input_specification'])
                combinations.add((facts['font_family'],facts['script'],facts['profile'],facts['minimum_pt'],facts['compatibility_mode']))
                with ZipFile(BytesIO(data)) as package:
                    body=ET.fromstring(package.read('word/document.xml')).find(generator.q('body'))
                    paragraphs=body.findall(generator.q('p'));self.assertEqual(len(paragraphs),2)
                    for paragraph,(rule,line) in zip(paragraphs,(('atLeast',facts['minimum_pt']*20),('auto',240))):
                        props=paragraph.find(generator.q('pPr'));spacing=props.find(generator.q('spacing'))
                        self.assertEqual(spacing.get(generator.q('lineRule')),rule)
                        self.assertEqual(spacing.get(generator.q('line')),str(line))
                        self.assertEqual(props.find(generator.q('snapToGrid')).get(generator.q('val')),'0')
                        self.assertEqual(len(paragraph.findall('.//'+generator.q('br'))),1)
        pairs=(('Yu Mincho','latin'),('Yu Mincho','cjk'),('MS Mincho','latin'),('MS Mincho','cjk'),('Arial','latin'),('Courier New','latin'))
        self.assertEqual(combinations,{(*pair,*rest) for pair in pairs for rest in itertools.product(('uniform12','uniform20','mixed20p10'),(12,20,28),(14,15))})
        with self.assertRaisesRegex(ValueError,'Original Mac minimum-line source changed'):
            list(generator.generate(self.parent+b'changed'))

    def test_original_mac_reference_and_unmodified_package_parts_are_preserved(self):
        manifest=read(checked_file(ROOT,self.metadata['mac_parent_manifest']))
        self.assertEqual(manifest['reference'],self.metadata['mac_parent_reference'])
        self.assertEqual(manifest['reference']['version'],'16.114.3')
        self.assertEqual(manifest['sourceSha256'],generator.PARENT_SHA256)
        with ZipFile(BytesIO(self.parent)) as original:
            section=ET.fromstring(original.read('word/document.xml')).find(generator.q('body')).find(generator.q('sectPr'))
            for row in self.rows:
                with self.subTest(case=row['name']),ZipFile(checked_file(ROOT,row['source'])) as package:
                    self.assertEqual(package.namelist(),original.namelist())
                    for part in original.namelist():
                        if part not in ('word/document.xml','word/settings.xml'):
                            self.assertEqual(package.read(part),original.read(part),part)
                    actual=ET.fromstring(package.read('word/document.xml')).find(generator.q('body')).find(generator.q('sectPr'))
                    self.assertEqual(ET.tostring(actual),ET.tostring(section))

    def test_actual_word_and_embedded_font_versions_are_retained(self):
        versions={'Arial':'Version 7.07','Courier New':'Version 6.95','MS Mincho':'Version 5.32','Yu Mincho':'Version 1.92'}
        for row in self.rows:
            with self.subTest(case=row['name']):
                capture=read(checked_file(ROOT,row['capture']));document,=capture['documents']
                self.assertEqual(capture['word'],row['word'])
                self.assertEqual(row['word']['product_version'],'16.0.19127.20936')
                self.assertEqual(row['word']['executable_sha256'],'7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                self.assertEqual(document['open_mode'],'normal-read-only')
                self.assertEqual(document['sha256'],row['source']['sha256'])
                self.assertEqual(document['compatibility_mode'],row['input_specification']['compatibility_mode'])
                family=row['input_specification']['font_family']
                programs=[p for p in row['original_word_font_programs'] if family in p['family']]
                self.assertTrue(programs)
                for program in programs:self.assertEqual(program['version'],[versions[family]])

    def test_independent_original_word_screen_and_pdf_agree_before_native_comparison(self):
        for row in self.rows:
            with self.subTest(case=row['name']):
                manifest=checked_file(ROOT,row['capture']);document,=read(manifest)['documents']
                emf,=extract(manifest)['fixtures']
                self.assertGreater(compare(pdf_origins(manifest.parent/document['pdf'],True),emf,row['name']),0)

if __name__=='__main__':unittest.main()
