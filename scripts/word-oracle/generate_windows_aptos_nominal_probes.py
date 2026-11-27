"""Reproduce nominal advance controls without changing captured parent formatting."""
from pathlib import Path
from io import BytesIO
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED
import hashlib,json,sys
from lxml import etree as E
ROOT=Path(__file__).resolve().parents[2]
import generate_windows_aptos_size_probes as parentgen

def generate(root=ROOT):
    metadata=json.loads((root/'tests/word-oracle/windows/aptos-font-size-observations.json').read_text(encoding='utf-8-sig'))
    parents={r['name']:r for r in metadata['fixtures']}
    for name,data,facts in parentgen.generate(root=root):
        if (facts['style'],facts['font_size_pt']) not in {('bold',14),('bolditalic',15),('regular',16)}:continue
        if hashlib.sha256(data).hexdigest()!=parents[name]['source']['sha256']:raise ValueError('Captured parent changed')
        for label,char in [('X','X'),('a','a'),('one','1'),('hyphen','-')]:
            with ZipFile(BytesIO(data)) as source:
                document=E.fromstring(source.read('word/document.xml'));texts=list(document.iter(parentgen.q('t')))
                if len(texts)!=4:raise ValueError('Parent paragraph coverage changed')
                for node in texts:node.text=char*48
                xml=E.tostring(document,encoding='utf-8',xml_declaration=True);buffer=BytesIO()
                with ZipFile(buffer,'w',compression=ZIP_DEFLATED) as target:
                    for info in source.infolist():
                        item=ZipInfo(info.filename,(2026,11,19,0,0,0));item.create_system=0;item.compress_type=ZIP_DEFLATED;item.external_attr=info.external_attr
                        target.writestr(item,xml if info.filename=='word/document.xml' else source.read(info.filename))
            result=f"word-windows-aptos-nominal-{facts['style']}-s{int(facts['font_size_pt'])}-{label}48-m{facts['compatibility_mode']}"
            yield result,buffer.getvalue(),dict(derived_parent=parents[name]['source'],mac_parent=parentgen.PARENT,mac_parent_sha256=parentgen.PARENT_SHA256,font_family=facts['font_family'],font_size_pt=facts['font_size_pt'],style=facts['style'],compatibility_mode=facts['compatibility_mode'],character=char,characters_per_paragraph=48,paragraphs=4,retained_kerning_thresholds_halfpoints=[200,200,2,2],changed_parts=['word/document.xml'])

