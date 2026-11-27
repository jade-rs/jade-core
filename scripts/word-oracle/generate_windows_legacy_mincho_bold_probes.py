"""Preserve original long MS Mincho bold inputs while varying compatibility/font slots."""
from pathlib import Path
from io import BytesIO
from zipfile import ZipFile,ZipInfo,ZIP_DEFLATED
import hashlib
from lxml import etree as E

ROOT=Path(__file__).resolve().parents[2]
PARENTS={12:('public/fixtures/word-windows-printer-metric-mincho-size12-bold-compat15.docx','92fd2cd6c1049b1d1a30afd10b24ebb6014f35a59056aa7c4a8a3ee80bfb67f6'),20:('public/fixtures/word-windows-printer-metric-mincho-size20-bold-compat15.docx','db2f49494095335b97b3814d98250c8988f0876bed9d4a82eb4108847b30d8ab')}
q=lambda n:'{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'+n


def generate(root=ROOT):
    for size,(relative,digest) in PARENTS.items():
        source=(root/relative).read_bytes()
        if hashlib.sha256(source).hexdigest()!=digest:
            raise ValueError('Original MS Mincho bold source changed')
        for mode,latin in [(12,'MS Mincho'),(14,'MS Mincho'),(12,'Times New Roman'),(14,'Times New Roman'),(15,'Times New Roman')]:
            with ZipFile(BytesIO(source)) as package:
                document=E.fromstring(package.read('word/document.xml'));settings=E.fromstring(package.read('word/settings.xml'))
                for fonts in document.iter(q('rFonts')):
                    assert fonts.get(q('eastAsia'))=='MS Mincho'
                    for slot in ('ascii','hAnsi','cs'):fonts.set(q(slot),latin)
                compatibility,=[n for n in settings.iter(q('compatSetting')) if n.get(q('name'))=='compatibilityMode']
                compatibility.set(q('val'),str(mode))
                parts={name:E.tostring(x,encoding='utf-8',xml_declaration=True,standalone=True) for name,x in [('word/document.xml',document),('word/settings.xml',settings)]}
                output=BytesIO()
                with ZipFile(output,'w',compression=ZIP_DEFLATED) as target:
                    for item in package.infolist():
                        info=ZipInfo(item.filename,(2026,11,19,0,0,0));info.create_system=0;info.compress_type=ZIP_DEFLATED;info.external_attr=item.external_attr
                        target.writestr(info,parts.get(item.filename,package.read(item.filename)))
            label='mincho' if latin=='MS Mincho' else 'times'
            name=f'word-windows-legacy-mincho-bold-{label}-s{size}-m{mode}'
            facts=dict(derived_parent=dict(file=relative,sha256=digest),parent_name=Path(relative).stem,font_family=latin,east_asia_font_family='MS Mincho',font_style='bold',font_size_pt=size,compatibility_mode=mode,line_ending='paragraph',text='日本語'*12,expected_non_whitespace_glyphs=36,changed_parts=list(parts))
            yield name,output.getvalue(),facts
