"""Isolate authored, inherited and default table indentation in Windows Word."""
import argparse
import hashlib
from pathlib import Path
import re
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
SOURCE_SHA256 = 'd6c325b84a42e5df290543623f000e00eafdbdbe72322b2f56fd391ba7a37d69'


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
    return len(cases) + 9


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-directory', type=Path, required=True)
    args = parser.parse_args()
    print('Generated', generate(args.output_directory), 'table style probes')
