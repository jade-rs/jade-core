"""Vary only Title character spacing in two original Windows Word documents."""
from pathlib import Path
import hashlib
import re
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

ROOT = Path(__file__).resolve().parents[2]
PARENTS = {
    'square': '78d78380ec53ec1ea5f278a4faf82d5b98aeb022eafebe9039b70cba7a4a0831',
    'sides': 'b23a4ec54305200d46c7642768372e5fbeef305ad7b3598d54c9137ed2e6752a',
}
SPACING_TWIPS = (0, 1, 2, 3, 4, 6, 8, 10)


def generate(output, root=ROOT):
    output.mkdir(parents=True, exist_ok=False)
    for label, digest in PARENTS.items():
        source = root / 'tests/word-oracle/windows/observations/sources' / (
            f'word-windows-float-lookahead-{label}-compat15.docx')
        if hashlib.sha256(source.read_bytes()).hexdigest() != digest:
            raise ValueError('Captured Title parent changed: ' + label)
        with ZipFile(source) as parent:
            styles = parent.read('word/styles.xml')
            match = re.search(rb'<w:style\b[^>]*w:styleId="Title"[^>]*>.*?</w:style>',
                              styles, re.S)
            old = b'<w:spacing w:val="5"/>'
            if match is None or match[0].count(old) != 1:
                raise ValueError('Captured Title spacing changed')
            for value in SPACING_TWIPS:
                title = match[0].replace(old, f'<w:spacing w:val="{value}"/>'.encode())
                changed = styles[:match.start()] + title + styles[match.end():]
                name = f'word-windows-title-calibri22-{label}-spacing{value}-compat15.docx'
                with ZipFile(output / name, 'x', compression=ZIP_DEFLATED,
                             compresslevel=6) as target:
                    for original in parent.infolist():
                        item = ZipInfo(original.filename, (2026, 11, 17, 0, 0, 0))
                        item.create_system = 0
                        item.compress_type = ZIP_DEFLATED
                        item.external_attr = original.external_attr
                        target.writestr(item, changed if original.filename == 'word/styles.xml'
                                        else parent.read(original.filename))
    return len(PARENTS) * len(SPACING_TWIPS)


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} Title tracking controls')
