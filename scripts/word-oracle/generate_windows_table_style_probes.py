"""Isolate authored, inherited and default table indentation in Windows Word."""
import argparse
import hashlib
from pathlib import Path
import re
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
SOURCE_SHA256 = 'd6c325b84a42e5df290543623f000e00eafdbdbe72322b2f56fd391ba7a37d69'
LEGACY_SHA256 = 'ca8e006695ecbc489b9a883003d803d9169c902289a6110937a535bbb762f55e'


def indent(value):
    if value is None:
        return ''
    width = value if isinstance(value, int) else 800
    kind = value if isinstance(value, str) else 'dxa'
    return f'<w:tblInd w:w="{width}" w:type="{kind}"/>'


def label(value):
    return 'omitted' if value is None else str(value)


def write_parts(target, parts):
    with ZipFile(target, 'w', compression=ZIP_DEFLATED, compresslevel=9) as archive:
        for name, data in sorted(parts.items()):
            if name.endswith(('.xml', '.rels')):
                ET.fromstring(data)
            info = ZipInfo(name, (2000, 1, 1, 0, 0, 0))
            info.create_system = 0
            info.compress_type = ZIP_DEFLATED
            archive.writestr(info, data)


def generate(output):
    source = ROOT / 'public/fixtures/header-footer-edge-cases.docx'
    if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA256:
        raise ValueError('Original header/footer source changed')
    with ZipFile(source) as archive:
        original = {name: archive.read(name) for name in archive.namelist()}
    output.mkdir(parents=True, exist_ok=False)
    cases = [(mode, parent, None, direct) for mode in (12, 14, 15)
             for parent in (None, 0) for direct in (None, 0)]
    cases += [(14, 240, child, direct) for child, direct in
              [(None, None), (0, None), ('nil', None), ('auto', None),
               ('pct', None), (None, 120), (None, 'auto')]]
    for mode, parent, child, direct in cases:
        parts = dict(original)
        document = parts['word/document.xml'].decode()
        root = document[:document.index('<w:body>')]
        section, = re.findall(r'<w:sectPr\b.*?</w:sectPr>', document, re.S)
        section = re.sub(r'<w:(?:headerReference|footerReference)\b[^>]*/>', '', section)
        run_properties = ('<w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" '
                          'w:eastAsia="Arial" w:cs="Arial"/><w:sz w:val="24"/>'
                          '<w:szCs w:val="24"/><w:kern w:val="0"/></w:rPr>')
        rows = ''
        for cells in (('AB', 'CD'), ('EF', 'GH')):
            rows += '<w:tr>' + ''.join(
                '<w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr>'
                '<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" '
                'w:lineRule="auto"/>' + run_properties + '</w:pPr><w:r>'
                + run_properties + '<w:t>' + text + '</w:t></w:r></w:p></w:tc>'
                for text in cells) + '</w:tr>'
        table = ('<w:tbl><w:tblPr><w:tblStyle w:val="ProbeChild"/>'
                 '<w:tblW w:w="4000" w:type="dxa"/><w:jc w:val="left"/>'
                 + indent(direct) + '<w:tblLayout w:type="fixed"/><w:tblCellMar>'
                 '<w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/>'
                 '<w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/>'
                 '</w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/>'
                 '<w:gridCol w:w="2000"/></w:tblGrid>' + rows + '</w:tbl>')
        parts['word/document.xml'] = (root + '<w:body>' + table + section
                                      + '</w:body></w:document>').encode()
        styles = parts['word/styles.xml'].decode()
        assert styles.count('</w:styles>') == 1
        extra = ('<w:style w:type="table" w:styleId="ProbeParent">'
                 '<w:name w:val="Probe Parent"/><w:tblPr>' + indent(parent)
                 + '</w:tblPr></w:style><w:style w:type="table" w:styleId="ProbeChild">'
                 '<w:name w:val="Probe Child"/><w:basedOn w:val="ProbeParent"/>'
                 '<w:tblPr>' + indent(child) + '</w:tblPr></w:style>')
        parts['word/styles.xml'] = styles.replace('</w:styles>', extra + '</w:styles>').encode()
        settings, count = re.subn(
            r'(<w:compatSetting w:name="compatibilityMode"[^>]*w:val=")\d+',
            lambda match: match[1] + str(mode), parts['word/settings.xml'].decode())
        assert count == 1
        parts['word/settings.xml'] = settings.encode()
        name = (f'word-windows-edge-table-style-parent{label(parent)}-child{label(child)}'
                f'-direct{label(direct)}-compat{mode}')
        write_parts(output / (name + '.docx'), parts)

    for mode in (12, 14, 15):
        name = f'word-windows-edge-table-style-parent0-childomitted-directomitted-compat{mode}'
        with ZipFile(output / (name + '.docx')) as archive:
            original = {name: archive.read(name) for name in archive.namelist()}
        for variant in ('default-zero', 'default-omitted', 'no-styles'):
            parts = dict(original)
            parts['word/document.xml'] = parts['word/document.xml'].replace(
                b'<w:tblStyle w:val="ProbeChild"/>', b'')
            styles = re.sub(r'<w:style\b[^>]*w:styleId="Probe(?:Parent|Child)".*?</w:style>',
                            '', parts['word/styles.xml'].decode(), flags=re.S)
            if variant == 'default-omitted':
                styles = re.sub(r'<w:tblInd\b[^>]*/>', '', styles)
            parts['word/styles.xml'] = styles.encode()
            if variant == 'no-styles':
                del parts['word/styles.xml']
                parts['word/_rels/document.xml.rels'] = re.sub(
                    rb'<Relationship\b(?=[^>]*Type="[^"]*/styles")[^>]*/>', b'',
                    parts['word/_rels/document.xml.rels'])
                parts['[Content_Types].xml'] = re.sub(
                    rb'<Override\b(?=[^>]*PartName="/word/styles.xml")[^>]*/>', b'',
                    parts['[Content_Types].xml'])
            name = f'word-windows-edge-table-style-{variant}-directomitted-compat{mode}'
            write_parts(output / (name + '.docx'), parts)
    generate_followups(output)
    return len(list(output.glob('*.docx')))


