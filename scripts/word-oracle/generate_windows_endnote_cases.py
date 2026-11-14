"""Generate endnote separator probes from the unchanged Mac endnote source.

Grid pitch: disabled, 18pt, 24pt; separator size: inherited, 10.5pt, 20pt.
Original Windows Word 16.0.19127.20936 captures are stored independently.
"""
import hashlib
from pathlib import Path
import re
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

ROOT = Path(__file__).resolve().parents[2]
SOURCE_SHA256 = '06477c7f53f9b7c80147bd15f7db44254b6e72ee5cd7e39af422a7b81b17d19e'


def main():
    fixtures = ROOT / 'public/fixtures'
    source = fixtures / 'endnote.docx'
    if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA256:
        raise ValueError('Original Mac endnote source changed')
    with ZipFile(source) as archive:
        original = {name: archive.read(name) for name in archive.namelist()}
    for grid in (0, 360, 480):
        for font in (0, 21, 40):
            parts = dict(original)
            grid_type = 'lines' if grid else 'none'
            text, count = re.subn(
                r'<w:docGrid\b[^>]*/>',
                f'<w:docGrid w:type="{grid_type}" w:linePitch="{grid or 360}"/>',
                parts['word/document.xml'].decode())
            if count != 1:
                raise ValueError('Original document grid changed')
            parts['word/document.xml'] = text.encode()
            if font:
                def change(match):
                    return match[0].replace(
                        '<w:r>', f'<w:r><w:rPr><w:sz w:val="{font}"/>'
                        f'<w:szCs w:val="{font}"/></w:rPr>')
                text, count = re.subn(
                    r'<w:endnote\b[^>]*w:type="separator"[^>]*>.*?</w:endnote>',
                    change, parts['word/endnotes.xml'].decode(), flags=re.S)
                if count != 1:
                    raise ValueError('Original endnote separator changed')
                parts['word/endnotes.xml'] = text.encode()
            path = fixtures / (
                f'word-windows-edge-endnote-separator-grid{grid}-font{font}-compat15.docx')
            with ZipFile(path, 'w', compression=ZIP_DEFLATED, compresslevel=9) as archive:
                for name, data in sorted(parts.items()):
                    info = ZipInfo(name, (2000, 1, 1, 0, 0, 0))
                    info.compress_type = ZIP_DEFLATED
                    archive.writestr(info, data)
    print('Generated 9 endnote separator/grid probes')


if __name__ == '__main__':
    main()
