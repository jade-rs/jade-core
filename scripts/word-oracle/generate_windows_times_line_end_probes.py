"""Source-only Word controls for terminal CJK bearings in justified Times runs."""
from pathlib import Path
from io import BytesIO
from zipfile import ZipFile,ZipInfo,ZIP_DEFLATED
import hashlib,json
from lxml import etree as E

ROOT=Path(__file__).resolve().parents[2]
PARENTS=Path('tests/word-oracle/windows/legacy-justification-observations.json')
PARENTS_SHA256='2f0cba4117165e39fac724f4232216072263697861aa958072937c3051aabe31'
q=lambda n:'{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'+n


def generate(root=ROOT):
    registry=(root/PARENTS).read_bytes()
    if hashlib.sha256(registry).hexdigest()!=PARENTS_SHA256:
        raise ValueError('Original Times line-end parent registry changed')
    parents={r['name']:r for r in json.loads(registry)['fixtures']}
    for bold in (False,True):
        for last,label in [('日','day'),('本','book')]:
            for ending in ('manual','paragraph'):
                row=parents[f'word-windows-legacy-font-times-s12-m14-{ending}-w8504']
                source=(root/row['source']['file']).read_bytes()
                if hashlib.sha256(source).hexdigest()!=row['source']['sha256']:
                    raise ValueError('Original Times line-end parent source changed')
                with ZipFile(BytesIO(source)) as package:
                    document=E.fromstring(package.read('word/document.xml'))
                    settings=E.fromstring(package.read('word/settings.xml'))
                    run,=document.iter(q('r'));props=run.find(q('rPr'))
                    props.find(q('rFonts')).set(q('eastAsia'),'MS Mincho')
                    for tag,enabled in [('b',bold),('i',False)]:
                        node=props.find(q(tag))
                        if node is None:node=E.SubElement(props,q(tag))
                        node.set(q('val'),'1' if enabled else '0')
                    text='Ag日本語 ABC Ag日本'+last
                    next(run.iter(q('t'))).text=text
                    balance,=settings.iter(q('balanceSingleByteDoubleByteWidth'));balance.set(q('val'),'1')
                    parts={n:E.tostring(x,encoding='utf-8',xml_declaration=True,standalone=True) for n,x in [('word/document.xml',document),('word/settings.xml',settings)]}
                    output=BytesIO()
                    with ZipFile(output,'w',compression=ZIP_DEFLATED) as target:
                        for item in package.infolist():
                            info=ZipInfo(item.filename,(2026,11,19,0,0,0));info.create_system=0;info.compress_type=ZIP_DEFLATED;info.external_attr=item.external_attr
                            target.writestr(info,parts.get(item.filename,package.read(item.filename)))
                style='bold' if bold else 'regular'
                name=f'word-windows-times-line-end-{label}-{style}-s12-m14-balance1-{ending}'
                facts=dict(derived_parent=row['source'],parent_name=row['name'],font_family='Times New Roman',east_asia_font_family='MS Mincho',font_style=style,font_size_pt=12,compatibility_mode=14,balance_single_byte_double_byte_width=True,line_ending=ending,line_width_twips=8504,text=text,following_text='X' if ending=='manual' else '',expected_non_whitespace_glyphs=14 if ending=='manual' else 13,changed_parts=list(parts),ending_east_asia_character=last)
                yield name,output.getvalue(),facts
