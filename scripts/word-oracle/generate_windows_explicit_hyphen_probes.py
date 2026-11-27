"""Reproduce source-form controls from the unchanged Word-for-Mac original."""
from copy import deepcopy
from io import BytesIO
from pathlib import Path
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED
import hashlib

from lxml import etree as ET

ROOT = Path(__file__).resolve().parents[2]
PARENT = Path('tests/word-oracle/mac-elsa-synthetic/explicit-hyphen-wrap-boundaries-mac/source.docx')
PARENT_SHA256 = '64e01a358c7c8c12a61d9b50068785afd05e4f3875dc199e5bdde5f3bb322f15'
KINDS = ('soft-element', 'soft-literal', 'nonbreak-element', 'ascii', 'none')
WIDTHS = (35, 42, 45, 48, 65, 110)
MODES = (14, 15)


def q(name):
    return '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}' + name


def generate(parent=None):
    parent = (ROOT / PARENT).read_bytes() if parent is None else parent
    if hashlib.sha256(parent).hexdigest() != PARENT_SHA256:
        raise ValueError('Original Mac hyphen source changed')
    with ZipFile(BytesIO(parent)) as original:
        for kind in KINDS:
            for width in WIDTHS:
                for mode in MODES:
                    document = ET.fromstring(original.read('word/document.xml'))
                    settings = ET.fromstring(original.read('word/settings.xml'))
                    body = document.find(q('body'))
                    template = deepcopy(body.findall(q('p'))[1])
                    section = body.find(q('sectPr'))
                    page, margins = section.find(q('pgSz')), section.find(q('pgMar'))
                    available = int(page.get(q('w'))) - int(margins.get(q('left'))) - int(margins.get(q('right')))
                    template.find(q('pPr') + '/' + q('ind')).set(q('right'), str(available - width * 20))
                    run = template.find(q('r'))
                    marker, text = run.find(q('softHyphen')), run.find(q('t'))
                    assert marker is not None and text.text == 'electro'
                    if kind == 'nonbreak-element':
                        marker.tag = q('noBreakHyphen')
                    elif kind != 'soft-element':
                        run.remove(marker)
                        text.text += '\u00ad' if kind == 'soft-literal' else '-' if kind == 'ascii' else ''
                    for child in list(body):
                        if child is not section:
                            body.remove(child)
                    body.insert(0, template)
                    compatibility = [e for e in settings.iter(q('compatSetting')) if e.get(q('name')) == 'compatibilityMode']
                    assert len(compatibility) == 1
                    compatibility[0].set(q('val'), str(mode))
                    changes = {
                        'word/document.xml': ET.tostring(document, encoding='utf-8', xml_declaration=True, standalone=True),
                        'word/settings.xml': ET.tostring(settings, encoding='utf-8', xml_declaration=True, standalone=True),
                    }
                    result = BytesIO()
                    with ZipFile(result, 'w', compression=ZIP_DEFLATED) as output:
                        for entry in original.infolist():
                            item = ZipInfo(entry.filename, (2026, 11, 18, 0, 0, 0))
                            item.create_system = 0
                            item.compress_type = ZIP_DEFLATED
                            item.external_attr = entry.external_attr
                            output.writestr(item, changes.get(entry.filename, original.read(entry.filename)))
                    data = result.getvalue()
                    name = f'word-windows-explicit-hyphen-{kind}-width{width}-compat{mode}'
                    facts = dict(name=name, source_sha256=hashlib.sha256(data).hexdigest(), marker_kind=kind,
                                 available_width_pt=width, compatibility_mode=mode, source_parent=PARENT.as_posix(),
                                 source_parent_sha256=PARENT_SHA256, mac_word_version='16.114.4',
                                 changed_parts=sorted(changes), paragraph_count=1,
                                 scope='Keep original section, source run styles, fonts, theme and all other package parts; isolate one electro/magnetic paragraph and vary only marker kind, paragraph width and compatibility mode. Derived control, not a new Word-for-Mac export.')
                    yield name, data, facts
