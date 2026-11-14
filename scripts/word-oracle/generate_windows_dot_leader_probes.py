"""Isolate Windows dot leader pitch and phase using original tab DOCX parts."""
import argparse
import hashlib
from pathlib import Path
import re
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

ROOT = Path(__file__).resolve().parents[2]
SOURCE_SHA256 = '4b2ab628789bfb0502aa779951fe4f94358a782b5e8f1fdf9b2e8d90deb358de'


def generate(output):
    source = ROOT / 'public/fixtures/tabs.docx'
    if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA256:
        raise ValueError('Original tabs input changed')
    with ZipFile(source) as archive:
        original = {n: archive.read(n) for n in archive.namelist()}
    cases = [(slug, face, size, 0) for slug, face in [
        ('arial', 'Arial'), ('century', 'Century'),
        ('yu-mincho', 'Yu Mincho'), ('ms-mincho', 'MS Mincho')]
        for size in (21, 24, 40)]
    cases += [('yu-mincho', 'Yu Mincho', 21, indent) for indent in (63, 120, 240)]
    cases = [(slug, face, size, indent, 8000, False) for slug, face, size, indent in cases]
    cases += [('yu-mincho', 'Yu Mincho', 21, 0, stop, True)
              for stop in range(7988, 8013, 2)]
    output.mkdir(parents=True, exist_ok=False)
    for slug, face, size, indent, stop, boundary in cases:
        parts = dict(original)
        paragraph = (f'<w:p><w:pPr><w:tabs><w:tab w:val="right" w:leader="dot" '
                     f'w:pos="{stop}"/></w:tabs><w:ind w:left="{indent}"/></w:pPr>'
                     '<w:r><w:t>Item1</w:t></w:r><w:r><w:tab/></w:r>'
                     '<w:r><w:t>10,000</w:t></w:r></w:p>')
        props = (f'<w:rPr><w:rFonts w:ascii="{face}" w:hAnsi="{face}" '
                 f'w:eastAsia="{face}" w:cs="{face}"/><w:kern w:val="0"/>'
                 f'<w:sz w:val="{size}"/><w:szCs w:val="{size}"/></w:rPr>')
        paragraph = paragraph.replace('<w:r>', '<w:r>' + props)
        text, count = re.subn(r'(<w:body>).*?(<w:sectPr)',
                             lambda m: m[1] + paragraph + m[2],
                             parts['word/document.xml'].decode(), flags=re.S)
        assert count == 1
        parts['word/document.xml'] = re.sub(r'<w:docGrid\b[^>]*/>',
            '<w:docGrid w:type="default" w:linePitch="360"/>', text).encode()
        variation = f'stop{stop}' if boundary else f'indent{indent}'
        name = f'word-windows-edge-dot-leader-{slug}-size{size}-{variation}-compat12'
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
    print('Generated', generate(args.output_directory), 'dot leader probes')
