"""Generate outline, inset, font, preset and exact-line textbox probes."""
import hashlib
from pathlib import Path
import re
from zipfile import ZipFile

from generate_windows_script_cases import write

ROOT = Path(__file__).resolve().parents[2]
SOURCE_SHA256 = '5c792b1eb9b78b40914c6f35a4b9e5bd0696d66f301d333021cc6343e490ffc7'


def main():
    fixtures = ROOT / 'public/fixtures'
    source = fixtures / 'footnote-table-textbox-order.docx'
    if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA256:
        raise ValueError('Original Mac textbox source changed')
    with ZipFile(source) as archive:
        original = {name: archive.read(name) for name in archive.namelist()}
    # slug, outline EMUs, top inset EMUs, face, half-points, preset, height, exact pitch
    cases = [(f'border{border}-top{top}', border, top, None, 21, 'rect', 457200, None)
             for border in (6350, 12700, 25400) for top in (0, 45720)]
    cases += [(f'{slug}-size{size}', 6350, 45720, face, size, 'rect', 457200, None)
              for slug, face in [('arial', 'Arial'), ('century', 'Century'),
                                 ('yu-mincho', 'Yu Mincho')] for size in (21, 40)]
    cases += [(f'shape-{shape.lower()}-border{border}', border, 45720,
               None, 21, shape, 914400, None)
              for shape in ('roundRect', 'triangle') for border in (6350, 12700, 25400)]
    cases += [(f'exact{line}-size{size}', 6350, 45720, 'Yu Mincho', size,
               'rect', 914400, line) for size in (21, 40) for line in (14, 28)]
    for slug, border, top, face, size, shape, height, exact in cases:
        parts = dict(original)
        text = parts['word/document.xml'].decode()
        for old, new in [('<a:ln w="6350">', f'<a:ln w="{border}">'),
                         ('tIns="45720"', f'tIns="{top}"'),
                         ('prst="rect"', f'prst="{shape}"')]:
            if text.count(old) != 1:
                raise ValueError('Original textbox structure changed')
            text = text.replace(old, new)
        text = text.replace('cy="457200"', f'cy="{height}"')
        if face:
            match = re.search(r'<wps:txbx>.*?</wps:txbx>', text)
            if match is None:
                raise ValueError('Original textbox story missing')
            props = (f'<w:rFonts w:ascii="{face}" w:hAnsi="{face}" w:eastAsia="{face}"/>'
                     f'<w:sz w:val="{size}"/><w:szCs w:val="{size}"/>')
            box = match[0].replace('<w:rPr>', '<w:rPr>' + props)
            box = re.sub(r'<w:r>(?!<w:rPr>)', '<w:r><w:rPr>' + props + '</w:rPr>', box)
            if exact is not None:
                box = box.replace('<w:p>', '<w:p><w:pPr>'
                                  f'<w:spacing w:line="{exact * 20}" w:lineRule="exact"/>'
                                  '</w:pPr>')
            text = text[:match.start()] + box + text[match.end():]
        parts['word/document.xml'] = text.encode()
        write(fixtures / f'word-windows-edge-textbox-{slug}-compat15.docx', parts)
    multiline = [(slug, face, size, None)
                 for slug, face in [('arial', 'Arial'), ('century', 'Century'),
                                    ('yu-mincho', 'Yu Mincho')] for size in (21, 40)]
    multiline += [('yu-mincho', 'Yu Mincho', size, line)
                  for size in (21, 40) for line in (14, 28)]
    for slug, face, size, line in multiline:
        parts = dict(original)
        text = parts['word/document.xml'].decode().replace('cy="457200"', 'cy="1828800"')
        props = (f'<w:rFonts w:ascii="{face}" w:hAnsi="{face}" w:eastAsia="{face}"/>'
                 f'<w:sz w:val="{size}"/><w:szCs w:val="{size}"/>')
        spacing = (f'<w:pPr><w:spacing w:line="{line * 20}" w:lineRule="exact"/></w:pPr>'
                   if line else '')
        story = ''.join(f'<w:p>{spacing}<w:r><w:rPr>{props}</w:rPr><w:t>{value}</w:t></w:r></w:p>'
                        for value in ('Box text', 'Second line', 'Third line'))
        text, count = re.subn(r'<w:txbxContent>.*?</w:txbxContent>',
                             '<w:txbxContent>' + story + '</w:txbxContent>', text, flags=re.S)
        if count != 1:
            raise ValueError('Original textbox content changed')
        parts['word/document.xml'] = text.encode()
        label = f'{slug}-size{size}' + (f'-exact{line}' if line else '-auto')
        write(fixtures / f'word-windows-edge-textbox-multiline-{label}-compat15.docx', parts)
    print('Generated 32 textbox probes')


if __name__ == '__main__':
    main()
