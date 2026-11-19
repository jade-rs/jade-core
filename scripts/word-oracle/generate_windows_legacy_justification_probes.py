"""Reproduce width/font controls from versioned manual-break Word originals."""
import hashlib
from io import BytesIO
import json
from pathlib import Path
from zipfile import ZipFile,ZipInfo,ZIP_DEFLATED

from lxml import etree as E

ROOT=Path(__file__).resolve().parents[2]
PARENTS=Path('tests/word-oracle/windows/hard-break-space-observations.json')
PARENTS_SHA256='2b3e8486b3de57fbb1432c60912c96c2d12e12d3c4af9172ca28784ed0b2bbc9'
WIDTHS=(6900,7000,7100,7200,7300,7700,8100,8504)
FONT_WIDTHS=(3000,3400,4000,8504)
FONTS=(('Arial',True,'arial-bold'),('Calibri',False,'calibri'),('Times New Roman',False,'times'))
q=lambda n:'{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'+n


def changed_document(data,xml):
    result=BytesIO()
    with ZipFile(BytesIO(data)) as source, ZipFile(result,'w',compression=ZIP_DEFLATED) as target:
        for info in source.infolist():
            item=ZipInfo(info.filename,(2026,11,19,0,0,0))
            item.create_system=0;item.compress_type=ZIP_DEFLATED;item.external_attr=info.external_attr
            target.writestr(item,xml if info.filename=='word/document.xml' else source.read(info.filename))
    return result.getvalue()


def generate(root=ROOT):
    data=(root/PARENTS).read_bytes()
    if hashlib.sha256(data).hexdigest()!=PARENTS_SHA256:
        raise ValueError('Original manual-break parent registry changed')
    parents={c['name']:c for c in json.loads(data)['fixtures']}

    def parent(mode,ending):
        row=parents[f'word-windows-hard-break-ea-arial-s12-m{mode}-{ending}-spaces0']
        source=(root/row['source']['file']).read_bytes()
        if hashlib.sha256(source).hexdigest()!=row['source']['sha256']:
            raise ValueError('Original manual-break parent source changed')
        with ZipFile(BytesIO(source)) as package:
            document=E.fromstring(package.read('word/document.xml'))
        return row,source,document

    def indented(document,width):
        paragraph,=document.iter(q('p'));properties=paragraph.find(q('pPr'))
        if properties is None or properties.find(q('ind')) is not None:
            raise ValueError('Original paragraph indentation changed')
        section,=document.iter(q('sectPr'));page=section.find(q('pgSz'));margins=section.find(q('pgMar'))
        body_width=int(page.get(q('w')))-int(margins.get(q('left')))-int(margins.get(q('right')))
        if body_width!=8504:
            raise ValueError('Original body width changed')
        indent=E.Element(q('ind'));indent.set(q('right'),str(body_width-width));properties.insert(1,indent)
        return paragraph

    for mode in (14,15):
        for ending in ('manual','paragraph'):
            for width in WIDTHS:
                row,source,document=parent(mode,ending)
                indented(document,width)
                xml=E.tostring(document,encoding='utf-8',xml_declaration=True,standalone=True)
                name=f'word-windows-legacy-width-arial-s12-m{mode}-{ending}-w{width}'
                spec=dict(derived_parent=row['source'],parent_name=row['name'],compatibility_mode=mode,
                    line_ending=ending,line_width_twips=width,right_indent_twips=8504-width,
                    text=row['input_specification']['text'],following_text=row['input_specification']['following_text'],
                    expected_non_whitespace_glyphs=row['input_specification']['expected_non_whitespace_glyphs'],
                    changed_parts=['word/document.xml'],scope='Only right indent changes; source text, fonts, settings and other package parts are byte-preserved')
                yield name,changed_document(source,xml),spec
    for family,bold,label in FONTS:
        for mode in (14,15):
            for ending in ('manual','paragraph'):
                for width in FONT_WIDTHS:
                    row,source,document=parent(mode,ending)
                    paragraph=indented(document,width)
                    run,=paragraph.iter(q('r'));properties=run.find(q('rPr'));fonts=properties.find(q('rFonts'))
                    for slot in ('ascii','hAnsi','cs'):
                        fonts.set(q(slot),family)
                    if bold:
                        properties.insert(1,E.Element(q('b')));properties.insert(2,E.Element(q('bCs')))
                    text='Ag日本語 ABC Ag日本語';next(run.iter(q('t'))).text=text
                    xml=E.tostring(document,encoding='utf-8',xml_declaration=True,standalone=True)
                    name=f'word-windows-legacy-font-{label}-s12-m{mode}-{ending}-w{width}'
                    spec=dict(derived_parent=row['source'],parent_name=row['name'],font_family=family,bold=bold,
                        font_size_pt=12,east_asia_font='Arial',compatibility_mode=mode,line_ending=ending,
                        line_width_twips=width,right_indent_twips=8504-width,text=text,
                        following_text='X' if ending=='manual' else '',
                        expected_non_whitespace_glyphs=sum(not c.isspace() for c in text)+(ending=='manual'),
                        changed_parts=['word/document.xml'])
                    yield name,changed_document(source,xml),spec
