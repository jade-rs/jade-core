"""Reproduce eight captured wide Yu Mincho full-cell controls."""
from pathlib import Path
import hashlib
import math
import re
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

ROOT = Path(__file__).resolve().parents[2]
PARENT = 'tests/word-oracle/windows/observations/sources/word-windows-mincho-wide-a3-size12-regular-compat15.docx'
PARENT_SHA256 = '985c94b8bd994ba5e6f8a54792a83fa640b95f107eabba1bd10a2579912e9e18'


def generate(output):
    parent = ROOT / PARENT
    if hashlib.sha256(parent.read_bytes()).hexdigest() != PARENT_SHA256:
        raise ValueError('Captured A3 parent changed')
    output.mkdir(parents=True, exist_ok=False)
    with ZipFile(parent) as archive:
        for size in (8.5, 11.5, 12.5, 14.5, 15.5, 17.5, 20, 22):
            xml = archive.read('word/document.xml').decode('utf-8')
            if xml.count('MS Mincho') != 12:
                raise ValueError('Captured font slots changed')
            xml = xml.replace('MS Mincho', 'Yu Mincho')
            xml, count = re.subn(r'(<w:sz(?:Cs)? w:val=")[0-9]+',
                                 lambda m: m[1] + str(int(size * 2)), xml)
            if count != 6:
                raise ValueError('Captured sizes changed')
            text = '日本語' * max(1, math.floor(950 / size / 9))
            xml, count = re.subn(r'(<w:t xml:space="preserve">).*?(</w:t>)',
                                 lambda m: m[1] + text + m[2], xml)
            if count != 3:
                raise ValueError('Captured text runs changed')
            token = ('%g' % size).replace('.', 'p')
            name = f'word-windows-yu-wide-halfpoint-size{token}-regular-compat15.docx'
            with ZipFile(output / name, 'x', compression=ZIP_DEFLATED, compresslevel=6) as target:
                for old in archive.infolist():
                    info = ZipInfo(old.filename, (2026, 11, 17, 0, 0, 0))
                    info.create_system = 0
                    info.compress_type = ZIP_DEFLATED
                    info.external_attr = old.external_attr
                    target.writestr(info, xml.encode('utf-8') if old.filename == 'word/document.xml'
                                    else archive.read(old.filename))
    return 8


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} wide Yu Mincho controls')
