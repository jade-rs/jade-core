"""Reproduce minimum-line controls from the immutable Mac original, without Word."""
from copy import deepcopy
from io import BytesIO
from zipfile import ZipFile,ZipInfo,ZIP_DEFLATED
import hashlib
from lxml import etree as ET
PARENT='tests/word-oracle/mac-elsa-synthetic/exact-and-minimum-lines-mac/source.docx'
PARENT_SHA256='c1bb8ae4b58d80653e16a81f57143247fc2e7d821a8fc321c456a54ee6e42e4a'
q=lambda n:'{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'+n
def properties(family,size,script):
    props=ET.Element(q('rPr'));fonts=ET.SubElement(props,q('rFonts'))
    for name in ('ascii','hAnsi','eastAsia','cs'):fonts.set(q(name),family)
    for name in ('b','i'):ET.SubElement(props,q(name)).set(q('val'),'0')
    for name in ('sz','szCs'):ET.SubElement(props,q(name)).set(q('val'),str(size*2))
    ET.SubElement(props,q('lang')).set(q('val'),'ja-JP' if script=='cjk' else 'en-US')
    return props

def generate(parent_bytes):
    if hashlib.sha256(parent_bytes).hexdigest()!=PARENT_SHA256:
        raise ValueError('Original Mac minimum-line source changed')
    with ZipFile(BytesIO(parent_bytes)) as parent:
        original=ET.fromstring(parent.read('word/document.xml'))
        section=original.find(q('body')).find(q('sectPr'))
        settings=ET.fromstring(parent.read('word/settings.xml'))
        for family,slug,scripts in [('Yu Mincho','yu-mincho',('latin','cjk')),
                                    ('MS Mincho','ms-mincho',('latin','cjk')),
                                    ('Arial','arial',('latin',)),('Courier New','courier-new',('latin',))]:
            for script in scripts:
                texts=('大きな文字です。','次の行の文字。') if script=='cjk' else ('Wider text on first line.','Next line of text.')
                for profile,first,last in [('uniform12',12,12),('uniform20',20,20),('mixed20p10',20,10)]:
                    for minimum in (12,20,28):
                        for mode in (14,15):
                            doc=deepcopy(original);body=doc.find(q('body'));body.clear()
                            for rule,line in [('atLeast',minimum*20),('auto',240)]:
                                paragraph=ET.SubElement(body,q('p'));props=ET.SubElement(paragraph,q('pPr'))
                                ET.SubElement(props,q('snapToGrid')).set(q('val'),'0')
                                spacing=ET.SubElement(props,q('spacing'))
                                for name,value in [('before','0'),('after','0'),('line',str(line)),('lineRule',rule)]:spacing.set(q(name),value)
                                props.append(properties(family,12,script))
                                for index,(text,size) in enumerate(zip(texts,(first,last))):
                                    run=ET.SubElement(paragraph,q('r'));run.append(properties(family,size,script))
                                    if index:ET.SubElement(run,q('br'))
                                    ET.SubElement(run,q('t')).text=text
                            body.append(deepcopy(section))
                            config=deepcopy(settings);compat=config.find(q('compat'))
                            if compat is None:compat=ET.SubElement(config,q('compat'))
                            modes=[e for e in compat.findall(q('compatSetting')) if e.get(q('name'))=='compatibilityMode']
                            assert len(modes)<=1
                            setting=modes[0] if modes else ET.SubElement(compat,q('compatSetting'))
                            for name,value in [('name','compatibilityMode'),('uri','http://schemas.microsoft.com/office/word'),('val',str(mode))]:setting.set(q(name),value)
                            changes={'word/document.xml':ET.tostring(doc,encoding='utf-8',xml_declaration=True),
                                     'word/settings.xml':ET.tostring(config,encoding='utf-8',xml_declaration=True)}
                            buffer=BytesIO()
                            with ZipFile(buffer,'w',compression=ZIP_DEFLATED) as bundle:
                                for entry in parent.infolist():
                                    info=ZipInfo(entry.filename,(2026,11,18,0,0,0));info.create_system=0
                                    info.compress_type=ZIP_DEFLATED;info.external_attr=entry.external_attr
                                    bundle.writestr(info,changes.get(entry.filename,parent.read(entry.filename)))
                            name=f'word-windows-minimum-line-{slug}-{script}-{profile}-min{minimum}-compat{mode}'
                            yield name,buffer.getvalue(),dict(parent_source=PARENT,
                                parent_sha256=PARENT_SHA256,font_family=family,font_size_pt=first,last_run_font_size_pt=last,
                                paragraph_mark_font_size_pt=12,script=script,language='ja-JP' if script=='cjk' else 'en-US',
                                profile=profile,minimum_pt=minimum,compatibility_mode=mode,snap_to_grid=False,
                                text_patterns=list(texts),line_rules=[dict(rule='atLeast',line=minimum*20),dict(rule='auto',line=240)],
                                changed_parts=sorted(changes))
