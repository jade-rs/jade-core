"""Reproduce Windows numeric spacing and compatibility controls from a Mac DOCX.

These are inputs only. Coordinate expectations must be captured from Word.
"""
import argparse
from copy import deepcopy
from io import BytesIO
from pathlib import Path
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

from lxml import etree as ET

ROOT = Path(__file__).resolve().parents[2]
PARENT = Path('tests/word-oracle/mac-elsa-synthetic/aptos-numeric-ranges-mac/source.docx')
W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'
W14 = 'http://schemas.microsoft.com/office/word/2010/wordml'
STYLES = [('regular', False, False), ('italic', False, True),
          ('bold', True, False), ('bolditalic', True, True)]
FAMILIES = [('Aptos', 'aptos'), ('Aptos Display', 'display'),
            ('Arial', 'arial'), ('Calibri', 'calibri')]
NUMERIC_TEXT = '第11 表1-12 11第 1-12表'
KERNING_TEXT = 'AV To W/o X-ray A-V T. T, 1-1 1-12 2-11 (-12) 1234567890'


def specifications():
    for mode in (14, 15):
        for family, label in FAMILIES:
            for hint in ('absent', 'default', 'eastasia', 'explicit-ea'):
                yield dict(group='hints', name=f'word-windows-numeric-hints-{label}-size14-{hint}-compat{mode}',
                           family=family, size=14, bold=False, italic=False, hint=hint,
                           mode=mode, text=NUMERIC_TEXT)
    for family, label in FAMILIES:
        for size in ((12, 14, 20, 40) if label in ('aptos', 'display') else (14,)):
            for style, bold, italic in STYLES:
                if size == 14 and style == 'regular':
                    continue
                yield dict(group='styles', name=f'word-windows-numeric-styles-{label}-size{size}-{style}-compat14',
                           family=family, size=size, bold=bold, italic=italic, hint='absent',
                           mode=14, text=NUMERIC_TEXT)
    for size in (12, 14, 20, 40):
        for style, bold, italic in STYLES:
            yield dict(group='cjk', name=f'word-windows-inherited-yu-cells-aptos-size{size}-{style}-compat14',
                       family='Aptos', size=size, bold=bold, italic=italic, hint='absent',
                       mode=14, text='第表天地文章日本語漢字' * 3)
    for family, label in FAMILIES[:2]:
        for style, bold, italic in STYLES:
            for mode in (14, 15):
                yield dict(group='kerning', name=f'word-windows-aptos-number-kerning-{label}-{style}-compat{mode}',
                           family=family, size=14, bold=bold, italic=italic,
                           mode=mode, text=KERNING_TEXT)


def probes(root=ROOT):
    q = lambda name: '{' + W + '}' + name
    q14 = lambda name: '{' + W14 + '}' + name
    with ZipFile(root / PARENT) as parent:
        original = ET.fromstring(parent.read('word/document.xml'))
        template = original.find(q('body')).find(q('p'))
        section = original.find(q('body')).find(q('sectPr'))
        for case in specifications():
            document = deepcopy(original)
            output = document.find(q('body'))
            output.clear()
            variants = ([dict(typography=t, kern=k) for t in
                         ('absent', 'default', 'tabular', 'proportional', 'standard-ligatures')
                         for k in (200, 2)] if case['group'] == 'kerning'
                        else [dict(dn=dn, kern=200) for dn in (None, True, False)])
            for variant in variants:
                paragraph = deepcopy(template)
                if case['group'] != 'kerning':
                    ppr = paragraph.find(q('pPr'))
                    for child in list(ppr):
                        if child.tag == q('autoSpaceDN'):
                            ppr.remove(child)
                    if variant['dn'] is not None:
                        ET.SubElement(ppr, q('autoSpaceDN')).set(q('val'), '1' if variant['dn'] else '0')
                run = paragraph.find(q('r'))
                props = run.find(q('rPr'))
                fonts = props.find(q('rFonts'))
                for attribute in ('ascii', 'hAnsi'):
                    fonts.set(q(attribute), case['family'])
                if case['group'] != 'kerning':
                    fonts.attrib.pop(q('hint'), None)
                    fonts.attrib.pop(q('eastAsia'), None)
                    if case['hint'] in ('default', 'eastasia'):
                        fonts.set(q('hint'), 'default' if case['hint'] == 'default' else 'eastAsia')
                    if case['hint'] == 'explicit-ea':
                        fonts.set(q('eastAsia'), 'Yu Mincho')
                props.find(q('sz')).set(q('val'), str(case['size'] * 2))
                props.find(q('b')).set(q('val'), '1' if case['bold'] else '0')
                props.find(q('i')).set(q('val'), '1' if case['italic'] else '0')
                props.find(q('kern')).set(q('val'), str(variant['kern']))
                if case['group'] == 'kerning':
                    typography = variant['typography']
                    if typography in ('default', 'tabular', 'proportional'):
                        ET.SubElement(props, q14('numSpacing')).set(q14('val'), typography)
                    if typography == 'standard-ligatures':
                        ET.SubElement(props, q14('ligatures')).set(q14('val'), 'standard')
                run.find(q('t')).text = case['text']
                output.append(paragraph)
            output.append(deepcopy(section))
            xml = ET.tostring(document, encoding='utf-8', xml_declaration=True)
            settings = parent.read('word/settings.xml')
            if case['mode'] != 14:
                tree = ET.fromstring(settings)
                setting, = [e for e in tree.iter(q('compatSetting')) if e.get(q('name')) == 'compatibilityMode']
                setting.set(q('val'), str(case['mode']))
                settings = ET.tostring(tree, encoding='utf-8', xml_declaration=True)
            buffer = BytesIO()
            day = 18 if case['group'] == 'kerning' else 17
            with ZipFile(buffer, 'w', compression=ZIP_DEFLATED) as bundle:
                for entry in parent.infolist():
                    item = ZipInfo(entry.filename, (2026, 11, day, 0, 0, 0))
                    item.create_system = 0
                    item.compress_type = ZIP_DEFLATED
                    item.external_attr = entry.external_attr
                    data = (xml if entry.filename == 'word/document.xml' else settings
                            if entry.filename == 'word/settings.xml' else parent.read(entry.filename))
                    bundle.writestr(item, data)
            yield case['name'], buffer.getvalue()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output_directory', type=Path)
    args = parser.parse_args()
    args.output_directory.mkdir(parents=True, exist_ok=False)
    for name, raw in probes():
        (args.output_directory / (name + '.docx')).write_bytes(raw)
