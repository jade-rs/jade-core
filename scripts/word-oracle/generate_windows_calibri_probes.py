"""Isolate Windows Calibri requested-size advances and enabled kerning."""
import argparse
import hashlib
from pathlib import Path
import re
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

ROOT = Path(__file__).resolve().parents[2]
SOURCE_SHA256 = '89839c9589cf3e4e246360340397bb20124331fcd40ebb0305a157d483dafbe2'


def generate(output):
    source = ROOT / 'public/fixtures/footnote-long-continuation.docx'
    if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA256:
        raise ValueError('Original probe input changed')
    with ZipFile(source) as archive:
        original = {n: archive.read(n) for n in archive.namelist()}
    texts = ['SS', 'EE', 'PP', 'AA', 'RR', 'TT', 'OO', 'SE', 'EP', 'PA', 'AR',
             'RA', 'AT', 'TO', 'OR', 'SEPARATOR', 'AV', 'To', 'Wa', 'Wo',
             'ABCDEFGHIJKLMNOPQRSTUVWXYZ', 'Header and Footer Edge Cases']
    output.mkdir(parents=True, exist_ok=False)
    cases = [(size, kern, 0) for size in (21, 24, 40, 44) for kern in (0, 2)]
    cases += [(44, kern, spacing) for kern in (0, 2) for spacing in (5, 10)]
    for size, kern, spacing in cases:
        parts = dict(original)
        tracking = f'<w:spacing w:val="{spacing}"/>' if spacing else ''
        props = ('<w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" '
                 'w:eastAsia="Calibri" w:cs="Calibri"/>'
                 f'{tracking}<w:kern w:val="{kern}"/><w:sz w:val="{size}"/>'
                 f'<w:szCs w:val="{size}"/></w:rPr>')
        body = ''.join('<w:p><w:pPr><w:spacing w:before="0" w:after="0" '
                       'w:line="240" w:lineRule="auto"/>'
                       f'<w:jc w:val="left"/>{props}</w:pPr>'
                       f'<w:r>{props}<w:t>{text}</w:t></w:r></w:p>' for text in texts)
        text, count = re.subn(r'(<w:body>).*?(<w:sectPr)',
                             lambda m: m[1] + body + m[2],
                             parts['word/document.xml'].decode(), flags=re.S)
        assert count == 1
        parts['word/document.xml'] = re.sub(r'<w:docGrid\b[^>]*/>',
            '<w:docGrid w:type="default" w:linePitch="360"/>', text).encode()
        suffix = f'-spacing{spacing}' if spacing else ''
        name = f'word-windows-edge-calibri-kern{kern}-size{size}{suffix}-compat15'
        with ZipFile(output / (name + '.docx'), 'w', compression=ZIP_DEFLATED,
                     compresslevel=9) as archive:
            for member, data in sorted(parts.items()):
                info = ZipInfo(member, (2000, 1, 1, 0, 0, 0))
                info.compress_type = ZIP_DEFLATED
                archive.writestr(info, data)
    return len(cases)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-directory', type=Path, required=True)
    args = parser.parse_args()
    print('Generated', generate(args.output_directory), 'Calibri probes')
