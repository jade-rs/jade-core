"""Isolate compatibility modes in unchanged Mac floating-wrap source documents."""
from pathlib import Path
import hashlib
import re
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

ROOT = Path(__file__).resolve().parents[2]
PARENTS = {
    'square': ('floating-square-wrap-mac',
               '66004027672929d429fba9659e2cec4f3e648c0311b0e5aba339f1ac4657dff6'),
    'sides': ('floating-wrap-sides-mac',
              '33a9df9946ed6df47bca02a13ccc41c829db7332aeb6ec38e5c183dac3c32f46'),
    'horizontal': ('word-mac-probe-floating-horizontal-14',
                   'a325e0ac1ee3c724a242367e6d067d73641b20ee0c821788f0ca6fb42d2d7f96'),
}


def generate(output, root=ROOT):
    output.mkdir(parents=True, exist_ok=False)
    for label, (name, digest) in PARENTS.items():
        source = root / 'tests/word-oracle/mac-elsa-synthetic' / name / 'source.docx'
        if hashlib.sha256(source.read_bytes()).hexdigest() != digest:
            raise ValueError('Original floating-wrap source changed: ' + name)
        with ZipFile(source) as archive:
            settings = archive.read('word/settings.xml').decode('utf-8')
            tags = [m[0] for m in re.finditer(r'<w:compatSetting\b[^>]*/>', settings)
                    if 'w:name="compatibilityMode"' in m[0]]
            if len(tags) != 1 or 'w:val="14"' not in tags[0]:
                raise ValueError('Original compatibility mode changed')
            for mode in (12, 14, 15):
                xml = settings.replace(tags[0], tags[0].replace('w:val="14"', f'w:val="{mode}"'))
                filename = f'word-windows-float-lookahead-{label}-compat{mode}.docx'
                with ZipFile(output / filename, 'x', compression=ZIP_DEFLATED, compresslevel=6) as target:
                    for old in archive.infolist():
                        info = ZipInfo(old.filename, (2026, 11, 17, 0, 0, 0))
                        info.create_system = 0
                        info.compress_type = ZIP_DEFLATED
                        info.external_attr = old.external_attr
                        target.writestr(info, xml.encode('utf-8') if old.filename == 'word/settings.xml'
                                        else archive.read(old.filename))
    return 9


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} floating-wrap controls')
