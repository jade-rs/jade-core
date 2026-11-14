"""Reproduce manual-break spacing controls from a pinned original DOCX."""
import argparse
import hashlib
from pathlib import Path
import re
from zipfile import ZipFile

from generate_windows_table_style_probes import write_parts

ROOT = Path(__file__).resolve().parents[2]
SOURCE_SHA256 = 'd6c325b84a42e5df290543623f000e00eafdbdbe72322b2f56fd391ba7a37d69'


def read_parts(source):
    with ZipFile(source) as archive:
        return {name: archive.read(name) for name in archive.namelist()}


def generate(output):
    source = ROOT / 'public/fixtures/header-footer-edge-cases.docx'
    assert hashlib.sha256(source.read_bytes()).hexdigest() == SOURCE_SHA256
    original = read_parts(source)
    document = original['word/document.xml'].decode()
    root = document[:document.index('<w:body>')]
    section, = re.findall(r'<w:sectPr\b.*?</w:sectPr>', document, re.S)
    section = re.sub(r'<w:(?:headerReference|footerReference)\b[^>]*/>', '', section)
    properties = ('<w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="Arial" '
                  'w:cs="Arial"/><w:sz w:val="24"/><w:szCs w:val="24"/>'
                  '<w:spacing w:val="0"/><w:kern w:val="0"/></w:rPr>')

    def paragraph(text, options='', run_suffix=''):
        return ('<w:p><w:pPr>' + options + '<w:spacing w:after="0" w:line="240" '
                'w:lineRule="auto"/>' + properties + '</w:pPr><w:r>' + properties
                + '<w:t>' + text + '</w:t>' + run_suffix + '</w:r></w:p>')

    output.mkdir(parents=True, exist_ok=False)
    cases = [(mode, before, kind) for mode in (12, 14, 15) for before in (0, 480)
             for kind in ('break-paragraph', 'break-inline')]
    cases += [(14, before, kind) for before in (0, 480)
              for kind in ('first-page', 'page-break-before')]
    for mode, before, kind in cases:
        parts = dict(original)
        heading = paragraph('Second page heading', '<w:pStyle w:val="Heading1"/>'
                            + ('<w:pageBreakBefore/>' if kind == 'page-break-before' else ''))
        heading = heading.replace('<w:spacing ', f'<w:spacing w:before="{before}" ')
        if kind == 'first-page':
            body = heading
        elif kind == 'break-inline':
            body = paragraph('Start', run_suffix='<w:br w:type="page"/>') + heading
        elif kind == 'break-paragraph':
            body = paragraph('Start') + paragraph('', run_suffix='<w:br w:type="page"/>') + heading
        else:
            body = paragraph('Start') + heading
        parts['word/document.xml'] = (root + '<w:body>' + body + paragraph('End') + section
                                      + '</w:body></w:document>').encode()
        data, count = re.subn(rb'(<w:compatSetting w:name="compatibilityMode"[^>]*w:val=")\d+',
                              lambda m: m[1] + str(mode).encode(), parts['word/settings.xml'])
        assert count == 1
        parts['word/settings.xml'] = data
        write_parts(output / f'word-windows-edge-heading-{kind}-before{before}-compat{mode}.docx', parts)

    holdouts = [(mode, kind, 'normal') for mode in (12, 14, 15)
                for kind in ('break-inline', 'break-paragraph')]
    holdouts += [(14, kind, variation) for kind in ('break-inline', 'break-paragraph')
                 for variation in ('direct-page-before', 'inherited-page-before')]
    for mode, kind, variation in holdouts:
        name = f'word-windows-edge-heading-{kind}-before480-compat{mode}'
        parts = read_parts(output / (name + '.docx'))
        old = b'<w:pStyle w:val="Heading1"/>'
        assert parts['word/document.xml'].count(old) == 1
        if variation == 'normal':
            replacement = b''
        elif variation == 'direct-page-before':
            replacement = old + b'<w:pageBreakBefore/>'
        else:
            replacement = b'<w:pStyle w:val="ProbeHeadingBreak"/>'
            style = (b'<w:style w:type="paragraph" w:styleId="ProbeHeadingBreak">'
                     b'<w:name w:val="Probe Heading Break"/><w:basedOn w:val="Heading1"/>'
                     b'<w:pPr><w:pageBreakBefore/></w:pPr></w:style>')
            assert parts['word/styles.xml'].count(b'</w:styles>') == 1
            parts['word/styles.xml'] = parts['word/styles.xml'].replace(b'</w:styles>', style + b'</w:styles>')
        parts['word/document.xml'] = parts['word/document.xml'].replace(old, replacement)
        write_parts(output / f'word-windows-edge-heading-{kind}-{variation}-before480-compat{mode}.docx', parts)
    # Retain the original observation inputs byte for byte. Their historical
    # string replacement also placed paragraph-only before attributes inside
    # character-spacing elements. Only corrected inputs become accepted gates.
    for legacy in sorted(output.glob('*.docx')):
        parts = read_parts(legacy)
        def clean_run_properties(match):
            return re.sub(rb'(<w:spacing) w:before="\d+"', rb'\1', match[0])
        parts['word/document.xml'] = re.sub(
            rb'<w:rPr>.*?</w:rPr>', clean_run_properties,
            parts['word/document.xml'], flags=re.S)
        name = legacy.name.replace('-compat', '-ppr-only-compat')
        write_parts(output / name, parts)
    return 2 * (len(cases) + len(holdouts))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-directory', type=Path, required=True)
    args = parser.parse_args()
    print('Generated', generate(args.output_directory), 'manual-break spacing probes')
