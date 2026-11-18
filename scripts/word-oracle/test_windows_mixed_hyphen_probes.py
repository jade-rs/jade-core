from io import BytesIO
from itertools import product
from zipfile import ZipFile
import json
import unittest

from lxml import etree as ET

import generate_windows_mixed_hyphen_probes as producer
from check_platform_coordinates import ROOT, checked_file, read_json


class MixedSourceHyphenOriginals(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.metadata=read_json(ROOT/'tests/word-oracle/windows/mixed-hyphen-observations.json')
        cls.rows=cls.metadata['fixtures']

    def test_all28_sources_and_portable_facts_reproduce_byte_exactly(self):
        generated={name:(data,facts) for name,data,facts in producer.generate()}
        self.assertEqual(len(generated),28)
        self.assertEqual(set(generated),{r['name'] for r in self.rows})
        combinations=set()
        for row in self.rows:
            with self.subTest(case=row['name']):
                data,facts=generated[row['name']]
                self.assertEqual(data,checked_file(ROOT,row['source']).read_bytes())
                self.assertEqual(facts,row['input_specification'])
                self.assertNotIn('source',facts)
                combinations.add((facts['pattern'],facts['available_width_pt'],facts['compatibility_mode']))
        self.assertEqual(combinations,set(product(producer.PATTERNS,(45,65),(14,15))))
        with self.assertRaisesRegex(ValueError,'Original Mac hyphen source changed'):
            list(producer.generate(b'changed original'))

    def test_source_identity_styles_and_unrelated_original_parts_are_preserved(self):
        q=producer.q
        with ZipFile(ROOT/producer.PARENT) as parent:
            original=ET.fromstring(parent.read('word/document.xml'))
            run_properties=[ET.tostring(node) for node in original.find(q('body')).findall(q('p'))[1].find(q('r')).findall(q('rPr'))]
            for row in self.rows:
                with self.subTest(case=row['name']),ZipFile(checked_file(ROOT,row['source'])) as child:
                    self.assertEqual(child.namelist(),parent.namelist())
                    for part in parent.namelist():
                        if part not in ('word/document.xml','word/settings.xml'):
                            self.assertEqual(child.read(part),parent.read(part),part)
                    body=ET.fromstring(child.read('word/document.xml')).find(q('body'))
                    self.assertEqual(len(body.findall(q('p'))),1)
                    self.assertEqual(ET.tostring(body.find(q('sectPr'))),ET.tostring(original.find(q('body')).find(q('sectPr'))))
                    tokens=[]
                    for run in body.find(q('p')).findall(q('r')):
                        self.assertEqual([ET.tostring(node) for node in run.findall(q('rPr'))],run_properties)
                        tokens.append([[ET.QName(node).localname,node.text] for node in run if node.tag!=q('rPr')])
                    self.assertEqual(tokens,row['input_specification']['tokens'])
                    self.assertEqual(sum((node.text or '').count('\u00ad') for node in body.iter(q('t'))),row['input_specification']['literal_control_count'])
                    self.assertEqual(len(list(body.iter(q('softHyphen')))),row['input_specification']['optional_control_count'])

    def test_actual_word_captures_keep_all28_originals_and_full_provenance(self):
        manifest=read_json(checked_file(ROOT,self.metadata['mac_parent_manifest']))
        self.assertEqual(manifest['reference'],self.metadata['mac_parent_reference'])
        self.assertEqual(manifest['reference']['platform'],'mac')
        self.assertEqual(manifest['reference']['version'],'16.114.4')
        captures={r['capture']['sha256'] for r in self.rows}
        self.assertEqual(len(captures),2)
        for row in self.rows:
            with self.subTest(case=row['name']):
                path=checked_file(ROOT,row['capture']);capture=read_json(path)
                document,=[d for d in capture['documents'] if d['name']==row['name']]
                expected_documents=8 if row['input_specification']['pattern'].startswith('break-') else 20
                self.assertEqual(len(capture['documents']),expected_documents)
                self.assertEqual(capture['word'],row['word'])
                self.assertEqual(row['word']['product_version'],'16.0.19127.20936')
                self.assertEqual(row['word']['executable_sha256'],'7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                self.assertEqual(document['open_mode'],'normal-read-only')
                self.assertEqual(document['sha256'],row['source']['sha256'])
                self.assertEqual(document['compatibility_mode'],row['input_specification']['compatibility_mode'])
                self.assertEqual(row['coordinate_gate_promotion'],'pending')
                fonts=row['original_word_font_programs']
                self.assertTrue(fonts)
                for font in fonts:
                    self.assertIn('Arial',font['family'])
                    self.assertEqual(font['version'],['Version 7.07'])
                    self.assertRegex(font['program_sha256'],r'^[a-f0-9]{64}$')
                with ZipFile(checked_file(ROOT,row['archive'])) as archive:
                    self.assertEqual(len(archive.namelist()),1+expected_documents*3)
                    self.assertEqual(archive.read('capture.json'),path.read_bytes())
                    self.assertEqual(archive.read('inputs/'+row['name']+'.docx'),checked_file(ROOT,row['source']).read_bytes())


if __name__=='__main__':
    unittest.main()
