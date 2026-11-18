"""Protect original Word percentage-table inputs, parts, versions and widths."""
import hashlib
from io import BytesIO
import itertools
import json
from pathlib import Path
import unittest
from zipfile import ZipFile
from lxml import etree as ET
from check_platform_coordinates import checked_file,compare,pdf_origins
from extract_windows_emf import extract
import generate_windows_percentage_table_probes as generator

ROOT=Path(__file__).resolve().parents[2]
read=lambda p:json.loads(p.read_text(encoding='utf-8-sig'))
sha=lambda data:hashlib.sha256(data).hexdigest()

class PercentageTableProbes(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.metadata=read(ROOT/'tests/word-oracle/windows/percentage-table-observations.json')
        cls.rows=cls.metadata['fixtures'];cls.parent=(ROOT/generator.PARENT).read_bytes()

    def test_all108_inputs_reproduce_exact_captured_bytes_and_matrix(self):
        generated={name:(data,facts) for name,data,facts in generator.generate(self.parent)}
        self.assertEqual(len(generated),108);self.assertEqual(set(generated),{r['name'] for r in self.rows})
        combinations=set()
        for row in self.rows:
            with self.subTest(case=row['name']):
                data,facts=generated[row['name']]
                self.assertEqual(data,checked_file(ROOT,row['source']).read_bytes())
                self.assertEqual(row['input_specification'],dict(name=row['name'],source_sha256=sha(data),**facts))
                combinations.add((facts['layout'],facts['table_percent'],facts['saved_grid_percent'],facts['cell_percent'],facts['source_compatibility_mode']))
        self.assertEqual(combinations,set(itertools.product(('autofit','fixed'),(50,100),(10,30,60),(10,30,60),(None,14,15))))
        with self.assertRaisesRegex(ValueError,'Original Mac borderless-table source changed'):
            list(generator.generate(self.parent+b'changed'))

    def test_mac_parent_package_parts_text_and_section_are_preserved(self):
        manifest=read(checked_file(ROOT,self.metadata['mac_parent_manifest']))
        self.assertEqual(manifest['reference'],self.metadata['mac_parent_reference'])
        self.assertEqual(manifest['reference']['platform'],'mac')
        self.assertEqual(manifest['sourceSha256'],generator.PARENT_SHA256)
        with ZipFile(BytesIO(self.parent)) as original:
            doc=ET.fromstring(original.read('word/document.xml'));body=doc.find(generator.q('body'))
            paragraphs=body.findall(generator.q('p'));table=body.find(generator.q('tbl'))
            expected=[t.text for p in (paragraphs[0],table,paragraphs[1]) for t in p.iter(generator.q('t'))]
            section=ET.tostring(body.find(generator.q('sectPr')))
            self.assertNotIn('word/settings.xml',original.namelist())
            for row in self.rows:
                with self.subTest(case=row['name']),ZipFile(checked_file(ROOT,row['source'])) as package:
                    facts=row['input_specification'];changes=set(facts['changed_parts'])
                    expected_changes={'word/document.xml'}
                    if facts['source_compatibility_mode'] is not None:
                        expected_changes|={'word/settings.xml','word/_rels/document.xml.rels','[Content_Types].xml'}
                    self.assertEqual(changes,expected_changes)
                    self.assertEqual(set(package.namelist()),set(original.namelist())|({'word/settings.xml'} if 'word/settings.xml' in changes else set()))
                    for part in original.namelist():
                        if part not in changes:self.assertEqual(package.read(part),original.read(part),part)
                    rendered=ET.fromstring(package.read('word/document.xml'));current=rendered.find(generator.q('body'))
                    self.assertEqual([t.text for t in current.iter(generator.q('t'))],expected)
                    self.assertEqual(ET.tostring(current.find(generator.q('sectPr'))),section)
                    if facts['source_compatibility_mode'] is not None:
                        rels=ET.fromstring(package.read('word/_rels/document.xml.rels'))
                        self.assertEqual(len({r.get('Id') for r in rels}),len(rels))
                        setting,=[r for r in rels if r.get('Type').endswith('/settings')]
                        self.assertEqual(setting.get('Target'),'settings.xml')
                        types=ET.fromstring(package.read('[Content_Types].xml'))
                        self.assertEqual(len([t for t in types if t.get('PartName')=='/word/settings.xml']),1)

    def test_word_compatibility_and_original_font_versions_are_retained(self):
        for row in self.rows:
            with self.subTest(case=row['name']):
                capture=read(checked_file(ROOT,row['capture']));document,=capture['documents']
                self.assertEqual(capture['word'],row['word'])
                self.assertEqual(row['word']['product_version'],'16.0.19127.20936')
                self.assertEqual(row['word']['executable_sha256'],'7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                self.assertEqual(document['open_mode'],'normal-read-only')
                self.assertEqual(document['compatibility_mode'],row['input_specification']['word_compatibility_mode'])
                self.assertEqual(document['sha256'],row['source']['sha256'])
                programs=[p for p in row['original_word_font_programs'] if 'Yu Mincho' in p['family']]
                self.assertTrue(programs)
                for program in programs:self.assertEqual(program['version'],['Version 1.92'])

    def test_original_word_uses_cell_percentages_independently_of_saved_grid(self):
        for row in self.rows:
            with self.subTest(case=row['name']):
                manifest=checked_file(ROOT,row['capture']);document,=read(manifest)['documents']
                word=pdf_origins(manifest.parent/document['pdf'],True);emf,=extract(manifest)['fixtures']
                self.assertGreater(compare(word,emf,row['name']+'/original-outputs'),0)
                first=lambda text:next(g for g in word['glyphs'] if g[1]==text)
                width=first('記')[2]-first('一')[2];facts=row['input_specification']
                self.assertLessEqual(abs(width-451.3*facts['table_percent']/100*facts['cell_percent']/100),.375)

if __name__=='__main__':unittest.main()
