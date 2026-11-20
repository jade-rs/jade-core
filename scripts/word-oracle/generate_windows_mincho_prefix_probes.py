"""Isolate leading Latin and terminal context in original Word bold CJK."""
from pathlib import Path
from io import BytesIO
from zipfile import ZipFile,ZipInfo,ZIP_DEFLATED
import hashlib,json
from lxml import etree as E
ROOT=Path(__file__).resolve().parents[2]
REGISTRY=Path('tests/word-oracle/windows/legacy-justification-observations.json')
REGISTRY_SHA256='2f0cba4117165e39fac724f4232216072263697861aa958072937c3051aabe31'
q=lambda n:'{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'+n


def generate(root=ROOT):
    registry=(root/REGISTRY).read_bytes()
    if hashlib.sha256(registry).hexdigest()!=REGISTRY_SHA256:raise ValueError('Original Mincho-context parent registry changed')
    row=next(r for r in json.loads(registry)['fixtures'] if r['name']=='word-windows-legacy-font-times-s12-m14-paragraph-w8504')
    source=(root/row['source']['file']).read_bytes()
    if hashlib.sha256(source).hexdigest()!=row['source']['sha256']:raise ValueError('Original Mincho-context parent source changed')
    variants=[('a-interior','A日本語 A'),('ag-interior','Ag日本語 A'),('g-interior','g日本語 A'),('a-space-interior','A 日本語 A'),('jp-latin-jp','日本語 ABC 日本語'),('ag-terminal','Ag日本語'),('ag-long-interior','Ag日本語日本語 A'),('ag-space-terminal','Ag 日本語')]
    for label,text in variants:
        language=False;left=False;fe=True
        with ZipFile(BytesIO(source)) as package:
            document=E.fromstring(package.read('word/document.xml'));settings=E.fromstring(package.read('word/settings.xml'))
            paragraph,=document.iter(q('p'));paragraph.find(q('pPr')).find(q('jc')).set(q('val'),'left' if left else 'both')
            run,=document.iter(q('r'));props=run.find(q('rPr'));props.find(q('rFonts')).set(q('eastAsia'),'MS Mincho')
            for tag,enabled in [('b',True),('i',False)]:
                node=props.find(q(tag))
                if node is None:node=E.SubElement(props,q(tag))
                node.set(q('val'),'1' if enabled else '0')
            if language:
                node=props.find(q('lang'))
                if node is None:node=E.SubElement(props,q('lang'))
                node.set(q('val'),'en-US');node.set(q('eastAsia'),'ja-JP')
            next(run.iter(q('t'))).text=text
            balance,=settings.iter(q('balanceSingleByteDoubleByteWidth'));balance.set(q('val'),'1')
            use_fe,=settings.iter(q('useFELayout'));use_fe.set(q('val'),'1' if fe else '0')
            parts={name:E.tostring(x,encoding='utf-8',xml_declaration=True,standalone=True) for name,x in [('word/document.xml',document),('word/settings.xml',settings)]}
            output=BytesIO()
            with ZipFile(output,'w',compression=ZIP_DEFLATED) as target:
                for item in package.infolist():
                    info=ZipInfo(item.filename,(2026,11,19,0,0,0));info.create_system=0;info.compress_type=ZIP_DEFLATED;info.external_attr=item.external_attr
                    target.writestr(info,parts.get(item.filename,package.read(item.filename)))
        name=f'word-windows-mincho-prefix-{label}-s12-m14'
        facts=dict(derived_parent=row['source'],parent_name=row['name'],font_family='Times New Roman',east_asia_font_family='MS Mincho',font_style='bold',font_size_pt=12,compatibility_mode=14,balance_single_byte_double_byte_width=True,line_ending='paragraph',line_width_twips=8504,text=text,following_text='',expected_non_whitespace_glyphs=len(text.replace(' ','')),changed_parts=list(parts),alignment='left' if left else 'both',explicit_japanese_language=language,use_fe_layout=fe)
        yield name,output.getvalue(),facts
