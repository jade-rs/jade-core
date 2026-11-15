"""Reproduce paired left/justified controls for a small expansion remainder."""
import argparse
from decimal import Decimal
from pathlib import Path

from generate_windows_paragraph_shading_probes import ROOT
from generate_windows_table_style_probes import read_parts, write_parts


def variants():
    for size in (12, 18, 22):
        for balance in (0, 1):
            for alignment in ('left', 'both'):
                width = Decimal(202) * size / 20
                suffix = 'left' if alignment == 'left' else 'both-width202'
                parent = f'word-windows-edge-justify-width-yu20-{suffix}-balance{balance}-compat15.docx'
                changes = [
                    ('<w:sz w:val="40"/>', f'<w:sz w:val="{size * 2}"/>'),
                    ('<w:szCs w:val="40"/>', f'<w:szCs w:val="{size * 2}"/>'),
                ]
                if alignment == 'both':
                    indent = (Decimal('425.2') - width) * 20
                    if indent != int(indent):
                        raise ValueError('Expected an exact twip width')
                    changes.append(('<w:ind w:right="4464"/>', f'<w:ind w:right="{int(indent)}"/>'))
                label = 'left' if alignment == 'left' else 'both-width' + str(width).replace('.', 'p')
                name = f'word-windows-edge-small-slack-yu{size}-{label}-balance{balance}-compat15.docx'
                yield name, parent, changes


def generate(output):
    sources = ROOT / 'tests/word-oracle/windows/observations/sources'
    output.mkdir(parents=True, exist_ok=False)
    count = 0
    for name, parent, changes in variants():
        parts = read_parts(sources / parent)
        document = parts['word/document.xml'].decode()
        for before, after in changes:
            if document.count(before) != 1:
                raise ValueError('Expected an unchanged original 20pt control')
            document = document.replace(before, after)
        parts['word/document.xml'] = document.encode()
        write_parts(output / name, parts)
        count += 1
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} paired controls')
