"""Reproduce Aptos wrap/kerning inputs from an unchanged Word-for-Mac DOCX.

Only inputs are generated here. Expected coordinates come from actual Word.
"""
import argparse
from copy import deepcopy
from io import BytesIO
from itertools import product
from pathlib import Path
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

from lxml import etree as ET

ROOT = Path(__file__).resolve().parents[2]
PARENT = Path('tests/word-oracle/mac-elsa-synthetic/aptos-ascii-kerning-mac/source.docx')
TEXTS = ('AV AW AY FA LT LV LY PA Ta Te To Tr Ty VA Vo WA We Wo Ya Yo',
         'Text Test Trial reserve waiver title X-ray 11 112233 -12 12.34')
STYLES = [('regular', False, False), ('italic', False, True),
          ('bold', True, False), ('bolditalic', True, True)]
W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'


def probes(source=ROOT / PARENT):
    q = lambda n: '{' + W + '}' + n
    with ZipFile(source) as parent:
        original = ET.fromstring(parent.read('word/document.xml'))
        template = original.find(q('body')).find(q('p'))
        section = original.find(q('body')).find(q('sectPr'))
        settings = ET.fromstring(parent.read('word/settings.xml'))
        for family, label in [('Aptos', 'aptos'), ('Aptos Display', 'display')]:
            for style, bold, italic in STYLES:
                for size, mode in product((14, 20), (14, 15)):
                    doc = deepcopy(original)
                    body = doc.find(q('body'))
                    body.clear()
                    for kern, text in product((200, 2), TEXTS):
                        paragraph = deepcopy(template)
                        run = paragraph.find(q('r'))
                        props = run.find(q('rPr'))
                        fonts = props.find(q('rFonts'))
                        for attr in ('ascii', 'hAnsi'):
                            fonts.set(q(attr), family)
                        props.find(q('sz')).set(q('val'), str(size * 2))
                        props.find(q('b')).set(q('val'), '1' if bold else '0')
                        props.find(q('i')).set(q('val'), '1' if italic else '0')
                        props.find(q('kern')).set(q('val'), str(kern))
                        run.find(q('t')).text = text
                        body.append(paragraph)
                    body.append(deepcopy(section))
                    xml = ET.tostring(doc, encoding='utf-8', xml_declaration=True)
                    new_settings = deepcopy(settings)
                    setting, = [e for e in new_settings.iter(q('compatSetting'))
                                if e.get(q('name')) == 'compatibilityMode']
                    setting.set(q('val'), str(mode))
                    buffer = BytesIO()
                    with ZipFile(buffer, 'w', compression=ZIP_DEFLATED) as bundle:
                        for entry in parent.infolist():
                            item = ZipInfo(entry.filename, (2026, 11, 18, 0, 0, 0))
                            item.create_system = 0
                            item.compress_type = ZIP_DEFLATED
                            item.external_attr = entry.external_attr
                            data = (xml if entry.filename == 'word/document.xml' else
                                    ET.tostring(new_settings, encoding='utf-8', xml_declaration=True)
                                    if entry.filename == 'word/settings.xml' and mode == 15 else
                                    parent.read(entry.filename))
                            bundle.writestr(item, data)
                    name = f'word-windows-aptos-long-kern-{label}-size{size}-{style}-compat{mode}'
                    yield name, buffer.getvalue()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output_directory', type=Path)
    args = parser.parse_args()
    args.output_directory.mkdir(parents=True, exist_ok=False)
    for name, raw in probes():
        (args.output_directory / (name + '.docx')).write_bytes(raw)
