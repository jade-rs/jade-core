"""Reproduce versioned Windows Word mixed-face justification inputs."""
from pathlib import Path
from io import BytesIO
from zipfile import ZipFile
import copy,hashlib,json
from lxml import etree as E
from generate_windows_legacy_justification_probes import changed_document
ROOT=Path(__file__).resolve().parents[2]
PARENTS=Path('tests/word-oracle/windows/legacy-justification-observations.json')
PARENTS_SHA256='2f0cba4117165e39fac724f4232216072263697861aa958072937c3051aabe31'
FACES=(('Arial','Calibri','arial-calibri'),('Arial','Times New Roman','arial-times'),('Calibri','Times New Roman','calibri-times'))
SPLICES=(('after-abc','Ag日本語 ABC ','Ag日本語'),('after-cjk','Ag日本語 ','ABC Ag日本語'))
# Authored near-fit widths from source-native natural metrics plus1.5pt,
# independently of Word expected coordinates. Raw authoring facts are archived.
TIGHT_WIDTHS={'arial-calibri':(2744,2649),'arial-times':(2772,2765),'calibri-times':(2632,2720)}
q=lambda n:'{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'+n

def generate(root=ROOT):
    data=(root/PARENTS).read_bytes()
    if hashlib.sha256(data).hexdigest()!=PARENTS_SHA256:raise ValueError('Mixed-face parent registry changed')
    parents={r['name']:r for r in json.loads(data)['fixtures']}
    for group,splices,widths in [('mixed-face',SPLICES[:1],(3000,8504)),('split-space-face',SPLICES[1:],(3000,8504)),('tight-face',SPLICES,None)]:
        for left,right,label in FACES:
            for mode in (14,15):
                for splice,lefttext,righttext in splices:
                    for ending in ('manual','paragraph'):
                        selected=widths if widths else (TIGHT_WIDTHS[label][0 if splice=='after-abc' else 1],)
                        for width in selected:
                            parent_width=width if widths else 3000
                            parent=parents[f'word-windows-legacy-font-calibri-s12-m{mode}-{ending}-w{parent_width}']
                            source=(root/parent['source']['file']).read_bytes()
                            if hashlib.sha256(source).hexdigest()!=parent['source']['sha256']:raise ValueError('Mixed-face parent bytes changed')
                            with ZipFile(BytesIO(source)) as package:document=E.fromstring(package.read('word/document.xml'))
                            paragraph,=document.iter(q('p'))
                            if widths is None:paragraph.find(q('pPr')).find(q('ind')).set(q('right'),str(8504-width))
                            second,=paragraph.iter(q('r'));first=copy.deepcopy(second)
                            for node in list(first):
                                if node.tag!=q('rPr'):first.remove(node)
                            text=E.Element(q('t'));text.set('{http://www.w3.org/XML/1998/namespace}space','preserve');text.text=lefttext;first.append(text)
                            next(second.iter(q('t'))).text=righttext
                            for run,family in ((first,left),(second,right)):
                                fonts=run.find(q('rPr')).find(q('rFonts'))
                                for slot in ('ascii','hAnsi','cs'):fonts.set(q(slot),family)
                            paragraph.insert(list(paragraph).index(second),first)
                            name=f'word-windows-{group}-'+(splice+'-' if group=='tight-face' else '')+f'{label}-s12-m{mode}-{ending}-w{width}'
                            spec=dict(derived_parent=parent['source'],left_font_family=left,right_font_family=right,font_size_pt=12,compatibility_mode=mode,line_ending=ending,line_width_twips=width,left_text=lefttext,right_text=righttext,following_text='X' if ending=='manual' else '',expected_non_whitespace_glyphs=14 if ending=='manual' else 13,changed_parts=['word/document.xml'])
                            yield name,changed_document(source,E.tostring(document,encoding='utf-8',xml_declaration=True,standalone=True)),spec
