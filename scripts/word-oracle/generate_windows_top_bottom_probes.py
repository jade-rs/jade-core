"""Reproduce24 Windows float-clearance controls from the unchanged Mac source."""
from pathlib import Path
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED
from io import BytesIO
from lxml import etree as E
import hashlib
import itertools

ROOT = Path(__file__).resolve().parents[2]
PARENT = Path('tests/word-oracle/mac-elsa-synthetic/floating-top-bottom-offset-mac/source.docx')
PARENT_SHA256 = '326f3eb97211babfc79d60f8902bfc1ca75f4fef6fdd95f53001341a3c18b2ca'
q = lambda n: '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}' + n
wp = lambda n: '{http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing}' + n


def generate(parent=None):
    parent = (ROOT / PARENT).read_bytes() if parent is None else parent
    if hashlib.sha256(parent).hexdigest() != PARENT_SHA256:
        raise ValueError('Original Mac float source changed')
    relative = PARENT.as_posix()
    with ZipFile(BytesIO(parent)) as z:
        for size,offset,bottom,mode in itertools.product((11,14),(26.25,52.5,78.75),(0,7.5),(14,15)):
            root=E.fromstring(z.read('word/document.xml'));settings=E.fromstring(z.read('word/settings.xml'))
            anchor=root.find('.//'+wp('anchor'));assert anchor is not None
            anchor.find(wp('positionV')+'/'+wp('posOffset')).text=str(round(offset*12700));anchor.set('distB',str(round(bottom*12700)))
            changed_runs=0
            for p in root.find(q('body')).findall(q('p')):
                if p.find(q('pPr')+'/'+q('pStyle')) is not None:continue
                for r in p.findall(q('r')):
                    if r.find(q('t')) is None:continue
                    rp=r.find(q('rPr'))
                    if rp is None:rp=E.Element(q('rPr'));r.insert(0,rp)
                    for n in ('sz','szCs'):
                        e=rp.find(q(n))
                        if e is None:e=E.SubElement(rp,q(n))
                        e.set(q('val'),str(size*2))
                    changed_runs+=1
            assert changed_runs>0
            compat=[e for e in settings.iter(q('compatSetting')) if e.get(q('name'))=='compatibilityMode'];assert len(compat)==1;compat[0].set(q('val'),str(mode))
            changes={'word/document.xml':E.tostring(root,xml_declaration=True,encoding='UTF-8',standalone=True),'word/settings.xml':E.tostring(settings,xml_declaration=True,encoding='UTF-8',standalone=True)}
            b=BytesIO()
            with ZipFile(b,'w',compression=ZIP_DEFLATED) as dest:
                for info in z.infolist():
                    item=ZipInfo(info.filename,(2026,11,18,0,0,0));item.create_system=0;item.compress_type=ZIP_DEFLATED;item.external_attr=info.external_attr
                    dest.writestr(item,changes.get(info.filename,z.read(info.filename)))
            name=f'word-windows-top-bottom-s{size}-o{str(offset).replace(".","p")}-b{str(bottom).replace(".","p")}-m{mode}'
            facts=dict(name=name,source_sha256=hashlib.sha256(b.getvalue()).hexdigest(),source_parent=relative,source_parent_sha256=hashlib.sha256(parent).hexdigest(),font_size_pt=size,vertical_offset_pt=offset,distance_bottom_pt=bottom,compatibility_mode=mode,changed_parts=sorted(changes))
            yield name, b.getvalue(), facts
