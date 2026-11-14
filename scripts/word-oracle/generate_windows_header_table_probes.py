"""Probe Windows table text-edge alignment in body, header and footer stories."""
import argparse
import hashlib
from pathlib import Path
import re
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

ROOT = Path(__file__).resolve().parents[2]
SOURCE_SHA256 = 'd6c325b84a42e5df290543623f000e00eafdbdbe72322b2f56fd391ba7a37d69'


def generate(output):
    source = ROOT / 'public/fixtures/header-footer-edge-cases.docx'
    if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA256:
        raise ValueError('Original header/footer input changed')
    with ZipFile(source) as archive:
        original = {n: archive.read(n) for n in archive.namelist()}
    table, = re.findall(r'<w:tbl>.*?</w:tbl>', original['word/header1.xml'].decode(), re.S)
    output.mkdir(parents=True, exist_ok=False)
    cases = [(mode, 'left', indent) for mode in (12, 14, 15) for indent in (False, True)]
    cases += [(14, align, indent) for align in ('center', 'right') for indent in (False, True)]
    for mode, align, indent in cases:
        parts = dict(original)
        settings, count = re.subn(
            r'(<w:compatSetting w:name="compatibilityMode"[^>]*w:val=")\d+(")',
            lambda m: m[1] + str(mode) + m[2], parts['word/settings.xml'].decode())
        assert count == 1
        parts['word/settings.xml'] = settings.encode()
        body = parts['word/document.xml'].decode()
        assert body.count('<w:sectPr') == 1
        parts['word/document.xml'] = body.replace('<w:sectPr', table + '<w:sectPr', 1).encode()
        props = '' if align == 'left' else f'<w:jc w:val="{align}"/>'
        if indent:
            props += '<w:tblInd w:w="0" w:type="dxa"/>'
        for member in ('word/document.xml', 'word/header1.xml', 'word/footer1.xml'):
            text, count = re.subn(r'<w:tblW\b[^>]*/>', lambda m: m[0] + props,
                                  parts[member].decode())
            assert count == 1
            parts[member] = text.encode()
        name = f'word-windows-edge-header-table-{align}-indent{"0" if indent else "omitted"}-ordered-compat{mode}'
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
    print('Generated', generate(args.output_directory), 'body/header/footer table probes')
