"""Reproduce long Latin/CJK controls for Windows printer-metric rounding."""
import argparse
from pathlib import Path
import re
from generate_windows_table_style_probes import read_parts, write_parts
ROOT = Path(__file__).resolve().parents[2]

def generate(output):
    parent = ROOT / 'tests/word-oracle/windows/observations/sources/word-windows-edge-language-arial-eastasia-ja-ja-ja-compat15.docx'
    original = read_parts(parent)
    document = original['word/document.xml'].decode()
    if document.count('A税あB ') != 3 or re.search(r'<w:(?:b|i)(?:\s|/|>)', document):
        raise ValueError('Expected regular three-run captured parent')
    output.mkdir(parents=True, exist_ok=False)
    count = 0
    variants = []
    for size in (12, 20, 24):
        for style, tags in [('regular', ''), ('bold', '<w:b/>'), ('italic', '<w:i/>'), ('bolditalic', '<w:b/><w:i/>')]:
            variants.append(('arial', 'Arial', size, style, tags, 'AgABCXYZ '))
    for size in (9, 10.5, 12, 20, 40):
        for style, tags in [('regular', ''), ('bold', '<w:b/>')]:
            variants.append(('mincho', 'MS Mincho', size, style, tags, '日本語' * 4))
    for face, family, size, style, tags, text in variants:
        xml = document.replace('A税あB ', text)
        xml, font_count = re.subn(r'<w:rFonts\b[^>]*/>', lambda m: m[0].replace('Arial', family) + tags, xml)
        xml, size_count = re.subn(r'(<w:sz(?:Cs)?\s+w:val=")\d+("/>)', lambda m: m[1] + str(int(size*2)) + m[2], xml)
        if font_count != 3 or size_count < 3:
            raise ValueError('Unexpected captured run/size structure')
        parts = dict(original)
        parts['word/document.xml'] = xml.encode()
        token = str(size).replace('.', 'p')
        name = f'word-windows-printer-metric-{face}-size{token}-{style}-compat15.docx'
        write_parts(output / name, parts)
        count += 1
    return count

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} long printer-metric controls')
