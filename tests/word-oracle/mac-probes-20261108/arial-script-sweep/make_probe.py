#!/usr/bin/env python3
"""Compare ordinary and ruby Arial script offsets, 6–30 pt by half points."""
from pathlib import Path
from zipfile import ZipFile, ZIP_DEFLATED

ROOT = Path(__file__).parent
FONT = '<w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="Arial"/>'

def run(text, half_points, script=None):
    alignment = f'<w:vertAlign w:val="{script}"/>' if script else ''
    return (f'<w:r><w:rPr>{FONT}<w:sz w:val="{half_points}"/>'
            f'{alignment}</w:rPr><w:t xml:space="preserve">{text}</w:t></w:r>')

body = []
for half_points in range(12, 61):
    for script in ('superscript', 'subscript'):
        for kind in ('ordinary', 'ruby'):
            label = f'{kind}-{script}-{half_points}: '
            base = run('BASE', 24)
            guide = run('ABC', half_points, script)
            if kind == 'ruby':
                text = (f'<w:r><w:ruby><w:rubyPr><w:rubyAlign w:val="left"/>'
                        f'<w:hps w:val="{half_points}"/><w:hpsRaise w:val="30"/>'
                        f'<w:hpsBaseText w:val="24"/></w:rubyPr>'
                        f'<w:rt>{guide}</w:rt><w:rubyBase>{base}</w:rubyBase>'
                        '</w:ruby></w:r>')
            else:
                text = base + run(' ', 12) + guide
            body.append('<w:p><w:pPr><w:jc w:val="left"/></w:pPr>'
                        + run(label, 12) + text + '</w:p>')

template = ROOT.parent / 'ruby-position' / 'source-template.docx'
with ZipFile(template) as src, ZipFile(ROOT / 'arial-script-sweep.docx', 'w', ZIP_DEFLATED) as dst:
    for name in src.namelist():
        data = src.read(name)
        if name == 'word/document.xml':
            xml = data.decode()
            start = xml.index('<w:body>') + len('<w:body>')
            end = xml.index('<w:sectPr>')
            data = (xml[:start] + ''.join(body) + xml[end:]).encode()
        dst.writestr(name, data)
print(f'wrote {len(body)} paragraphs')
