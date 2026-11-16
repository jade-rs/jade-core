"""Reproduce the original Windows Word Japanese font-alias controls."""
import argparse
import hashlib
from pathlib import Path
import re
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED
from generate_windows_wide_cjk_size_probes import ROOT, BASE, BASE_SHA256


def generate(output):
    parent = ROOT / BASE
    if hashlib.sha256(parent.read_bytes()).hexdigest() != BASE_SHA256:
        raise ValueError('Captured A3 parent changed')
    output.mkdir(parents=True, exist_ok=False)
    count = 0
    with ZipFile(parent) as archive:
        for face, family in [('mincho', 'Mincho'), ('ms-mincho', 'MS-Mincho'),
                             ('ms-gothic', 'MS-Gothic')]:
            for bold in (False, True):
                xml = archive.read('word/document.xml').decode('utf-8')
                xml, changed = re.subn(r'(<w:sz(?:Cs)? w:val=")[0-9]+',
                                      lambda match: match[1] + '21', xml)
                if changed != 6 or xml.count('MS Mincho') != 12:
                    raise ValueError('Captured three-run parent changed')
                xml = xml.replace('MS Mincho', family)
                xml, changed = re.subn(r'(<w:t xml:space="preserve">).*?(</w:t>)',
                                      lambda match: match[1] + '\u65e5\u672c\u8a9e' * 10 + match[2], xml)
                if changed != 3:
                    raise ValueError('Captured text changed')
                if bold:
                    xml, changed = re.subn(r'(<w:rFonts[^>]*/>)', r'\1<w:b/>', xml)
                    if changed != 3:
                        raise ValueError('Captured font declarations changed')
                style = 'bold' if bold else 'regular'
                name = f'word-windows-ms-font-alias-{face}-size10p5-{style}-compat15.docx'
                with ZipFile(output / name, 'x', compression=ZIP_DEFLATED) as target:
                    for original in archive.infolist():
                        info = ZipInfo(original.filename, (2026, 11, 16, 0, 0, 0))
                        info.create_system = 0
                        info.compress_type = ZIP_DEFLATED
                        info.external_attr = original.external_attr
                        data = xml.encode('utf-8') if original.filename == 'word/document.xml' else archive.read(original.filename)
                        target.writestr(info, data)
                count += 1
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} font-alias controls')
