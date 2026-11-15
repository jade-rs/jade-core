"""Reproduce manual-line width controls independently at three font sizes."""
import argparse
from decimal import Decimal
from pathlib import Path

from generate_windows_paragraph_shading_probes import ROOT
from generate_windows_table_style_probes import read_parts, write_parts

SIZES = (12, 18, 22)
EQUIVALENT_WIDTHS = ('186', '186.5', '187', '187.5')
PARENT = 'word-windows-edge-justify-width-yu20-both-width188-balance1-compat15.docx'


def generate(output):
    base = read_parts(ROOT / 'tests/word-oracle/windows/observations/sources' / PARENT)
    output.mkdir(parents=True, exist_ok=False)
    count = 0
    for size in SIZES:
        for equivalent in EQUIVALENT_WIDTHS:
            width = Decimal(equivalent) * Decimal(size) / 20
            indent = (Decimal('425.2') - width) * 20
            if indent != int(indent):
                raise ValueError('Control width must be an exact twip')
            parts = dict(base)
            document = parts['word/document.xml'].decode()
            for before, after in (
                ('<w:sz w:val="40"/>', f'<w:sz w:val="{size * 2}"/>'),
                ('<w:szCs w:val="40"/>', f'<w:szCs w:val="{size * 2}"/>'),
                ('<w:ind w:right="4744"/>', f'<w:ind w:right="{int(indent)}"/>'),
            ):
                if document.count(before) != 1:
                    raise ValueError('Expected unchanged 20pt/188pt parent control')
                document = document.replace(before, after)
            parts['word/document.xml'] = document.encode()
            label = str(width).replace('.', 'p')
            name = f'word-windows-edge-manual-size-yu{size}-width{label}-balance1-compat15.docx'
            write_parts(output / name, parts)
            count += 1
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} size/width controls')
