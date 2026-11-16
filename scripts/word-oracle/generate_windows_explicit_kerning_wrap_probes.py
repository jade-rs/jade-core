"""Reproduce six captured nominal-cell controls with explicit w:kern values."""
from pathlib import Path
import hashlib
import re
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

ROOT = Path(__file__).resolve().parents[2]
PARENT = 'public/fixtures/word-windows-yu-nominal-wrap-size20-width319p95-compat15.docx'
PARENT_SHA256 = 'ec613f40a6760acabd690d6c030aa7d2b8091d26571d6a13ce4a88721d7fcc7b'


def generate(output):
    parent = ROOT / PARENT
    if hashlib.sha256(parent.read_bytes()).hexdigest() != PARENT_SHA256:
        raise ValueError('Captured nominal-wrap parent changed')
    output.mkdir(parents=True, exist_ok=False)
    with ZipFile(parent) as archive:
        for face, family, widths in [('yu', 'Yu Mincho', (319.95, 320.0)),
                                     ('mincho', 'MS Mincho', (319.95,))]:
            for width in widths:
                # Preserve the original filenames: their labels record the
                # authored thresholds, not an assertion about engine flags.
                for label, threshold in [('disabled', 32767), ('enabled', 0)]:
                    xml = archive.read('word/document.xml').decode('utf-8')
                    if xml.count('Yu Mincho') != 12 or '<w:kern ' in xml:
                        raise ValueError('Captured font/kerning properties changed')
                    xml = xml.replace('Yu Mincho', family)
                    if xml.count('<w:sz ') != 3:
                        raise ValueError('Captured run sizes changed')
                    xml = xml.replace('<w:sz ', f'<w:kern w:val="{threshold}"/><w:sz ')
                    page = int(re.search(r'<w:pgSz w:w="([0-9]+)"', xml)[1])
                    left = int(re.search(r'<w:pgMar[^>]* w:left="([0-9]+)"', xml)[1])
                    xml, count = re.subn(r'(<w:pgMar[^>]* w:right=")[0-9]+',
                                         lambda m: m[1] + str(page - left - round(width * 20)), xml)
                    if count != 1:
                        raise ValueError('Captured margins changed')
                    token = ('%g' % width).replace('.', 'p')
                    name = f'word-windows-nominal-wrap-kerning-{face}-{label}-width{token}-compat15.docx'
                    with ZipFile(output / name, 'x', compression=ZIP_DEFLATED, compresslevel=6) as target:
                        for old in archive.infolist():
                            info = ZipInfo(old.filename, (2026, 11, 17, 0, 0, 0))
                            info.create_system = 0
                            info.compress_type = ZIP_DEFLATED
                            info.external_attr = old.external_attr
                            target.writestr(info, xml.encode('utf-8') if old.filename == 'word/document.xml'
                                            else archive.read(old.filename))
    return 6


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} explicit-kerning wrap controls')
