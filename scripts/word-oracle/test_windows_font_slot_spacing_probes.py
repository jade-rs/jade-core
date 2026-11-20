"""Reject loss of original Word mixed-face inputs or coordinate assertions."""
from copy import deepcopy
import json
from pathlib import Path
import unittest
from unittest.mock import patch
from zipfile import ZipFile
from io import BytesIO
from lxml import etree as E
import check_windows_font_slot_spacing as check
from check_platform_coordinates import ROOT,read_json,pdf_origins
import generate_windows_aptos_spacing_probes as producer
import generate_windows_legacy_mode_balance_probes as legacy_producer
import generate_windows_times_script_slot_probes as slot_producer
import generate_windows_times_japanese_spacing_probes as japanese_producer
import generate_windows_times_line_end_probes as line_end_producer
import generate_windows_legacy_mincho_bold_probes as long_bold_producer
import generate_windows_mincho_context_probes as context_producer
import generate_windows_mincho_font_switch_probes as switch_producer
import generate_windows_mincho_prefix_probes as prefix_producer
import generate_windows_times_balanced_space_probes as space_producer
import generate_windows_times_balanced_space_mode_probes as space_mode_producer
import generate_windows_gdi_bold_justification_probes as gdi_producer
import generate_windows_gdi_bold_natural_probes as natural_producer
import generate_windows_origin_phase_probes as phase_producer
import generate_windows_origin_tie_probes as tie_producer
from itertools import chain
import paragraph_backgrounds

