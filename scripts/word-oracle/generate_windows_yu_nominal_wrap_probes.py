"""Reproduce five captured Yu Mincho nominal-cell wrap boundary controls."""
from pathlib import Path
import hashlib
import re
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

ROOT = Path(__file__).resolve().parents[2]
PARENT_SHA256 = 'a03b09579ab8410e8958e683f78b89e9a9d48b39dacaaf7842d04ab2af0757d7'


def generate(output):
    parent = ROOT / 'public/fixtures/word-windows-wide-a3-ms-mincho-size22-compat15.docx'
    if hashlib.sha256(parent.read_bytes()).hexdigest() != PARENT_SHA256:
        raise ValueError('Captured full-cell parent changed')
    output.mkdir(parents=True, exist_ok=False)
    with ZipFile(parent) as archive:
        original = archive.read('word/document.xml').decode('utf-8')
        for width in (319.9, 319.95, 320.0, 320.05, 320.1):
            doc = original.replace('MS Mincho', 'Yu Mincho')
            if original.count('MS Mincho') != 12:
                raise ValueError('Captured font slots changed')
            doc, count = re.subn(r'(<w:sz(?:Cs)? w:val=")[0-9]+', r'\g<1>40', doc)
            if count != 6:
                raise ValueError('Captured run sizes changed')
            texts = iter(('日' * 43, '日' * 43, '日' * 42))
            doc, count = re.subn(r'(<w:t xml:space="preserve">).*?(</w:t>)',
                                 lambda m: m[1] + next(texts) + m[2], doc)
            if count != 3:
                raise ValueError('Captured text runs changed')
            page = int(re.search(r'<w:pgSz w:w="([0-9]+)"', doc)[1])
            left = int(re.search(r'<w:pgMar[^>]* w:left="([0-9]+)"', doc)[1])
            doc, count = re.subn(r'(<w:pgMar[^>]* w:right=")[0-9]+',
                                 lambda m: m[1] + str(page - left - round(width * 20)), doc)
            if count != 1:
                raise ValueError('Captured margins changed')
            token = ('%g' % width).replace('.', 'p')
            name = f'word-windows-yu-nominal-wrap-size20-width{token}-compat15.docx'
            with ZipFile(output / name, 'x', compression=ZIP_DEFLATED, compresslevel=6) as target:
                for old in archive.infolist():
                    info = ZipInfo(old.filename, (2026, 11, 17, 0, 0, 0))
                    info.create_system = 0
                    info.compress_type = ZIP_DEFLATED
                    info.external_attr = old.external_attr
                    target.writestr(info, doc.encode('utf-8') if old.filename == 'word/document.xml'
                                    else archive.read(old.filename))
    return 5


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} nominal-cell wrap controls')
