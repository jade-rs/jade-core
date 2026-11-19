"""Wide-page repeated positive-kern controls from original Aptos size parents."""
from pathlib import Path
from io import BytesIO
from zipfile import ZipFile,ZipInfo,ZIP_DEFLATED
from lxml import etree as E
import hashlib,json,sys
ROOT=Path(__file__).resolve().parents[2]
import generate_windows_aptos_size_probes as parentgen
q=parentgen.q

def generate(root=ROOT):
    parents={r['name']:r for r in json.loads((root/'tests/word-oracle/windows/aptos-font-size-observations.json').read_text(encoding='utf-8'))['fixtures']}
    for parent_name,data,spec in parentgen.generate(root):
        parent=parents[parent_name]
        if hashlib.sha256(data).hexdigest()!=parent['source']['sha256']:raise ValueError('Original font-size parent changed')
        for char in ('X','V','Z'):
            with ZipFile(BytesIO(data)) as source:
                tree=E.fromstring(source.read('word/document.xml'));texts=list(tree.iter(q('t')))
                if len(texts)!=4:raise ValueError('Expected four original paragraphs')
                for node in texts:node.text=char*48
                page,=tree.iter(q('pgSz'));height=int(page.get(q('h')));page.set(q('w'),'17280');page.set(q('orient'),'landscape')
                xml=E.tostring(tree,encoding='utf-8',xml_declaration=True);buffer=BytesIO()
                with ZipFile(buffer,'w',compression=ZIP_DEFLATED) as target:
                    for info in source.infolist():
                        item=ZipInfo(info.filename,(2026,11,19,0,0,0));item.create_system=0;item.compress_type=ZIP_DEFLATED;item.external_attr=info.external_attr
                        target.writestr(item,xml if info.filename=='word/document.xml' else source.read(info.filename))
            size=str(spec['font_size_pt']).removesuffix('.0').replace('.','p')
            name=f"word-windows-positive-kern-{spec['style']}-s{size}-{char}48-m{spec['compatibility_mode']}"
            yield name,buffer.getvalue(),dict(derived_parent=parent['source'],mac_parent=parentgen.PARENT,mac_parent_sha256=parentgen.PARENT_SHA256,font_family=spec['font_family'],font_size_pt=spec['font_size_pt'],style=spec['style'],compatibility_mode=spec['compatibility_mode'],character=char,characters_per_paragraph=48,paragraphs=4,retained_kerning_thresholds_halfpoints=[200,200,2,2],page_width_twips=17280,page_height_twips=height,page_orientation='landscape',changed_parts=['word/document.xml'])