class MixedScriptFontSlots(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.index=read_json(ROOT/check.INDEX);cls.verified=check.verify()
        cls.generated={n:(data,spec) for n,data,spec in chain(producer.generate(),legacy_producer.generate(),slot_producer.generate(),japanese_producer.generate(),line_end_producer.generate(),long_bold_producer.generate(),context_producer.generate(),switch_producer.generate(),prefix_producer.generate(),space_producer.generate(),space_mode_producer.generate(),gdi_producer.generate(),natural_producer.generate(),phase_producer.generate(),tie_producer.generate())}
        cls.pdf_models={};cls.pdf_bytes={}
        for _,document,manifest in cls.verified[4].values():
            path=manifest.parent/document['pdf'];cls.pdf_models[path]=pdf_origins(path);cls.pdf_bytes[path]=path.read_bytes()

    def test_original_sources_reproduce_and_all434_are_required(self):
        cases,required,xps,models,_=self.verified
        self.assertEqual((len(cases),len(required),len(xps),len(models)),(434,434,434,434))
        self.assertEqual(len({r['source']['sha256'] for r in cases}),422)
        self.assertEqual(sorted(r['documents'] for r in self.index['captures']),[4, 8, 8, 8, 8, 10, 16, 16, 16, 16, 20, 24, 32, 32, 32, 40, 48, 96])
        self.assertEqual(sum(r['input_specification']['line_ending']=='manual' for r in cases),202)
        self.assertEqual(sum(r['glyph_count'] for r in cases),5705)
        for r in cases:
            data,spec=self.generated[r['name']]
            self.assertEqual((ROOT/r['source']['file']).read_bytes(),data)
            self.assertEqual(r['input_specification'],spec)
            self.assertEqual(r['font_program_source']['kind'],'original-xps')

    def test_only_documented_parts_fonts_sizes_and_settings_change(self):
        q=producer.q
        for name,(data,spec) in self.generated.items():
            with self.subTest(name=name),ZipFile(ROOT/spec['derived_parent']['file']) as old,ZipFile(BytesIO(data)) as new:
                self.assertEqual(old.namelist(),new.namelist())
                changed={n for n in old.namelist() if old.read(n)!=new.read(n)}
                if name in {'word-windows-times-ea-arial-s12-m14-balance0-manual', 'word-windows-times-ea-arial-s12-m14-balance0-paragraph'}:
                    self.assertEqual(changed,set())
                else:
                    self.assertTrue(changed)
                self.assertLessEqual(changed,set(spec['changed_parts']))
                document=E.fromstring(new.read('word/document.xml'))
                settings=E.fromstring(new.read('word/settings.xml'))
                mode,=[n for n in settings.iter(q('compatSetting')) if n.get(q('name'))=='compatibilityMode']
                self.assertEqual(int(mode.get(q('val'))),spec['compatibility_mode'])
                if 'balance_single_byte_double_byte_width' in spec:
                    balanced,=settings.iter(q('balanceSingleByteDoubleByteWidth'))
                    self.assertEqual(balanced.get(q('val'),'1') not in ('0','false','off'),spec['balance_single_byte_double_byte_width'])
                else:
                    old_settings=E.fromstring(old.read('word/settings.xml'))
                    self.assertEqual([E.tostring(n) for n in settings.iter(q('balanceSingleByteDoubleByteWidth'))],
                                     [E.tostring(n) for n in old_settings.iter(q('balanceSingleByteDoubleByteWidth'))])
                text=''.join(n.text or '' for n in document.iter(q('t')))
                self.assertEqual(text,spec.get('text',spec.get('left_text','')+spec.get('right_text',''))+spec.get('following_text',''))
                self.assertEqual(len(''.join(text.split())),spec['expected_non_whitespace_glyphs'])
                self.assertEqual(bool(list(document.iter(q('br')))),spec['line_ending']=='manual')
                if 'alignment' in spec:
                    node,=document.iter(q('jc'));self.assertEqual(node.get(q('val')),spec['alignment'])
                if 'use_fe_layout' in spec:
                    node,=settings.iter(q('useFELayout'));self.assertEqual(node.get(q('val'),'1') not in ('0','false','off'),spec['use_fe_layout'])
                if 'page_left_margin_twips' in spec:
                    node,=document.iter(q('pgMar'))
                    self.assertEqual(int(node.get(q('left'))),spec['page_left_margin_twips'])
                    self.assertEqual(int(node.get(q('right'))),spec['page_right_margin_twips'])
                    size,=document.iter(q('pgSz'))
                    self.assertEqual(int(size.get(q('w')))-int(node.get(q('left')))-int(node.get(q('right'))),spec['line_width_twips'])
                runs=list(document.iter(q('r')))
                if 'left_font_family' in spec:
                    self.assertEqual(len(runs),2)
                    for run,side in zip(runs,('left','right')):
                        properties=run.find(q('rPr'));fonts=properties.find(q('rFonts'))
                        for slot in ('ascii','hAnsi','cs'):
                            self.assertEqual(fonts.get(q(slot)),spec[side+'_font_family'])
                        if side+'_font_size_pt' in spec:
                            for tag in ('sz','szCs'):
                                self.assertEqual(int(properties.find(q(tag)).get(q('val'))),2*spec[side+'_font_size_pt'])
                        self.assertEqual(next(run.iter(q('t'))).text,spec[side+'_text'])
                elif 'east_asia_font_family' in spec:
                    old_document=E.fromstring(old.read('word/document.xml'))
                    self.assertEqual(len(runs),len(list(old_document.iter(q('r')))))
                    for run in runs:
                        properties=run.find(q('rPr'));fonts=properties.find(q('rFonts'))
                        for slot in ('ascii','hAnsi'):
                            self.assertEqual(fonts.get(q(slot)),spec['font_family'])
                        self.assertEqual(fonts.get(q('eastAsia')),spec['east_asia_font_family'])
                        for tag in ('sz','szCs'):
                            self.assertEqual(int(properties.find(q(tag)).get(q('val'))),2*spec['font_size_pt'])
                        style=spec.get('font_style','regular')
                        for tag,enabled in [('b','bold' in style),('i','italic' in style)]:
                            node=properties.find(q(tag))
                            actual=node is not None and node.get(q('val'),'1') not in ('0','false','off')
                            self.assertEqual(actual,enabled)
                elif 'style' in spec:
                    run,=runs;properties=run.find(q('rPr'))
                    for tag,enabled in (('b',spec['bold']),('bCs',spec['bold']),('i',spec['italic']),('iCs',spec['italic'])):
                        self.assertEqual(properties.find(q(tag)) is not None,enabled)
                    for slot in ('ascii','hAnsi','cs'):
                        self.assertEqual(properties.find(q('rFonts')).get(q(slot)),spec['font_family'])
                else:
                    self.assertEqual(old.read('word/document.xml'),new.read('word/document.xml'))

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
        for mutate in (lambda i:i['word'].update(product_version='16.0.0.0'),lambda i:i['word'].update(executable_sha256='0'*64),lambda i:i['fixtures'][0]['source'].update(sha256='0'*64),lambda i:i['fixtures'][0]['font_program_source'].update(sha256='0'*64),lambda i:i['fixtures'][0]['input_specification'].update(compatibility_mode=99)):
            index=deepcopy(self.index);mutate(index)
            with patch.object(check,'read_json',side_effect=lambda p:index if p==ROOT/check.INDEX else original(p)),self.assertRaises(ValueError):check.verify()

    def pipeline(self,mutation=None,name=None):
        cases,_,_,expected,documents=self.verified
        if name is not None:cases=[r for r in cases if r['name']==name]
        models={(c['name'],plane):dict(deepcopy(expected[c['name']]),input_sha256=c['source']['sha256']) for c in cases for plane in ('native','visible')}
        physical={name:deepcopy(self.pdf_models[manifest.parent/document['pdf']]) for name,(_,document,manifest) in documents.items() if any(r['name']==name for r in cases)}
        if mutation:mutation(models,physical)
        real=Path.read_bytes
        def data(path):
            if path.name=='native.pdf':return b'Independent Jade PDF'
            return self.pdf_bytes[path] if path in self.pdf_bytes else real(path)
        def pdf(path,unclipped=False):return physical[path.parent.name] if path.name=='native.pdf' else self.pdf_models[path]
        with patch.object(check,'verify',return_value=self.verified),patch.object(check,'read_json',side_effect=lambda p:models[(p.parent.name,p.stem)]),patch.object(check,'pdf_origins',side_effect=pdf),patch.object(Path,'read_bytes',new=data),patch.object(paragraph_backgrounds,'compare_paragraph_backgrounds') as background:
            if name is not None:
                case,=cases;_,document,manifest=documents[name]
                result=check.assert_rendered_case(case,Path('rendered'),expected[name],document,manifest,ROOT)
            else:result=check.check_rendered_fonts(Path('rendered'),ROOT)
            self.assertEqual(background.call_count,len(cases));return result

    def test_all434_reach_both_xps_and_background_checks(self):
        rows=self.pipeline();self.assertEqual(len(rows),434)
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
                    with self.subTest(name=name,plane=plane,kind=kind),self.assertRaises(ValueError):self.pipeline(mutate,name)
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
                with self.subTest(name=name,physical=kind),self.assertRaises(ValueError):self.pipeline(mutate,name)

if __name__=='__main__':unittest.main()