def read_parts(source):
    with ZipFile(source) as archive:
        return {name: archive.read(name) for name in archive.namelist()}


def remove_effects(parts):
    del parts['word/stylesWithEffects.xml']
    parts['word/_rels/document.xml.rels'] = re.sub(
        rb'<Relationship\b(?=[^>]*Type="[^"]*/stylesWithEffects")[^>]*/>', b'',
        parts['word/_rels/document.xml.rels'])
    parts['[Content_Types].xml'] = re.sub(
        rb'<Override\b(?=[^>]*PartName="/word/stylesWithEffects.xml")[^>]*/>', b'',
        parts['[Content_Types].xml'])


def set_normal_indent(data, value):
    def edit(match):
        text, count = re.subn(r'<w:tblInd\b[^>]*/>', indent(value), match[0])
        assert count == 1
        return text
    text, count = re.subn(r'<w:style\b[^>]*w:styleId="TableNormal".*?</w:style>',
                          edit, data.decode(), flags=re.S)
    assert count == 1
    return text.encode()


def generate_followups(output):
    """Vary one source feature at a time; no renderer output enters these inputs."""
    prefix = 'word-windows-edge-table-style-'

    def source(suffix):
        return read_parts(output / (prefix + suffix + '.docx'))

    def write(suffix, parts):
        write_parts(output / (prefix + suffix + '.docx'), parts)

    legacy = ROOT / 'public/fixtures/word-windows-edge-table-indents-compat14.docx'
    assert hashlib.sha256(legacy.read_bytes()).hexdigest() == LEGACY_SHA256
    old = read_parts(legacy)

    for child in ('omitted', 'nil'):
        original = source(f'parent240-child{child}-directomitted-compat14')
        for mode in (12, 15):
            parts = dict(original)
            data, count = re.subn(
                rb'(<w:compatSetting w:name="compatibilityMode"[^>]*w:val=")14',
                lambda match: match[1] + str(mode).encode(), parts['word/settings.xml'])
            assert count == 1
            parts['word/settings.xml'] = data
            write(f'parent240-child{child}-directomitted-compat{mode}', parts)

    normal = source('default-zero-directomitted-compat14')
    for kind in ('nil', 'auto', 'pct'):
        parts = dict(normal)
        parts['word/styles.xml'] = set_normal_indent(parts['word/styles.xml'], kind)
        write(f'default-{kind}-directomitted-compat14', parts)
    for variant in ('no-table-default', 'custom-empty-default'):
        parts = dict(normal)
        styles = re.sub(r'<w:style\b(?=[^>]*w:type="table")[^>]*>.*?</w:style>',
                        '', parts['word/styles.xml'].decode(), flags=re.S)
        if variant == 'custom-empty-default':
            styles = styles.replace('</w:styles>', '<w:style w:type="table" w:default="1" '
                                    'w:styleId="ProbeDefault"><w:name w:val="Probe Default"/>'
                                    '</w:style></w:styles>')
        parts['word/styles.xml'] = styles.encode()
        write(f'{variant}-directomitted-compat14', parts)
    bare_primary = source('no-table-default-directomitted-compat14')

    for variant in ('styles-minimal', 'settings-minimal', 'styles-and-settings-minimal',
                    'styles-no-latent', 'snap-off'):
        parts = dict(bare_primary)
        if variant in ('styles-minimal', 'styles-and-settings-minimal'):
            parts['word/styles.xml'] = old['word/styles.xml']
        if variant in ('settings-minimal', 'styles-and-settings-minimal'):
            parts['word/settings.xml'] = old['word/settings.xml']
        if variant == 'styles-no-latent':
            data, count = re.subn(rb'<w:latentStyles\b[^>]*>.*?</w:latentStyles>', b'',
                                  parts['word/styles.xml'], flags=re.S)
            assert count == 1
            parts['word/styles.xml'] = data
        if variant == 'snap-off':
            parts['word/document.xml'] = parts['word/document.xml'].replace(
                b'<w:pPr>', b'<w:pPr><w:snapToGrid w:val="0"/>')
        write(f'bisect-{variant}-compat14', parts)

    for variant in ('positive', 'nil', 'before-paragraph', 'old-without-borders'):
        parts = dict(bare_primary)
        if variant in ('positive', 'nil'):
            style = 'single' if variant == 'positive' else 'nil'
            borders = '<w:tblBorders>' + ''.join(
                f'<w:{side} w:val="{style}" w:sz="4" w:color="000000"/>'
                for side in ('top', 'left', 'bottom', 'right', 'insideH', 'insideV'))
            borders += '</w:tblBorders>'
            parts['word/document.xml'] = parts['word/document.xml'].replace(
                b'<w:tblLayout', borders.encode() + b'<w:tblLayout')
        elif variant == 'before-paragraph':
            document = parts['word/document.xml'].decode()
            properties, = set(re.findall(r'<w:rPr>.*?</w:rPr>', document, re.S))
            before = ('<w:p><w:pPr><w:snapToGrid w:val="0"/>'
                      '<w:spacing w:before="0" w:after="0"/>' + properties
                      + '</w:pPr><w:r>' + properties + '<w:t>Start</w:t></w:r></w:p>')
            parts['word/document.xml'] = document.replace('<w:body>', '<w:body>' + before).encode()
        else:
            parts = dict(old)
            data, count = re.subn(rb'<w:tblBorders>.*?</w:tblBorders>', b'',
                                  parts['word/document.xml'], flags=re.S)
            assert count == 12
            parts['word/document.xml'] = data
        write(f'border-{variant}-compat14', parts)

    for origin, original, pairs in (
            ('new', bare_primary, [(100, 108), (108, 160), (100, 160), (120, 108), (0, 108)]),
            ('old', old, [(108, 160), (100, 108), (108, 108)])):
        for left, right in pairs:
            parts = dict(original)
            def margins(match):
                text, count = re.subn(rb'(<w:left w:w=")[^"]+',
                                      lambda m: m[1] + str(left).encode(), match[0])
                assert count == 1
                text, count = re.subn(rb'(<w:right w:w=")[^"]+',
                                      lambda m: m[1] + str(right).encode(), text)
                assert count == 1
                return text
            data, count = re.subn(rb'<w:tblCellMar>.*?</w:tblCellMar>', margins,
                                  parts['word/document.xml'], flags=re.S)
            assert count == (1 if origin == 'new' else 12)
            parts['word/document.xml'] = data
            write(f'margin-{origin}-left{left}-right{right}-compat14', parts)

    new_document = source('bisect-styles-and-settings-minimal-compat14')['word/document.xml'].decode()
    old_document = old['word/document.xml'].decode()
    old_section, = re.findall(r'<w:sectPr\b.*?</w:sectPr>', old_document, re.S)
    first_body = re.search(r'<w:body>(.*?</w:tbl>)', old_document, re.S)[1]
    for variant in ('rich-root', 'plain-root', 'plain-root-old-section',
                    'old-first-table-only', 'old-first-table-auto-row'):
        parts = dict(old)
        document = new_document
        if variant.startswith('plain-root'):
            document = re.sub(r'<w:document\b[^>]*>', '<w:document xmlns:w="http://schemas.'
                              'openxmlformats.org/wordprocessingml/2006/main">', document, count=1)
        if variant == 'plain-root-old-section':
            document = re.sub(r'<w:sectPr\b.*?</w:sectPr>', old_section, document, flags=re.S)
        if variant.startswith('old-first-table'):
            body = first_body
            if variant.endswith('auto-row'):
                body = re.sub(r'<w:trPr>.*?</w:trPr>', '', body, flags=re.S)
            document = ('<w:document xmlns:w="http://schemas.openxmlformats.org/'
                        'wordprocessingml/2006/main"><w:body>' + body + old_section
                        + '</w:body></w:document>')
        parts['word/document.xml'] = document.encode()
        write(f'package-{variant}-compat14', parts)

    effects_cases = [
        ('no-table-default-no-effects', 'no-table-default', 'remove'),
        ('default-nil-no-effects', 'default-nil', 'remove'),
        ('default-omitted-no-effects', 'default-omitted', 'remove'),
        ('no-styles-no-effects', 'no-styles', 'remove'),
        ('primary-nil-effects240', 'default-nil', 'change-effects'),
        ('primary0-effects240', 'default-zero', 'change-effects'),
        ('primary-omitted-effects240', 'default-omitted', 'change-effects'),
        ('primary240-effects0', 'default-zero', 'change-primary')]
    for name, base, operation in effects_cases:
        parts = source(base + '-directomitted-compat14')
        if operation == 'remove':
            remove_effects(parts)
        else:
            member = ('word/stylesWithEffects.xml' if operation == 'change-effects'
                      else 'word/styles.xml')
            parts[member] = set_normal_indent(parts[member], 240)
        write(f'effects-{name}-compat14', parts)

    for variant in ('normal240-no-effects-implicit', 'normal240-no-effects-explicit',
                    'normal240-effects240-implicit', 'custom-default240-no-effects'):
        parts = dict(normal)
        parts['word/styles.xml'] = set_normal_indent(parts['word/styles.xml'], 240)
        if variant == 'normal240-effects240-implicit':
            parts['word/stylesWithEffects.xml'] = set_normal_indent(parts['word/stylesWithEffects.xml'], 240)
        else:
            remove_effects(parts)
        if variant == 'normal240-no-effects-explicit':
            parts['word/document.xml'] = parts['word/document.xml'].replace(
                b'<w:tblPr>', b'<w:tblPr><w:tblStyle w:val="TableNormal"/>')
        if variant == 'custom-default240-no-effects':
            parts['word/styles.xml'] = parts['word/styles.xml'].replace(
                b'w:styleId="TableNormal"', b'w:styleId="ProbeDefault"').replace(
                b'w:val="Normal Table"', b'w:val="Probe Default"')
        write(f'dual-{variant}-compat14', parts)
    for variant, primary, secondary in (
            ('primary240-effects480', 240, 480), ('primary-empty-effects480', 'empty', 480),
            ('primary-nil-effects480', 'nil', 480), ('primary-absent-effects480', None, 480),
            ('primary240-effects-absent', 240, None)):
        parts = dict(bare_primary)
        parts['word/document.xml'] = parts['word/document.xml'].replace(
            b'<w:tblPr>', b'<w:tblPr><w:tblStyle w:val="ProbeDual"/>')
        for member, value in (('word/styles.xml', primary), ('word/stylesWithEffects.xml', secondary)):
            if value is None:
                continue
            properties = '' if value == 'empty' else indent(value)
            style = ('<w:style w:type="table" w:styleId="ProbeDual">'
                     '<w:name w:val="Probe Dual"/><w:tblPr>' + properties + '</w:tblPr></w:style>')
            assert parts[member].count(b'</w:styles>') == 1
            parts[member] = parts[member].replace(b'</w:styles>', style.encode() + b'</w:styles>')
        write(f'dual-{variant}-compat14', parts)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-directory', type=Path, required=True)
    args = parser.parse_args()
    print('Generated', generate(args.output_directory), 'table style probes')
