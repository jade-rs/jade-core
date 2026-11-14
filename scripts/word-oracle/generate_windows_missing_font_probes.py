"""Reproduce the missing-family controls captured in separate Word processes."""
import argparse
import hashlib
from pathlib import Path
import re

from generate_windows_table_style_probes import read_parts, write_parts

ROOT = Path(__file__).resolve().parents[2]
SOURCE_SHA256 = 'a61b160c57a76b7d22cdba0aed72dadaab198655fae7eb24de788c8a04227f3b'
CASES = [('roman', 'JadeUnavailableFixtureA', 'roman'),
         ('swiss', 'JadeUnavailableFixtureB', 'swiss'),
         ('modern', 'JadeUnavailableFixtureC', 'modern'),
         ('auto', 'JadeUnavailableFixtureD', 'auto'),
         ('hiragino-swiss', 'ヒラギノ角ゴ ProN W3', 'swiss'),
         ('hiragino-latin', 'Hiragino Kaku Gothic ProN W3', 'roman'),
         ('hiragino-w6', 'ヒラギノ角ゴ ProN W6', 'roman'),
         ('unknown-ja-roman', '未導入検証字体甲', 'roman'),
         ('unknown-ja-swiss', '未導入検証字体乙', 'swiss')]


def generate(output):
    source = ROOT / 'public/fixtures/hello.docx'
    assert hashlib.sha256(source.read_bytes()).hexdigest() == SOURCE_SHA256
    original = read_parts(source)
    output.mkdir(parents=True, exist_ok=False)
    for label, face, family in CASES:
        parts = dict(original)

        def edit(match):
            replacement = (f'<w:rFonts w:ascii="{face}" w:hAnsi="{face}" '
                           f'w:eastAsia="{face}" w:cs="{face}"/>')
            value, count = re.subn(r'<w:rFonts\b[^>]*/>', replacement, match[0], count=1)
            assert count == 1
            return value

        styles, count = re.subn(r'<w:style\b[^>]*w:styleId="本文".*?</w:style>',
                                edit, parts['word/styles.xml'].decode(), flags=re.S)
        assert count == 1
        parts['word/styles.xml'] = styles.encode()
        replacement = (f'<w:font w:name="{face}"><w:charset w:val="00"/>'
                       f'<w:family w:val="{family}"/><w:pitch w:val="'
                       + ('fixed' if family == 'modern' else 'default') + '"/></w:font>')
        fonts, count = re.subn(r'<w:font w:name="ヒラギノ角ゴ ProN W3">.*?</w:font>',
                               replacement, parts['word/fontTable.xml'].decode(), flags=re.S)
        assert count == 1
        parts['word/fontTable.xml'] = fonts.encode()
        write_parts(output / f'word-windows-edge-font-independent-{label}-hello-compat15.docx', parts)
    return len(CASES)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-directory', type=Path, required=True)
    args = parser.parse_args()
    print(f'Generated {generate(args.output_directory)} missing-font controls')
