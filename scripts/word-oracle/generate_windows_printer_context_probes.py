"""Reproduce original Word printer-context and exact-width wrap controls."""
import argparse
import hashlib
from pathlib import Path
import re
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED
from generate_windows_wide_cjk_size_probes import ROOT

BASE = 'tests/word-oracle/windows/observations/sources/word-windows-wide-a3-mincho-no-grid-size10p5-compat15.docx'
BASE_SHA256 = '7587d91b4a6cf99c1884a34e9365fcc020b71cc0f1320090aec6b6cf993a6878'


def generate(output, group='context'):
    parent = ROOT / BASE
    if hashlib.sha256(parent.read_bytes()).hexdigest() != BASE_SHA256:
        raise ValueError('Captured no-grid A3 parent changed')
    if group == 'context':
        variants = [(face, family, fe, grid) for face, family in
                    [('en', 'MS Mincho'), ('ja', '\uff2d\uff33 \u660e\u671d')]
                    for fe in (0, 1) for grid in (0, 1)]
    elif group == 'wrap':
        variants = (419.9, 420, 420.1, 420.3, 420.5)
    else:
        raise ValueError('Unknown printer-context group')
    output.mkdir(parents=True, exist_ok=False)
    with ZipFile(parent) as archive:
        for variant in variants:
            doc = archive.read('word/document.xml').decode('utf-8')
            settings = archive.read('word/settings.xml').decode('utf-8')
            if group == 'context':
                face, family, fe, grid = variant
                if doc.count('MS Mincho') != 12:
                    raise ValueError('Captured font declarations changed')
                doc = doc.replace('MS Mincho', family)
                if not grid:
                    doc, count = re.subn(r'<w:docGrid[^>]*/>', '', doc)
                    if count != 1:
                        raise ValueError('Captured grid changed')
                if not fe:
                    settings, count = re.subn(r'<w:useFELayout[^>]*/>', '', settings)
                    if count != 1:
                        raise ValueError('Captured compatibility settings changed')
                name = f'word-windows-printer-context-{face}-fe{fe}-grid{grid}-size10p5-compat15'
            else:
                page = int(re.search(r'<w:pgSz w:w="([0-9]+)"', doc)[1])
                left = int(re.search(r'<w:pgMar[^>]* w:left="([0-9]+)"', doc)[1])
                right = page - left - round(variant * 20)
                doc, count = re.subn(r'(<w:pgMar[^>]* w:right=")[0-9]+',
                                     lambda match: match[1] + str(right), doc)
                if count != 1:
                    raise ValueError('Captured margins changed')
                token = str(variant).replace('.', 'p')
                name = f'word-windows-printer-wrap-width{token}-size10p5-compat15'
            with ZipFile(output / (name + '.docx'), 'x', compression=ZIP_DEFLATED) as target:
                for original in archive.infolist():
                    info = ZipInfo(original.filename, (2026, 11, 16, 0, 0, 0))
                    info.create_system = 0
                    info.compress_type = ZIP_DEFLATED
                    info.external_attr = original.external_attr
                    data = (doc.encode('utf-8') if original.filename == 'word/document.xml'
                            else settings.encode('utf-8') if original.filename == 'word/settings.xml'
                            else archive.read(original.filename))
                    target.writestr(info, data)
    return len(variants)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--group', choices=('context', 'wrap'), default='context')
    args = parser.parse_args()
    print(f'Generated {generate(args.output, args.group)} printer controls')
