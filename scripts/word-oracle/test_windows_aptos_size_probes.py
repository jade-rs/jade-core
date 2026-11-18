"""Protect size-only Word inputs and original versioned measurement evidence."""
from copy import deepcopy
from io import BytesIO
import itertools,json,unittest
from zipfile import ZipFile
from lxml import etree as ET
from check_platform_coordinates import checked_file
import generate_windows_aptos_size_probes as producer
import check_windows_aptos_sizes as gate

ROOT=producer.ROOT
read=lambda p:json.loads(p.read_text(encoding='utf-8-sig'))

class AptosSizeProbes(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.metadata=read(ROOT/'tests/word-oracle/windows/aptos-font-size-observations.json')
        cls.rows=cls.metadata['fixtures']

    def test_all64_inputs_and_facts_reproduce_exactly(self):
        generated={name:(data,facts) for name,data,facts in producer.generate()}
        self.assertEqual(len(generated),64);self.assertEqual(set(generated),{r['name'] for r in self.rows})
        actual=set()
        for row in self.rows:
            with self.subTest(case=row['name']):
                data,facts=generated[row['name']]
                self.assertEqual(data,checked_file(ROOT,row['source']).read_bytes())
                self.assertEqual(facts,row['input_specification'])
                actual.add((facts['style'],facts['font_size_pt'],facts['compatibility_mode']))
        self.assertEqual(actual,set(itertools.product(('regular','italic','bold','bolditalic'),(13,13.5,14,14.5,15,15.5,16,20),(14,15))))
        parent=producer.PARENTS[0]
        with self.assertRaisesRegex(ValueError,'Captured Aptos parent changed'):
            list(producer.generate_parent((ROOT/parent['file']).read_bytes()+b'changed',parent))

    def test_only_run_size_changes_and_size14_recaptures_are_identical(self):
        repeats=0
        for row in self.rows:
            facts=row['input_specification'];source=checked_file(ROOT,row['source']).read_bytes();before=(ROOT/facts['parent_source']).read_bytes()
            with self.subTest(case=row['name']),ZipFile(BytesIO(before)) as parent,ZipFile(BytesIO(source)) as child:
                self.assertEqual(set(parent.namelist()),set(child.namelist()))
                for part in parent.namelist():
                    if part!='word/document.xml':self.assertEqual(parent.read(part),child.read(part),part)
                old,new=(ET.fromstring(z.read('word/document.xml')) for z in (parent,child))
                self.assertEqual([t.text for t in old.iter(producer.q('t'))],[t.text for t in new.iter(producer.q('t'))])
                self.assertEqual([ET.tostring(p) for p in old.iter(producer.q('pPr'))],[ET.tostring(p) for p in new.iter(producer.q('pPr'))])
                self.assertEqual(ET.tostring(old.find('.//'+producer.q('sectPr'))),ET.tostring(new.find('.//'+producer.q('sectPr'))))
                for old_run,new_run in zip(old.findall('.//'+producer.q('r')),new.findall('.//'+producer.q('r')),strict=True):
                    old_props,new_props=(deepcopy(r.find(producer.q('rPr'))) for r in (old_run,new_run))
                    for tag in ('sz','szCs'):
                        a,b=old_props.find(producer.q(tag)),new_props.find(producer.q(tag))
                        self.assertEqual(a is None,b is None)
                        if a is not None:
                            self.assertEqual(b.get(producer.q('val')),str(int(facts['font_size_pt']*2)))
                            old_props.remove(a);new_props.remove(b)
                    self.assertEqual(ET.tostring(old_props),ET.tostring(new_props))
                if facts['font_size_pt']==14:
                    self.assertEqual(source,before);self.assertEqual(facts['changed_parts'],[]);repeats+=1
                else:self.assertEqual(facts['changed_parts'],['word/document.xml'])
        self.assertEqual(repeats,8)

    def test_actual_word_modes_executable_and_original_font_versions_are_retained(self):
        for row in self.rows:
            with self.subTest(case=row['name']):
                capture=read(checked_file(ROOT,row['capture']));document,=capture['documents']
                self.assertEqual(capture['word'],row['word'])
                self.assertEqual(row['word']['product_version'],'16.0.19127.20936')
                self.assertEqual(row['word']['executable_sha256'],'7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                self.assertEqual(document['sha256'],row['source']['sha256'])
                self.assertEqual(document['compatibility_mode'],row['input_specification']['compatibility_mode'])
                self.assertEqual(document['open_mode'],'normal-read-only')
                programs=[p for p in row['original_word_font_programs'] if 'Aptos Display' in p['family']]
                self.assertTrue(programs)
                for program in programs:self.assertEqual(program['version'],['Version 2.01;O365'])

    def test_mac_original_and_pending_measurements_cannot_be_relabelled(self):
        manifest=read(checked_file(ROOT,self.metadata['mac_parent_manifest']))
        self.assertEqual(manifest['reference'],self.metadata['mac_parent_reference'])
        self.assertEqual(manifest['reference']['platform'],'mac')
        self.assertEqual(manifest['reference']['version'],'16.114.3')
        self.assertEqual(manifest['sourceSha256'],'ce26341fd72f4480edc033ee2093d803ad8e1527723255ab0980c760b7e1dff0')
        for row in self.rows:
            self.assertEqual(row['coordinate_gate_promotion'],'aptos-sizes' if row['name'] in gate.REQUIRED_CASE_NAMES else 'pending')
            self.assertEqual(row['screen_gate_promotion'],'aptos-size-screen')
            self.assertEqual(row['input_specification']['mac_parent_sha256'],manifest['sourceSha256'])

if __name__=='__main__':unittest.main()
