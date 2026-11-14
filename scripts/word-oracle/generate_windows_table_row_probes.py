"""Separate built-in style identity and first-row table-indent exceptions."""
from pathlib import Path
import re
import shutil

from generate_windows_table_style_probes import read_parts, write_parts

import argparse
import hashlib

ROOT = Path(__file__).resolve().parents[2]
SOURCE_SHA256 = {'word-windows-edge-table-style-dual-normal240-no-effects-implicit-compat14.docx': '38197b685752fc3be47b82f79d8d0d6b8e792d95def79a44516df0af2405f377',
 'word-windows-edge-table-style-parent0-childomitted-directomitted-compat14.docx': 'd44f6e4052f2e99070dde72fa62330c6907e11bd24943a1fcc27d09673be6d66',
 'word-windows-edge-table-style-parent240-childomitted-directomitted-compat14.docx': '37698c3c3e98cbf9650ea719fdf1c1655dbf4b7dc34e469c015e2ff2096443bd',
 'word-windows-edge-table-style-dual-normal240-effects240-implicit-compat14.docx': '3ecaf6dca9179c159a5c5cdb9ad98cec038c3112d2d658dd68a6b5df71ee12f2'}


def generate(output):
    for name, expected in SOURCE_SHA256.items():
        assert hashlib.sha256((ROOT / 'public/fixtures' / name).read_bytes()).hexdigest() == expected
    output.mkdir(parents=True, exist_ok=False)
    base = ROOT / 'public/fixtures'
    control = base / 'word-windows-edge-table-style-dual-normal240-no-effects-implicit-compat14.docx'
    shutil.copyfile(control, output / control.name)
    original = read_parts(control)
    for identity in ('normal', 'probe'):
        for name in ('normal', 'probe'):
            for custom in (None, 0, 1):
                parts = dict(original)
                def edit(match):
                    block = match[0]
                    style_id = 'TableNormal' if identity == 'normal' else 'ProbeIdentity'
                    style_name = 'Normal Table' if name == 'normal' else 'Probe Named Table'
                    block = block.replace('w:styleId="TableNormal"', f'w:styleId="{style_id}"', 1)
                    block, count = re.subn(r'<w:name\b[^>]*/>',
                                           f'<w:name w:val="{style_name}"/>', block, count=1)
                    assert count == 1
                    end = block.index('>')
                    tag = re.sub(r' w:customStyle="[^"]*"', '', block[:end])
                    if custom is not None:
                        tag += f' w:customStyle="{custom}"'
                    return tag + block[end:]
                styles, count = re.subn(r'<w:style\b[^>]*w:styleId="TableNormal".*?</w:style>',
                                        edit, parts['word/styles.xml'].decode(), flags=re.S)
                assert count == 1
                parts['word/styles.xml'] = styles.encode()
                label = 'omitted' if custom is None else str(custom)
                target = output / f'word-windows-edge-table-identity-id{identity}-name{name}-custom{label}-compat14.docx'
                write_parts(target, parts)


    def row_exception(parent, row, value, kind, mode, alignment='left'):
        source = base / f'word-windows-edge-table-style-parent{parent}-childomitted-directomitted-compat14.docx'
        parts = read_parts(source)
        document = parts['word/document.xml'].decode()
        rows = list(re.finditer(r'<w:tr>', document))
        assert len(rows) == 2
        point = rows[row].end()
        exception = f'<w:tblPrEx><w:tblInd w:w="{value}" w:type="{kind}"/></w:tblPrEx>'
        document = document[:point] + exception + document[point:]
        if alignment != 'left':
            document, count = re.subn(r'<w:jc w:val="left"/>',
                                      f'<w:jc w:val="{alignment}"/>', document)
            assert count == 1
        parts['word/document.xml'] = document.encode()
        settings, count = re.subn(rb'(<w:compatSetting w:name="compatibilityMode"[^>]*w:val=")\d+',
                                  lambda m: m[1] + str(mode).encode(), parts['word/settings.xml'])
        assert count == 1
        parts['word/settings.xml'] = settings
        target = output / (f'word-windows-edge-table-row-indent-parent{parent}-row{row + 1}'
                           f'-{kind}{value}-{alignment}-compat{mode}.docx')
        write_parts(target, parts)


    for mode in (12, 14, 15):
        for parent in (0, 240):
            for row, value in ((0, 0), (0, 240), (1, 240 if parent == 0 else 0)):
                row_exception(parent, row, value, 'dxa', mode)
    for alignment in ('center', 'right'):
        for row, value in ((0, 0), (0, 240), (1, 240)):
            row_exception(0, row, value, 'dxa', 14, alignment)
    for mode in (12, 15):
        for parent in (0, 240):
            for value in (0, 240):
                for alignment in ('center', 'right'):
                    row_exception(parent, 0, value, 'dxa', mode, alignment)
    for parent in (0, 240):
        for kind in ('nil', 'auto', 'pct'):
            row_exception(parent, 0, 800, kind, 14)


    for parent in (0, 240):
        for kind, value in [('dxa', 0), ('dxa', 120), ('auto', 800)]:
            source = output / f'word-windows-edge-table-row-indent-parent{parent}-row1-dxa240-left-compat14.docx'
            parts = read_parts(source)
            document = parts['word/document.xml'].decode()
            properties = re.search(r'<w:tblPr>.*?</w:tblPr>', document, re.S)
            assert properties and '<w:tblInd' not in properties[0]
            replacement = properties[0].replace('<w:tblLayout', f'<w:tblInd w:w="{value}" w:type="{kind}"/><w:tblLayout', 1)
            assert replacement != properties[0]
            document = document[:properties.start()] + replacement + document[properties.end():]
            parts['word/document.xml'] = document.encode()
            write_parts(output / f'word-windows-edge-table-row-direct-parent{parent}-row1-dxa240-direct{kind}{value}-compat14.docx', parts)
    for identity in ('normal', 'probe'):
        for custom in ('omitted', '0', '1'):
            source = output / f'word-windows-edge-table-identity-id{identity}-namenormal-custom{custom}-compat14.docx'
            parts = read_parts(source)
            styles, count = re.subn(r'<w:name w:val="Normal Table"/>', '', parts['word/styles.xml'].decode(), count=1)
            assert count == 1
            parts['word/styles.xml'] = styles.encode()
            write_parts(output / f'word-windows-edge-table-identity-id{identity}-nameomitted-custom{custom}-compat14.docx', parts)
    # Keep authored primary definitions when the effects copy has the same id.
    control = base / 'word-windows-edge-table-style-dual-normal240-effects240-implicit-compat14.docx'
    shutil.copyfile(control, output / control.name)
    original = read_parts(control)
    for identity in ('normal', 'probe'):
        for name in ('normal', 'probe', 'omitted'):
            for custom in (None, 0, 1):
                parts = dict(original)
                def edit_effects_primary(match):
                    block = match[0]
                    style_id = 'TableNormal' if identity == 'normal' else 'ProbeIdentity'
                    block = block.replace('w:styleId="TableNormal"', f'w:styleId="{style_id}"', 1)
                    style_name = 'Normal Table' if name == 'normal' else 'Probe Named Table'
                    replacement = '' if name == 'omitted' else f'<w:name w:val="{style_name}"/>'
                    block, count = re.subn(r'<w:name\b[^>]*/>', replacement, block, count=1)
                    assert count == 1
                    end = block.index('>')
                    tag = re.sub(r' w:customStyle="[^"]*"', '', block[:end])
                    if custom is not None:
                        tag += f' w:customStyle="{custom}"'
                    return tag + block[end:]
                styles, count = re.subn(r'<w:style\b[^>]*w:styleId="TableNormal".*?</w:style>',
                                       edit_effects_primary, parts['word/styles.xml'].decode(), flags=re.S)
                assert count == 1
                parts['word/styles.xml'] = styles.encode()
                label = 'omitted' if custom is None else str(custom)
                write_parts(output / f'word-windows-edge-table-identity-effects-id{identity}-name{name}-custom{label}-compat14.docx', parts)
    return len(list(output.glob('*.docx')))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-directory', type=Path, required=True)
    args = parser.parse_args()
    print('Generated', generate(args.output_directory), 'table identity/row indentation controls')
