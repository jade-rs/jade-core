"""Reject loss of original Word space-pair inputs or coordinate assertions."""
from copy import deepcopy
import json
from pathlib import Path
import unittest
from unittest.mock import patch
from zipfile import ZipFile
from io import BytesIO
from lxml import etree as E
import check_windows_space_pair as check
from check_platform_coordinates import ROOT,read_json,pdf_origins
import generate_windows_space_pair_probes as producer
import paragraph_backgrounds

class SpacePairKerning(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.index=read_json(ROOT/check.INDEX);cls.verified=check.verify()
        cls.generated={n:(data,spec) for n,data,spec in producer.generate()}
        cls.pdf_models={};cls.pdf_bytes={}
        for _,document,manifest in cls.verified[4].values():
            path=manifest.parent/document['pdf'];cls.pdf_models[path]=pdf_origins(path);cls.pdf_bytes[path]=path.read_bytes()

    def test_original_sources_reproduce_and_all32_are_required(self):
        cases,required,xps,models,_=self.verified
        self.assertEqual((len(cases),len(required),len(xps),len(models)),(32,32,32,32))
        self.assertEqual(len({(r['input_specification']['font_family'],r['input_specification']['compatibility_mode'],r['input_specification']['kerning_half_points'],r['input_specification']['text']) for r in cases}),32)
        for r in cases:
            data,spec=self.generated[r['name']]
            self.assertEqual((ROOT/r['source']['file']).read_bytes(),data)
            self.assertEqual(r['input_specification'],spec)
            self.assertEqual(r['font_program_source']['kind'],'original-xps')

    def test_only_documented_font_kerning_and_text_properties_change(self):
        q=producer.q
        for name,(data,spec) in self.generated.items():
            with self.subTest(name=name),ZipFile(ROOT/spec['derived_parent']['file']) as old,ZipFile(BytesIO(data)) as new:
                self.assertEqual(old.namelist(),new.namelist())
                for member in old.namelist():
                    if member!='word/document.xml':self.assertEqual(old.read(member),new.read(member))
                before,after=[E.fromstring(z.read('word/document.xml')) for z in (old,new)]
                oldrun,=before.iter(q('r'));run,=after.iter(q('r'));properties=run.find(q('rPr'))
                fonts=properties.find(q('rFonts'));oldfonts=oldrun.find(q('rPr')).find(q('rFonts'))
                for slot in ('ascii','hAnsi','cs'):
                    self.assertEqual(fonts.get(q(slot)),spec['font_family']);fonts.set(q(slot),oldfonts.get(q(slot)))
                setting=properties.find(q('kern'));self.assertEqual(setting.get(q('val')),str(spec['kerning_half_points']));properties.remove(setting)
                self.assertEqual(next(run.iter(q('t'))).text,spec['text']);next(run.iter(q('t'))).text=next(oldrun.iter(q('t'))).text
                self.assertEqual(E.tostring(before),E.tostring(after))

    def test_required_promotions_cannot_be_removed_renamed_or_duplicated(self):
        for field in ('font_gate_promotion','xps_gate_promotion'):
            for kind in ('remove','unknown','rename','duplicate'):
                index=deepcopy(self.index)
                if kind=='remove':index['fixtures'][0][field]='pending'
                elif kind=='unknown':index['fixtures'][0][field]='unknown'
                elif kind=='rename':index['fixtures'][0]['name']+='-changed'
                else:index['fixtures'].append(deepcopy(index['fixtures'][0]))
                with self.subTest(field=field,kind=kind),self.assertRaises(ValueError):check.locked_promotions(index)

    def test_wrong_word_source_and_original_font_program_are_rejected(self):
        original=check.read_json
        for mutate in (lambda i:i['word'].update(product_version='16.0.0.0'),lambda i:i['word'].update(executable_sha256='0'*64),lambda i:i['fixtures'][0]['source'].update(sha256='0'*64),lambda i:i['fixtures'][0]['font_program_source'].update(sha256='0'*64)):
            index=deepcopy(self.index);mutate(index)
            with patch.object(check,'read_json',side_effect=lambda p:index if p==ROOT/check.INDEX else original(p)),self.assertRaises(ValueError):check.verify()

    def pipeline(self,mutation=None):
        cases,_,_,expected,documents=self.verified
        models={(c['name'],plane):dict(deepcopy(expected[c['name']]),input_sha256=c['source']['sha256']) for c in cases for plane in ('native','visible')}
        physical={name:deepcopy(self.pdf_models[manifest.parent/document['pdf']]) for name,(_,document,manifest) in documents.items()}
        if mutation:mutation(models,physical)
        real=Path.read_bytes
        def data(path):
            if path.name=='native.pdf':return b'Independent Jade PDF'
            return self.pdf_bytes[path] if path in self.pdf_bytes else real(path)
        def pdf(path,unclipped=False):return physical[path.parent.name] if path.name=='native.pdf' else self.pdf_models[path]
        with patch.object(check,'verify',return_value=self.verified),patch.object(check,'read_json',side_effect=lambda p:models[(p.parent.name,p.stem)]),patch.object(check,'pdf_origins',side_effect=pdf),patch.object(Path,'read_bytes',new=data),patch.object(paragraph_backgrounds,'compare_paragraph_backgrounds') as background:
            result=check.check_rendered_fonts(Path('rendered'),ROOT);self.assertEqual(background.call_count,32);return result

    def test_all32_reach_both_xps_and_background_checks(self):
        rows=self.pipeline();self.assertEqual(len(rows),32)
        self.assertTrue(all(r['required_font'] and r['required_xps'] and set(r['planes'])=={'native','visible'} for r in rows))

    def test_each_case_rejects_both_axis_drift_and_structural_corruption(self):
        for row in self.verified[0]:
            name=row['name']
            for plane in ('native','visible'):
                for kind in ('x','y','missing','input','page','nonfinite'):
                    def mutate(models,physical):
                        model=models[(name,plane)]
                        if kind in ('x','y'):model['glyphs'][0][2 if kind=='x' else 3]+=.5
                        elif kind=='missing':model['glyphs'].pop()
                        elif kind=='input':model['input_sha256']='0'*64
                        elif kind=='page':model['page_count']+=1
                        else:model['glyphs'][0][2]=float('nan')
                    with self.subTest(name=name,plane=plane,kind=kind),self.assertRaises(ValueError):self.pipeline(mutate)
            for kind in ('blank','text','order','page','nonfinite'):
                if kind == 'order' and name.endswith('-aa-aa'):
                    continue  # Identical letters have no distinguishable text-order swap.
                def mutate(models,physical):
                    model=physical[name]
                    if kind=='blank':model['glyphs']=[]
                    elif kind=='text':model['glyphs'][0][1]='?'
                    elif kind=='order':model['glyphs'][0],model['glyphs'][2]=model['glyphs'][2],model['glyphs'][0]
                    elif kind=='page':model['page_count']+=1
                    else:model['glyphs'][0][2]=float('inf')
                with self.subTest(name=name,physical=kind),self.assertRaises(ValueError):self.pipeline(mutate)

if __name__=='__main__':unittest.main()
