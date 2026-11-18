"""Reproduce setting-only controls from immutable captured nominal parents."""
from pathlib import Path
from io import BytesIO
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED
import hashlib
import json
import sys
from lxml import etree as E
ROOT=Path(__file__).resolve().parents[2]
import generate_windows_aptos_nominal_probes as parentgen
q=parentgen.parentgen.q


def generate(root=ROOT):
    parents={r['name']:r for r in json.loads((root/'tests/word-oracle/windows/aptos-nominal-observations.json').read_text(encoding='utf-8'))['fixtures']}
    for parent_name,data,spec in parentgen.generate(root):
        if (spec['style'],spec['character']) not in {('bold','X'),('bolditalic','X'),('regular','1'),('regular','-')}:continue
        parent=parents[parent_name]
        if hashlib.sha256(data).hexdigest()!=parent['source']['sha256']:raise ValueError('Original nominal parent changed')
        for enabled in (False,True):
            with ZipFile(BytesIO(data)) as source:
                tree=E.fromstring(source.read('word/settings.xml'));compat=tree.find(q('compat'))
                if compat is None:compat=E.SubElement(tree,q('compat'))
                for node in list(compat.findall(q('usePrinterMetrics'))):compat.remove(node)
                node=E.Element(q('usePrinterMetrics'));node.set(q('val'),'1' if enabled else '0');compat.insert(0,node)
                xml=E.tostring(tree,encoding='utf-8',xml_declaration=True);buffer=BytesIO()
                with ZipFile(buffer,'w',compression=ZIP_DEFLATED) as target:
                    for info in source.infolist():
                        item=ZipInfo(info.filename,(2026,11,19,0,0,0));item.create_system=0;item.compress_type=ZIP_DEFLATED;item.external_attr=info.external_attr
                        target.writestr(item,xml if info.filename=='word/settings.xml' else source.read(info.filename))
            label={'X':'X','1':'one','-':'hyphen'}[spec['character']]
            name=f"word-windows-printer-metrics-{spec['style']}-s{int(spec['font_size_pt'])}-{label}-m{spec['compatibility_mode']}-p{int(enabled)}"
            yield name,buffer.getvalue(),dict(derived_parent=parent['source'],mac_parent=spec['mac_parent'],mac_parent_sha256=spec['mac_parent_sha256'],font_family=spec['font_family'],font_size_pt=spec['font_size_pt'],style=spec['style'],character=spec['character'],compatibility_mode=spec['compatibility_mode'],use_printer_metrics=enabled,changed_parts=['word/settings.xml'])


