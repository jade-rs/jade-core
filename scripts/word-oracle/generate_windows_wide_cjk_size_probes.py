"""Reproduce original Windows Word wide Japanese printer-metric controls."""
import argparse
import hashlib
import math
from pathlib import Path
import re
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

ROOT = Path(__file__).resolve().parents[2]
BASE = 'tests/word-oracle/windows/observations/sources/word-windows-mincho-wide-a3-size12-regular-compat15.docx'
BASE_SHA256 = '985c94b8bd994ba5e6f8a54792a83fa640b95f107eabba1bd10a2579912e9e18'


def generate(output, group='sizes'):
    source = ROOT / BASE
    if hashlib.sha256(source.read_bytes()).hexdigest() != BASE_SHA256:
        raise ValueError('Captured A3 parent changed')
    if group == 'sizes':
        variants = [('mincho', 'MS Mincho', size) for size in
                    (8.5, 9, 9.5, 11.5, 12.5, 14.5, 15, 15.5, 17.5, 18.5,
                     20.5, 21, 21.5, 23.5, 24, 24.5)]
    elif group in ('half-device', 'large-half-device'):
        variants = [(face, family, size) for face, family in
                    [('mincho', 'MS Mincho'), ('gothic', 'MS Gothic'), ('yu', 'Yu Mincho')]
                    for size in ((7.5, 19.5, 25.5, 28.5) if group == 'half-device' else (31.5, 34.5, 37.5, 40.5, 43.5))]
    else:
        raise ValueError('Unknown printer-metric probe group')
    output.mkdir(parents=True, exist_ok=False)
    with ZipFile(source) as archive:
        for face, family, size in variants:
            xml = archive.read('word/document.xml').decode('utf-8')
            xml, changed = re.subn(r'(<w:sz(?:Cs)? w:val=")[0-9]+',
                                  lambda match: match[1] + str(int(size * 2)), xml)
            if changed != 6 or xml.count('MS Mincho') != 12:
                raise ValueError('Captured three-run parent structure changed')
            xml = xml.replace('MS Mincho', family)
            repetitions = max(1, math.floor(950 / size / 9))
            xml, changed = re.subn(r'(<w:t xml:space="preserve">).*?(</w:t>)',
                                  lambda match: match[1] + '日本語' * repetitions + match[2], xml)
            if changed != 3:
                raise ValueError('Captured text changed')
            category = 'half-device-independent' if group == 'half-device' else 'large-half-device-independent'
            token = str(size).replace('.', 'p')
            name = (f'word-windows-wide-cjk-a3-size{token}-regular-compat15' if group == 'sizes'
                    else f'word-windows-{category}-{face}-size{token}-regular-compat15')
            with ZipFile(output / (name + '.docx'), 'x', compression=ZIP_DEFLATED) as target:
                for original in archive.infolist():
                    info = ZipInfo(original.filename, (2026, 11, 16, 0, 0, 0))
                    info.create_system = 0
                    info.compress_type = ZIP_DEFLATED
                    info.external_attr = original.external_attr
                    data = xml.encode('utf-8') if original.filename == 'word/document.xml' else archive.read(original.filename)
                    target.writestr(info, data)
    return len(variants)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--group', choices=('sizes', 'half-device', 'large-half-device'), default='sizes')
    args = parser.parse_args()
    print(f'Generated {generate(args.output, args.group)} printer-metric controls')
