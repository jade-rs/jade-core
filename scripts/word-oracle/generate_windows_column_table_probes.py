"""Derive fixed-table width controls from an immutable Mac original."""
from copy import deepcopy
import hashlib
from io import BytesIO
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

from lxml import etree as ET

PARENT = 'tests/word-oracle/mac-elsa-synthetic/column-table-fixed-preferred-width-mac/source.docx'
PARENT_SHA256 = 'c9d087ca03e36ef6587c0d3a15e24e86653e25de6cbbce006fd25d25672d0350'
W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'
q = lambda name: '{' + W + '}' + name


def generate(parent_bytes):
    """Yield name, exact DOCX bytes and source facts; preserve every other part."""
    if hashlib.sha256(parent_bytes).hexdigest() != PARENT_SHA256:
        raise ValueError('Original Mac table source changed')
    with ZipFile(BytesIO(parent_bytes)) as parent:
        document = ET.fromstring(parent.read('word/document.xml'))
        settings = ET.fromstring(parent.read('word/settings.xml'))
        assert len(document.findall('.//' + q('tbl'))) == 1
        for frame in ('single', 'equal', 'unequal'):
            for width_pt in (80, 100, 120):
                for alignment in ('left', 'center', 'right'):
                    for mode in (14, 15):
                        doc = deepcopy(document)
                        table = doc.find('.//' + q('tbl'))
                        props = table.find(q('tblPr'))
                        props.find(q('tblW')).set(q('w'), str(width_pt * 20))
                        props.find(q('jc')).set(q('val'), alignment)
                        section = doc.find('.//' + q('sectPr'))
                        columns = section.find(q('cols'))
                        columns.clear()
                        columns.set(q('num'), '1' if frame == 'single' else '2')
                        columns.set(q('space'), '480')
                        if frame == 'unequal':
                            columns.set(q('equalWidth'), '0')
                            first = ET.SubElement(columns, q('col'))
                            first.set(q('w'), '2500')
                            first.set(q('space'), '480')
                            second = ET.SubElement(columns, q('col'))
                            second.set(q('w'), '6046')
                        config = deepcopy(settings)
                        compatibility = config.find(q('compat'))
                        if compatibility is None:
                            compatibility = ET.SubElement(config, q('compat'))
                        modes = [e for e in compatibility.findall(q('compatSetting'))
                                 if e.get(q('name')) == 'compatibilityMode']
                        assert len(modes) <= 1
                        setting = modes[0] if modes else ET.SubElement(compatibility, q('compatSetting'))
                        setting.set(q('name'), 'compatibilityMode')
                        setting.set(q('uri'), 'http://schemas.microsoft.com/office/word')
                        setting.set(q('val'), str(mode))
                        changed = {'word/document.xml': ET.tostring(doc, encoding='utf-8', xml_declaration=True),
                                   'word/settings.xml': ET.tostring(config, encoding='utf-8', xml_declaration=True)}
                        buffer = BytesIO()
                        with ZipFile(buffer, 'w', compression=ZIP_DEFLATED) as output:
                            for entry in parent.infolist():
                                info = ZipInfo(entry.filename, (2026, 11, 18, 0, 0, 0))
                                info.create_system = 0
                                info.compress_type = ZIP_DEFLATED
                                info.external_attr = entry.external_attr
                                output.writestr(info, changed.get(entry.filename, parent.read(entry.filename)))
                        name = f'word-windows-column-table-{frame}-width{width_pt}-{alignment}-compat{mode}'
                        yield name, buffer.getvalue(), dict(
                            parent_source=PARENT, parent_sha256=PARENT_SHA256,
                            frame=frame, preferred_width_pt=width_pt, saved_grid_width_pt=100,
                            alignment=alignment, compatibility_mode=mode,
                            changed_parts=sorted(changed))
