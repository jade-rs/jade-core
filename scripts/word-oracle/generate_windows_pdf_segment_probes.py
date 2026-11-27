"""Reproduce four Word PDF segmentation observations from an immutable Mac source."""
from io import BytesIO
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED
import copy
from lxml import etree as E
from generate_windows_top_bottom_probes import generate as generate_float_controls, q


def generate():
    parents = [data for name, data, _ in generate_float_controls()
               if name == 'word-windows-top-bottom-s11-o52p5-b0-m15']
    if len(parents) != 1:
        raise ValueError('Missing unchanged derived source')
    with ZipFile(BytesIO(parents[0])) as source:
        for mode in (14, 15):
            for split in (False, True):
                root = E.fromstring(source.read('word/document.xml'))
                settings = E.fromstring(source.read('word/settings.xml'))
                drawings = list(root.iter(q('drawing')))
                if len(drawings) != 1:
                    raise ValueError('Expected one removable original drawing')
                drawings[0].getparent().remove(drawings[0])
                node = list(root.iter(q('t')))[-1]
                run = node.getparent()
                text = node.text
                if len(text) != 556 or not text.startswith('OFFSET_TOP'):
                    raise ValueError('Original body text changed')
                if split:
                    second = copy.deepcopy(run)
                    node.text = text[:100]
                    second.find(q('t')).text = text[100:]
                    color = E.SubElement(second.find(q('rPr')), q('color'))
                    color.set(q('val'), '000001')
                    run.addnext(second)
                compat, = [e for e in settings.iter(q('compatSetting'))
                           if e.get(q('name')) == 'compatibilityMode']
                compat.set(q('val'), str(mode))
                changes = {'word/document.xml': E.tostring(root, xml_declaration=True,
                           encoding='UTF-8', standalone=True),
                           'word/settings.xml': E.tostring(settings, xml_declaration=True,
                           encoding='UTF-8', standalone=True)}
                data = BytesIO()
                with ZipFile(data, 'w', compression=ZIP_DEFLATED) as target:
                    for info in source.infolist():
                        item = ZipInfo(info.filename, (2026, 11, 18, 0, 0, 0))
                        item.create_system = 0
                        item.compress_type = ZIP_DEFLATED
                        target.writestr(item, changes.get(info.filename, source.read(info.filename)))
                name = f'word-windows-pdf256-nofloat-{"split100" if split else "single"}-m{mode}'
                yield name, data.getvalue()
