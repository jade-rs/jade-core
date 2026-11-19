"""Reject loss of original Word controls or required/pending render assertions."""
from copy import deepcopy
from io import BytesIO
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from zipfile import ZipFile

from lxml import etree as E

import check_windows_legacy_justification as check
import check_windows_font_coordinates as fonts
import check_platform_coordinates as platform
from check_platform_coordinates import ROOT,read_json,pdf_origins
import generate_windows_legacy_justification_probes as producer
import paragraph_backgrounds


class LegacyJustification(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.index=read_json(ROOT/check.INDEX)
        cls.verified=check.verify()
        cls.generated={n:(data,spec) for n,data,spec in producer.generate()}
        cls.pdf_models={}
        cls.pdf_bytes={}
        for _,document,manifest in cls.verified[4].values():
            path=manifest.parent/document['pdf']
            cls.pdf_models[path]=pdf_origins(path)
            cls.pdf_bytes[path]=path.read_bytes()

    def test_all_eighty_originals_reproduce_with_locked_promotions(self):
        cases,required,xps,models,_=self.verified
        self.assertEqual((len(cases),len(required),len(xps),len(models)),(80,68,66,80))
        self.assertEqual(sum(c['input_specification'].get('font_family') is None for c in cases),32)
        for case in cases:
            data,spec=self.generated[case['name']]
            self.assertEqual((ROOT/case['source']['file']).read_bytes(),data)
            self.assertEqual(case['input_specification'],spec)
        self.assertEqual(sum(c.get('pending_font_reason')=='word-pdf-subset-missing-kerning' for c in cases),8)
        self.assertEqual(sum(c.get('pending_font_reason')=='windows-modern-mixed-justification' for c in cases),4)

    def test_only_indentation_and_selected_font_text_properties_change(self):
        q=producer.q
        for name,(data,spec) in self.generated.items():
            with self.subTest(name=name),ZipFile(ROOT/spec['derived_parent']['file']) as old,ZipFile(BytesIO(data)) as new:
                self.assertEqual(old.namelist(),new.namelist())
                for member in old.namelist():
                    if member!='word/document.xml':self.assertEqual(old.read(member),new.read(member))
                before,after=[E.fromstring(z.read('word/document.xml')) for z in (old,new)]
                properties=after.find('.//'+q('pPr'));indent=properties.find(q('ind'))
                self.assertEqual(int(indent.get(q('right'))),8504-spec['line_width_twips'])
                properties.remove(indent)
                if 'font_family' in spec:
                    oldrun,=before.iter(q('r'));run,=after.iter(q('r'))
                    oldfonts=oldrun.find(q('rPr')).find(q('rFonts'));fonts=run.find(q('rPr')).find(q('rFonts'))
                    for slot in ('ascii','hAnsi','cs'):
                        self.assertEqual(fonts.get(q(slot)),spec['font_family'])
                        fonts.set(q(slot),oldfonts.get(q(slot)))
                    if spec['bold']:
                        for tag in ('b','bCs'):run.find(q('rPr')).remove(run.find(q('rPr')).find(q(tag)))
                    next(run.iter(q('t'))).text=next(oldrun.iter(q('t'))).text
                self.assertEqual(E.tostring(before),E.tostring(after))

    def test_required_promotions_cannot_be_removed_or_swapped(self):
        for field,promotion in [('font_gate_promotion','legacy-justify-font'),('xps_gate_promotion','legacy-justify-xps')]:
            for operation in ('remove','swap','unknown'):
                index=deepcopy(self.index)
                required=next(c for c in index['fixtures'] if c[field]==promotion)
                pending=next(c for c in index['fixtures'] if c[field]=='pending')
                required[field]='pending' if operation!='unknown' else 'unknown'
                if operation=='swap':pending[field]=promotion
                with self.subTest(field=field,operation=operation),self.assertRaises(ValueError):check.locked_promotions(index)

    def test_wrong_word_identity_source_or_pending_classification_is_rejected(self):
        original=check.read_json
        for edit in (lambda i:i['word'].update(product_version='16.0.0.0'),
                     lambda i:i['word'].update(executable_sha256='0'*64),
                     lambda i:i['fixtures'][0]['source'].update(sha256='0'*64),
                     lambda i:next(c for c in i['fixtures'] if c.get('pending_font_reason')=='word-pdf-subset-missing-kerning').update(pending_font_reason='windows-modern-mixed-justification')):
            index=deepcopy(self.index);edit(index)
            with patch.object(check,'read_json',side_effect=lambda p:index if p==ROOT/check.INDEX else original(p)),self.assertRaises(ValueError):check.verify()

    def test_pending_renderer_reaches_all_twelve_sources_and_both_paths(self):
        cases,required,_,_,documents=self.verified
        calls=[]
        def run(args,**options):
            self.assertTrue(options['check'])
            self.assertEqual(args[3],'windows')
            self.assertIn('--fonts-directory',args)
            destination=Path(args[2]);plane=destination.stem
            self.assertEqual('--unclipped-tables' in args,plane=='native')
            calls.append((destination.parent.name,plane))
            destination.write_bytes(json.dumps(dict(fonts=[])).encode())
        def extract(original,target):target.mkdir()
        with tempfile.TemporaryDirectory() as temporary,patch.object(platform,'extract_fonts',side_effect=extract),patch.object(check.subprocess,'run',side_effect=run),patch.object(fonts,'require_original_font_programs_registered') as registered:
            check.render_pending(Path('renderer'),Path(temporary),ROOT,cases,required,documents)
        expected={(c['name'],plane) for c in cases if c['name'] not in required for plane in ('native','visible')}
        self.assertEqual(set(calls),expected)
        self.assertEqual((len(calls),registered.call_count),(24,24))

    def pipeline(self,mutation=None):
        cases,_,_,expected,documents=self.verified
        models={(c['name'],plane):dict(deepcopy(expected[c['name']]),input_sha256=c['source']['sha256']) for c in cases for plane in ('native','visible')}
        physical={name:deepcopy(self.pdf_models[manifest.parent/document['pdf']]) for name,(_,document,manifest) in documents.items()}
        if mutation:mutation(models,physical)
        original_bytes=Path.read_bytes
        def data(path):
            if path.name=='native.pdf':return b'Independent Jade PDF'
            return self.pdf_bytes[path] if path in self.pdf_bytes else original_bytes(path)
        def rendered(path):return models[(path.parent.name,path.stem)]
        def pdf(path,unclipped=False):return physical[path.parent.name] if path.name=='native.pdf' else self.pdf_models[path]
        with patch.object(check,'verify',return_value=self.verified),patch.object(check,'render_pending') as pending,patch.object(check,'read_json',side_effect=rendered),patch.object(check,'pdf_origins',side_effect=pdf),patch.object(Path,'read_bytes',new=data),patch.object(paragraph_backgrounds,'compare_paragraph_backgrounds') as background:
            result=check.check_rendered_fonts(Path('rendered'),ROOT,Path('renderer'))
            pending.assert_called_once()
            self.assertEqual(background.call_count,80)
            return result

    def test_all_eighty_reach_text_and_background_checks_and_required_xps_paths(self):
        rows=self.pipeline()
        self.assertEqual(len(rows),80)
        self.assertEqual(sum(r['required_font'] for r in rows),68)
        self.assertEqual(sum(r['required_xps'] for r in rows),66)
        self.assertTrue(all(set(r['planes'])=={'native','visible'} for r in rows if r['required_xps']))

    def test_each_required_xps_case_rejects_both_axis_displacements_on_both_paths(self):
        for name in self.verified[2]:
            for plane in ('native','visible'):
                for axis in (2,3):
                    def mutation(models,physical):models[(name,plane)]['glyphs'][0][axis]+=.5
                    with self.subTest(name=name,plane=plane,axis=axis),self.assertRaises(ValueError):self.pipeline(mutation)

    def test_each_pending_case_rejects_missing_glyphs_wrong_input_pages_and_nonfinite_axes(self):
        pending=[c['name'] for c in self.verified[0] if c['name'] not in self.verified[1]]
        for name in pending:
            for plane in ('native','visible'):
                for kind in ('missing','input','page','x','y'):
                    def mutation(models,physical):
                        model=models[(name,plane)]
                        if kind=='missing':model['glyphs'].pop()
                        elif kind=='input':model['input_sha256']='0'*64
                        elif kind=='page':model['page_count']+=1
                        else:model['glyphs'][0][2 if kind=='x' else 3]=float('nan')
                    with self.subTest(name=name,plane=plane,kind=kind),self.assertRaises(ValueError):self.pipeline(mutation)
            for kind in ('blank','order','page','finite'):
                def mutation(models,physical):
                    model=physical[name]
                    if kind=='blank':model['glyphs']=[]
                    elif kind=='order':model['glyphs'][0],model['glyphs'][2]=model['glyphs'][2],model['glyphs'][0]
                    elif kind=='page':model['page_count']+=1
                    else:model['glyphs'][0][2]=float('inf')
                with self.subTest(name=name,physical=kind),self.assertRaises(ValueError):self.pipeline(mutation)


if __name__=='__main__':unittest.main()
