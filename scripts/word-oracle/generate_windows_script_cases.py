"""Generate Word script/control sweeps and Century typography-switch probes."""
import hashlib
import json
from pathlib import Path
import re
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

ROOT = Path(__file__).resolve().parents[2]
SOURCE_SHA256 = 'fc73dc3e8cc5118bbd0468c783ddf277d886d5e6a91e9522e13c7ca391c55f1c'


def write(path, parts):
    with ZipFile(path, 'w', compression=ZIP_DEFLATED, compresslevel=9) as archive:
        for name, data in sorted(parts.items()):
            info = ZipInfo(name, (2000, 1, 1, 0, 0, 0))
            info.compress_type = ZIP_DEFLATED
            archive.writestr(info, data)


def main():
    fixtures = ROOT / 'public/fixtures'
    source = fixtures / 'word-mac-probe-vert-align-sweep.docx'
    if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA256:
        raise ValueError('Original sweep source changed')
    with ZipFile(source) as archive:
        original = {name: archive.read(name) for name in archive.namelist()}
    for style, bold, italic in [('regular', False, False), ('bold', True, False),
                                 ('italic', False, True), ('bold-italic', True, True)]:
        parts = dict(original)
        text = parts['word/document.xml'].decode()
        text = re.sub(r'<w:b(?:\s[^>]*)?/>|<w:i(?:\s[^>]*)?/>', '', text)
        flags = ('<w:b/>' if bold else '') + ('<w:i/>' if italic else '')
        text = text.replace('<w:sz ', flags + '<w:sz ')
        parts['word/document.xml'] = text.encode()
        write(fixtures / f'word-windows-edge-vert-align-sweep-{style}-compat15.docx', parts)
        text, count = re.subn(r'<w:vertAlign\b[^>]*/>', '', text)
        if count != 392:
            raise ValueError('Script row coverage changed')
        parts['word/document.xml'] = text.encode()
        write(fixtures / f'word-windows-edge-vert-align-control-{style}-compat15.docx', parts)

    release = json.loads((ROOT / 'tests/word-oracle/release-corpus.json').read_text())
    hashes = {row['fixture']: row['docx_sha256'] for row in release['fixtures']}
    for source in sorted(fixtures.glob('word-windows-edge-yu-mincho-opentype-*.docx')):
        if hashlib.sha256(source.read_bytes()).hexdigest() != hashes[source.stem]:
            raise ValueError('Original typography source changed')
        with ZipFile(source) as archive:
            parts = {name: archive.read(name) for name in archive.namelist()}
        parts = {name: data.replace(b'Yu Mincho', b'Century') if name.endswith('.xml') else data
                 for name, data in parts.items()}
        write(source.with_name(source.name.replace('-yu-mincho-', '-century-')), parts)
    print('Generated 8 script/control sweeps and 9 Century typography probes')


if __name__ == '__main__':
    main()
