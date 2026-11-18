"""Reproduce28 mixed source-form controls from the unchanged Mac original."""
from pathlib import Path
from io import BytesIO
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED
from copy import deepcopy
from lxml import etree as ET
import hashlib

ROOT=Path(__file__).resolve().parents[2]
PARENT=Path('tests/word-oracle/mac-elsa-synthetic/explicit-hyphen-wrap-boundaries-mac/source.docx')
PARENT_SHA256='64e01a358c7c8c12a61d9b50068785afd05e4f3875dc199e5bdde5f3bb322f15'
q=lambda name:'{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'+name
PATTERNS={
    'literal-optional':[[('t','electro\u00admag'),('softHyphen',None),('t','netic')]],
    'optional-literal':[[('t','electro'),('softHyphen',None),('t','mag\u00adnetic')]],
    'separated':[[('t','electro\u00admagnetic re'),('softHyphen',None),('t','enter')]],
    'unicode-prefix':[[('t','ıelectro\u00admag'),('softHyphen',None),('t','netic')]],
    'split-runs':[[('t','electro\u00ad')],[('t','mag'),('softHyphen',None)],[('t','netic')]],
    'break-literals':[[('t','electro\u00admag'),('br',None),('t','netic re\u00adenter')]],
    'break-mixed':[[('t','electro'),('softHyphen',None),('t','mag'),('br',None),('t','netic\u00adreenter')]],
}


def generate(parent=None):
    parent=(ROOT/PARENT).read_bytes() if parent is None else parent
    if hashlib.sha256(parent).hexdigest()!=PARENT_SHA256:
        raise ValueError('Original Mac hyphen source changed')
    with ZipFile(BytesIO(parent)) as original:
        for pattern,tokens in PATTERNS.items():
            for width in (45,65):
                for mode in (14,15):
                    document=ET.fromstring(original.read('word/document.xml'))
                    settings=ET.fromstring(original.read('word/settings.xml'))
                    body=document.find(q('body'));paragraph=deepcopy(body.findall(q('p'))[1]);section=body.find(q('sectPr'))
                    template=deepcopy(paragraph.find(q('r')))
                    for child in list(paragraph):
                        if child.tag==q('r'):paragraph.remove(child)
                    for entries in tokens:
                        run=deepcopy(template)
                        for child in list(run):
                            if child.tag!=q('rPr'):run.remove(child)
                        for tag,value in entries:
                            node=ET.SubElement(run,q(tag));node.text=value
                            if tag=='t':node.set('{http://www.w3.org/XML/1998/namespace}space','preserve')
                        paragraph.append(run)
                    page,margins=section.find(q('pgSz')),section.find(q('pgMar'))
                    available=int(page.get(q('w')))-int(margins.get(q('left')))-int(margins.get(q('right')))
                    paragraph.find(q('pPr')+'/'+q('ind')).set(q('right'),str(available-width*20))
                    for child in list(body):
                        if child is not section:body.remove(child)
                    body.insert(0,paragraph)
                    compatibility=[e for e in settings.iter(q('compatSetting')) if e.get(q('name'))=='compatibilityMode']
                    assert len(compatibility)==1;compatibility[0].set(q('val'),str(mode))
                    changes={'word/document.xml':ET.tostring(document,encoding='utf-8',xml_declaration=True,standalone=True),
                             'word/settings.xml':ET.tostring(settings,encoding='utf-8',xml_declaration=True,standalone=True)}
                    data=BytesIO()
                    with ZipFile(data,'w',compression=ZIP_DEFLATED) as output:
                        for entry in original.infolist():
                            item=ZipInfo(entry.filename,(2026,11,18,0,0,0));item.create_system=0;item.compress_type=ZIP_DEFLATED;item.external_attr=entry.external_attr
                            output.writestr(item,changes.get(entry.filename,original.read(entry.filename)))
                    name=f'word-windows-hyphen-mixed-{pattern}-w{width}-m{mode}'
                    facts=dict(name=name,source_sha256=hashlib.sha256(data.getvalue()).hexdigest(),pattern=pattern,tokens=[[list(token) for token in run] for run in tokens],
                        available_width_pt=width,compatibility_mode=mode,source_parent=PARENT.as_posix(),
                        source_parent_sha256=PARENT_SHA256,mac_word_version='16.114.4',changed_parts=sorted(changes),
                        literal_control_count=sum(value.count('\u00ad') for run in tokens for tag,value in run if value),
                        optional_control_count=sum(tag=='softHyphen' for run in tokens for tag,value in run),
                        scope='Derived Windows mixed source-form controls; unchanged Mac parent, run properties, section, fonts, theme and all other package parts. Not new Word-for-Mac exports.')
                    yield name,data.getvalue(),facts
