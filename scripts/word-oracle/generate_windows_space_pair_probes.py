"""Reproduce actual Word space-pair kerning controls from retained originals."""
from pathlib import Path
from io import BytesIO
from zipfile import ZipFile
import hashlib,json
from lxml import etree as E
from generate_windows_legacy_justification_probes import changed_document

ROOT=Path(__file__).resolve().parents[2]
PARENTS=Path('tests/word-oracle/windows/hard-break-space-observations.json')
PARENTS_SHA256='eed12b70997dc4f8964b6ed4ce18d318bee8c93625a283bc460ad2fba1c51d33'
q=lambda n:'{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'+n

def generate(root=ROOT):
    data=(root/PARENTS).read_bytes()
    if hashlib.sha256(data).hexdigest()!=PARENTS_SHA256:
        raise ValueError('Original space-pair parent registry changed')
    parents={c['name']:c for c in json.loads(data)['fixtures']}
    for family,label in [('Arial','arial'),('Times New Roman','times')]:
        for mode in (14,15):
            parent=parents[f'word-windows-hard-break-ea-arial-s12-m{mode}-paragraph-spaces0']
            source=(root/parent['source']['file']).read_bytes()
            if hashlib.sha256(source).hexdigest()!=parent['source']['sha256']:
                raise ValueError('Original space-pair parent bytes changed')
            for kern in (0,2):
                for tail in ('AA','BB','VA','TA'):
                    with ZipFile(BytesIO(source)) as package:
                        document=E.fromstring(package.read('word/document.xml'))
                    run,=document.iter(q('r'));properties=run.find(q('rPr'));fonts=properties.find(q('rFonts'))
                    for slot in ('ascii','hAnsi','cs'):fonts.set(q(slot),family)
                    if properties.find(q('kern')) is not None:raise ValueError('Unexpected parent kerning')
                    setting=E.Element(q('kern'));setting.set(q('val'),str(kern));properties.insert(1,setting)
                    text='AA '+tail;next(run.iter(q('t'))).text=text
                    xml=E.tostring(document,encoding='utf-8',xml_declaration=True,standalone=True)
                    name=f'word-windows-space-pair-{label}-s12-m{mode}-kern{kern}-aa-{tail.lower()}'
                    yield name,changed_document(source,xml),dict(derived_parent=parent['source'],parent_name=parent['name'],
                        font_family=family,font_size_pt=12,compatibility_mode=mode,kerning_half_points=kern,text=text,
                        expected_non_whitespace_glyphs=4,changed_parts=['word/document.xml'])
