"""Reproduce the captured Times New Roman East Asian script-slot controls."""
from pathlib import Path
from io import BytesIO
from zipfile import ZipFile
import hashlib
import json
from lxml import etree as E
from generate_windows_aptos_spacing_probes import changed_parts, xml, q

ROOT=Path(__file__).resolve().parents[2]
PARENTS=Path('tests/word-oracle/windows/legacy-justification-observations.json')
PARENTS_SHA256='2f0cba4117165e39fac724f4232216072263697861aa958072937c3051aabe31'


def generate(root=ROOT):
    registry=(root/PARENTS).read_bytes()
    if hashlib.sha256(registry).hexdigest()!=PARENTS_SHA256:
        raise ValueError('Original Times script-slot parent registry changed')
    parents={r['name']:r for r in json.loads(registry)['fixtures']}
    groups=[('times-ea',[(14,None)], [('Arial','arial'),('Calibri','calibri'),('Times New Roman','times'),('MS Mincho','msmincho')]),
            ('times-ea-style-mode',[(12,'regular'),(15,'regular'),(14,'bold'),(14,'italic'),(14,'bolditalic')],[('Arial','arial'),('Times New Roman','times')])]
    for group,conditions,slots in groups:
        for mode,style in conditions:
            for east,label in slots:
                for balance in (False,True):
                    for ending in ('manual','paragraph'):
                        row=parents[f'word-windows-legacy-font-times-s12-m14-{ending}-w8504']
                        source=(root/row['source']['file']).read_bytes()
                        if hashlib.sha256(source).hexdigest()!=row['source']['sha256']:
                            raise ValueError('Original Times script-slot parent source changed')
                        with ZipFile(BytesIO(source)) as package:
                            document=E.fromstring(package.read('word/document.xml'))
                            settings=E.fromstring(package.read('word/settings.xml'))
                        run,=document.iter(q('r'))
                        properties=run.find(q('rPr'))
                        fonts=properties.find(q('rFonts'))
                        old_east=fonts.get(q('eastAsia'))
                        fonts.set(q('eastAsia'),east)
                        if style is not None:
                            for tag,enabled in [('b','bold' in style),('i','italic' in style)]:
                                node=properties.find(q(tag))
                                if node is None:
                                    node=E.SubElement(properties,q(tag))
                                node.set(q('val'),'1' if enabled else '0')
                            compatibility,=[node for node in settings.iter(q('compatSetting')) if node.get(q('name'))=='compatibilityMode']
                            compatibility.set(q('val'),str(mode))
                        balanced,=settings.iter(q('balanceSingleByteDoubleByteWidth'))
                        balanced.set(q('val'),str(int(balance)))
                        suffix=f'-{style}' if style is not None else ''
                        name=f'word-windows-{group}-{label}{suffix}-s12-m{mode}-balance{int(balance)}-{ending}'
                        facts=dict(derived_parent=row['source'],parent_name=row['name'],font_family='Times New Roman',east_asia_font_family=east,font_size_pt=12,compatibility_mode=mode,balance_single_byte_double_byte_width=balance,line_ending=ending,line_width_twips=8504,text='Ag日本語 ABC Ag日本語',following_text='X' if ending=='manual' else '',expected_non_whitespace_glyphs=14 if ending=='manual' else 13,changed_parts=['word/document.xml','word/settings.xml'])
                        if style is None:
                            facts['parent_east_asia_font_family']=old_east
                        else:
                            facts['font_style']=style
                        yield name,changed_parts(source,{'word/document.xml':xml(document),'word/settings.xml':xml(settings)}),facts
