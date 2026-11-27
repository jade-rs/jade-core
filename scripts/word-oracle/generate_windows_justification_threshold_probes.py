"""Reproduce four original Word controls at a manual-line fit threshold."""
import argparse
from decimal import Decimal
from pathlib import Path

from generate_windows_paragraph_shading_probes import ROOT
from generate_windows_table_style_probes import read_parts, write_parts

WIDTHS = ('186.5', '187', '187.5', '188.5')


def generate(output):
    source = ROOT / 'tests/word-oracle/windows/observations/sources' / (
        'word-windows-edge-justify-width-yu20-both-width188-balance1-compat15.docx')
    base = read_parts(source)
    output.mkdir(parents=True, exist_ok=False)
    for width in WIDTHS:
        parts = dict(base)
        document = parts['word/document.xml'].decode()
        if document.count('<w:ind w:right="4744"/>') != 1:
            raise ValueError('Expected the unchanged 188pt parent control')
        indent = int((Decimal('425.2') - Decimal(width)) * 20)
        parts['word/document.xml'] = document.replace(
            '<w:ind w:right="4744"/>', f'<w:ind w:right="{indent}"/>').encode()
        name = 'word-windows-edge-justify-threshold-yu20-both-width' + width.replace('.', 'p')
        write_parts(output / (name + '-balance1-compat15.docx'), parts)
    return len(WIDTHS)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} threshold controls')
