"""Reproduce original Word controls crossing small justification budgets."""
import argparse
from decimal import Decimal
from pathlib import Path
import re

from generate_windows_paragraph_shading_probes import ROOT
from generate_windows_table_style_probes import read_parts, write_parts

WIDTHS = (186, 188, 190, 192, 194, 198, 202, 210)


def generate(output):
    output.mkdir(parents=True, exist_ok=False)
    count = 0
    for balance in (0, 1):
        source = ROOT / 'tests/word-oracle/windows/observations/sources' / (
            f'word-windows-edge-justify-tiers-yu20-cjk3-space3-left-balance{balance}-compat15.docx')
        base = read_parts(source)
        for width in (None, *WIDTHS):
            parts = dict(base)
            document, replacements = re.subn(
                r'<w:t xml:space="preserve">.*?</w:t>',
                '<w:t xml:space="preserve">Ag日本語 ABC ABC</w:t>',
                parts['word/document.xml'].decode(), flags=re.S)
            if replacements != 1 or '<w:ind' in document:
                raise ValueError('Unexpected parent text or indentation')
            label = 'left'
            if width is not None:
                label = f'both-width{width}'
                indent = int((Decimal('425.2') - Decimal(width)) * 20)
                if document.count('<w:jc w:val="left"/>') != 1:
                    raise ValueError('Unexpected parent alignment')
                document = document.replace('<w:jc w:val="left"/>',
                    f'<w:ind w:right="{indent}"/><w:jc w:val="both"/>')
            parts['word/document.xml'] = document.encode()
            name = f'word-windows-edge-justify-width-yu20-{label}-balance{balance}-compat15.docx'
            write_parts(output / name, parts)
            count += 1
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} width controls')
