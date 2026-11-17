"""Reproduce Windows Courier/grid inputs while preserving the Mac original.

The original Mac fixture has an unbound mc:Ignorable prefix. Windows controls
bind that prefix in their own styles.xml; the Mac source/goldens stay immutable.
This generates inputs only. Drawing expectations must come from actual Word.
"""
import argparse
from copy import deepcopy
from io import BytesIO
from pathlib import Path
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

from lxml import etree as ET

ROOT = Path(__file__).resolve().parents[2]
PARENT = Path('tests/word-oracle/mac-elsa-synthetic/adjacent-fixed-table-grids/source.docx')
W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'
W14 = 'http://schemas.microsoft.com/office/word/2010/wordml'
MC = 'http://schemas.openxmlformats.org/markup-compatibility/2006'
STYLES = [('regular', False, False), ('italic', False, True),
          ('bold', True, False), ('bolditalic', True, True)]
LINE_RULES = [(240, 'auto'), (480, 'auto'), (480, 'exact')]


def _archive(parent, changes):
    buffer = BytesIO()
    with ZipFile(buffer, 'w', compression=ZIP_DEFLATED) as bundle:
        for entry in parent.infolist():
            item = ZipInfo(entry.filename, (2026, 11, 18, 0, 0, 0))
            item.create_system = 0
            item.compress_type = ZIP_DEFLATED
            item.external_attr = entry.external_attr
            bundle.writestr(item, changes.get(entry.filename, parent.read(entry.filename)))
    return buffer.getvalue()


def probes(source=ROOT / PARENT):
    q = lambda n: '{' + W + '}' + n
    with ZipFile(source) as parent:
        styles = parent.read('word/styles.xml')
        tree = ET.fromstring(styles)
        if tree.get('{' + MC + '}Ignorable') != 'w14' or 'w14' in tree.nsmap:
            raise ValueError('Changed Mac parent requires fresh original Word observations')
        root_at = styles.index(b'<', styles.index(b'?>') + 2)
        end = styles.index(b'>', root_at)
        styles = styles[:end] + b' xmlns:w14="' + W14.encode() + b'"' + styles[end:]
        original = ET.fromstring(parent.read('word/document.xml'))
        body = original.find(q('body'))
        heading, template, section = body[0], body[1], body[-1]
        settings = ET.fromstring(parent.read('word/settings.xml'))
        settings_bytes = {14: parent.read('word/settings.xml')}
        setting, = [e for e in settings.iter(q('compatSetting'))
                    if e.get(q('name')) == 'compatibilityMode']
        assert setting.get(q('val')) == '14'
        setting.set(q('val'), '15')
        settings_bytes[15] = ET.tostring(settings, encoding='utf-8', xml_declaration=True)
        for mode in (14, 15):
            name = 'word-windows-adjacent-fixed-table-grids-bound-namespace'
            if mode == 15:
                name += '-compat15'
            yield name, _archive(parent, {'word/styles.xml': styles,
                                         'word/settings.xml': settings_bytes[mode]})
        for size, label in [(10.5, '10p5'), (12, '12'), (20, '20'), (40, '40')]:
            for style, bold, italic in STYLES:
                for mode in (14, 15):
                    document = deepcopy(original)
                    output = document.find(q('body'))
                    output.clear()
                    output.append(deepcopy(heading))
                    for line, rule in LINE_RULES:
                        paragraph = deepcopy(template)
                        spacing = paragraph.find(q('pPr')).find(q('spacing'))
                        spacing.set(q('line'), str(line))
                        spacing.set(q('lineRule'), rule)
                        run, = paragraph.findall(q('r'))
                        props = run.find(q('rPr'))
                        for tag in ('sz', 'szCs'):
                            props.find(q(tag)).set(q('val'), str(int(size * 2)))
                        for tag, flag in [('b', bold), ('i', italic)]:
                            element = props.find(q(tag))
                            if element is None:
                                element = ET.SubElement(props, q(tag))
                            element.set(q('val'), '1' if flag else '0')
                        output.append(paragraph)
                    output.append(deepcopy(section))
                    name = f'word-windows-courier-line-size{label}-{style}-compat{mode}'
                    yield name, _archive(parent, {
                        'word/styles.xml': styles,
                        'word/document.xml': ET.tostring(document, encoding='utf-8', xml_declaration=True),
                        'word/settings.xml': settings_bytes[mode],
                    })


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output_directory', type=Path)
    args = parser.parse_args()
    args.output_directory.mkdir(parents=True, exist_ok=False)
    for name, raw in probes():
        (args.output_directory / (name + '.docx')).write_bytes(raw)
