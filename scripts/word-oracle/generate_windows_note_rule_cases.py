"""Reproduce the twelve versioned Windows Word endnote-rule font probes."""
import hashlib
from pathlib import Path
import re
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

ROOT = Path(__file__).resolve().parents[2]
SOURCE_SHA256 = '06477c7f53f9b7c80147bd15f7db44254b6e72ee5cd7e39af422a7b81b17d19e'


def main():
    source = ROOT / 'public/fixtures/endnote.docx'
    if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA256:
        raise ValueError('Original Mac endnote source changed')
    with ZipFile(source) as archive:
        original = {name: archive.read(name) for name in archive.namelist()}
    for slug, face in [('arial', 'Arial'), ('century', 'Century'),
                       ('yu-mincho', 'Yu Mincho'), ('ms-mincho', 'MS Mincho')]:
        for size in (21, 24, 40):
            parts = dict(original)
            props = (f'<w:rPr><w:rFonts w:ascii="{face}" w:hAnsi="{face}" '
                     f'w:eastAsia="{face}" w:cs="{face}"/>'
                     f'<w:sz w:val="{size}"/><w:szCs w:val="{size}"/></w:rPr>')
            text, count = re.subn(
                r'<w:endnote\b[^>]*w:type="separator"[^>]*>.*?</w:endnote>',
                lambda match: match[0].replace('<w:r>', '<w:r>' + props),
                parts['word/endnotes.xml'].decode(), flags=re.S)
            if count != 1:
                raise ValueError('Original separator changed')
            parts['word/endnotes.xml'] = text.encode()
            target = ROOT / 'public/fixtures' / (
                f'word-windows-edge-endnote-rule-{slug}-size{size}-compat15.docx')
            with ZipFile(target, 'w', compression=ZIP_DEFLATED, compresslevel=9) as archive:
                for name, data in sorted(parts.items()):
                    info = ZipInfo(name, (2000, 1, 1, 0, 0, 0))
                    info.compress_type = ZIP_DEFLATED
                    archive.writestr(info, data)
    print('Generated 12 endnote separator font/size probes')


if __name__ == '__main__':
    main()
