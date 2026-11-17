"""Reproduce the captured Yu Mincho Latin advance and kerning controls."""
from pathlib import Path
import hashlib
import re
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

ROOT = Path(__file__).resolve().parents[2]
PARENT = ('tests/word-oracle/windows/observations/sources/'
          'word-windows-wide-a3-yu-ascii-size16-compat15.docx')
PARENT_SHA256 = '232f29133863d55e12f8d2cbf7793ad7f4d2231478b25b8c23002c03a2e2a75c'
PATTERNS = {'b': 'B' * 30, 'bc': 'BC' * 12, 'z': 'z' * 36,
            'z-space': 'z ' * 12, 'space-a': ' A' * 12,
            'mixed': 'AgABCxyz ' * 3}


def generate(output):
    parent = ROOT / PARENT
    if hashlib.sha256(parent.read_bytes()).hexdigest() != PARENT_SHA256:
        raise ValueError('Captured Yu Mincho parent changed')
    output.mkdir(parents=True, exist_ok=False)
    with ZipFile(parent) as archive:
        for size in (9.5, 16):
            for pattern, text in PATTERNS.items():
                for kerning in ('inherit', 'zero'):
                    xml = archive.read('word/document.xml').decode('utf-8')
                    if xml.count('Yu Mincho') != 12 or '<w:kern ' in xml:
                        raise ValueError('Captured font/kerning properties changed')
                    xml, runs = re.subn(r'(<w:t xml:space="preserve">)[^<]*(</w:t>)',
                                        lambda m: m[1] + text + m[2], xml)
                    xml, sizes = re.subn(r'(<w:sz(?:Cs)? w:val=")[0-9]+',
                                         lambda m: m[1] + str(round(size * 2)), xml)
                    if runs != 3 or sizes != 6:
                        raise ValueError('Captured text/size structure changed')
                    if kerning == 'zero':
                        xml = xml.replace('<w:sz ', '<w:kern w:val="0"/><w:sz ')
                    token = ('%g' % size).replace('.', 'p')
                    name = f'word-windows-yu-advance-{pattern}-{kerning}-size{token}-compat15.docx'
                    with ZipFile(output / name, 'x', compression=ZIP_DEFLATED, compresslevel=6) as target:
                        for old in archive.infolist():
                            info = ZipInfo(old.filename, (2026, 11, 17, 0, 0, 0))
                            info.create_system = 0
                            info.compress_type = ZIP_DEFLATED
                            info.external_attr = old.external_attr
                            target.writestr(info, xml.encode('utf-8') if old.filename == 'word/document.xml'
                                            else archive.read(old.filename))
    return 24


def generate_fe_layout(output):
    """Change only useFELayout in five of the Latin controls, in both directions."""
    import tempfile
    output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory() as directory:
        parents = Path(directory) / 'parents'
        generate(parents)
        for token in ('b-inherit-size16', 'z-inherit-size16', 'mixed-inherit-size16',
                      'mixed-inherit-size9p5', 'z-inherit-size9p5'):
            with ZipFile(parents / f'word-windows-yu-advance-{token}-compat15.docx') as parent:
                settings = parent.read('word/settings.xml').decode('utf-8')
                if settings.count('<w:useFELayout/>') != 1:
                    raise ValueError('Captured useFELayout property changed')
                for label, value in (('off', '0'), ('on', '1')):
                    xml = settings.replace('<w:useFELayout/>', f'<w:useFELayout w:val="{value}"/>')
                    name = f'word-windows-yu-fe-layout-{token}-{label}-compat15.docx'
                    with ZipFile(output / name, 'x', compression=ZIP_DEFLATED, compresslevel=6) as target:
                        for old in parent.infolist():
                            info = ZipInfo(old.filename, (2026, 11, 17, 0, 0, 0))
                            info.create_system = 0
                            info.compress_type = ZIP_DEFLATED
                            info.external_attr = old.external_attr
                            target.writestr(info, xml.encode('utf-8') if old.filename == 'word/settings.xml'
                                            else parent.read(old.filename))
    return 10


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--fe-layout', action='store_true')
    args = parser.parse_args()
    print(f'Generated {(generate_fe_layout if args.fe_layout else generate)(args.output)} controls')
