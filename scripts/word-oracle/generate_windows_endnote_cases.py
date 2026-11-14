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
    # Keep the earlier malformed `none` probes as unpromoted observations.
    # The schema's inactive grid type is `default`, asserted separately.
    for grid in (0, 'default', 360, 480):
        for font in (0, 21, 40):
            parts = dict(original)
            grid_type = 'default' if grid == 'default' else ('lines' if grid else 'none')
            pitch = grid if grid in (360, 480) else 360
            text, count = re.subn(
                r'<w:docGrid\b[^>]*/>',
                f'<w:docGrid w:type="{grid_type}" w:linePitch="{pitch}"/>',
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
    for slug, face in [('theme', None), ('century', 'Century'),
                       ('arial', 'Arial'), ('yu-mincho', 'Yu Mincho')]:
        for half_points in (21, 24, 40):
            parts = dict(original)
            paragraphs = []
            for style in ('plain', 'script', 'normal'):
                fonts = (f'<w:rFonts w:ascii="{face}" w:hAnsi="{face}" '
                         f'w:eastAsia="{face}"/>' if face else '')
                props = (fonts + f'<w:sz w:val="{half_points}"/>'
                         f'<w:szCs w:val="{half_points}"/>')
                extra = ''
                if style != 'plain':
                    script = '<w:vertAlign w:val="superscript"/>' if style == 'script' else ''
                    extra = f'<w:r><w:rPr>{props}{script}</w:rPr><w:t>i</w:t></w:r>'
                paragraphs.append(
                    '<w:p><w:pPr><w:spacing w:before="0" w:after="0" '
                    'w:line="240" w:lineRule="auto"/>'
                    f'<w:rPr>{props}</w:rPr></w:pPr><w:r><w:rPr>{props}</w:rPr>'
                    f'<w:t>Hello</w:t></w:r>{extra}</w:p>')
            text = re.sub(r'(<w:body>).*?(<w:sectPr)',
                          lambda match: match[1] + ''.join(paragraphs) + match[2],
                          parts['word/document.xml'].decode(), flags=re.S)
            parts['word/document.xml'] = re.sub(
                r'<w:docGrid\b[^>]*/>',
                '<w:docGrid w:type="default" w:linePitch="360"/>', text).encode()
            path = fixtures / f'word-windows-edge-default-grid-{slug}-size{half_points}-compat15.docx'
            with ZipFile(path, 'w', compression=ZIP_DEFLATED, compresslevel=9) as archive:
                for name, data in sorted(parts.items()):
                    info = ZipInfo(name, (2000, 1, 1, 0, 0, 0))
                    info.compress_type = ZIP_DEFLATED
                    archive.writestr(info, data)
    print('Generated 12 separator/grid probes and 12 inactive-grid controls')


if __name__ == '__main__':
    main()
