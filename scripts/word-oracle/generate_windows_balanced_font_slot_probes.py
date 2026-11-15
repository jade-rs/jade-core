"""Reproduce captured balanced Arial/font-slot and East Asia hint controls."""
import argparse
from pathlib import Path

from generate_windows_paragraph_shading_probes import ROOT
from generate_windows_table_style_probes import read_parts, write_parts


def fonts(latin, east, hint=None):
    suffix = f' w:hint="{hint}"' if hint else ''
    return (f'<w:rFonts w:ascii="{latin}" w:hAnsi="{latin}" '
            f'w:eastAsia="{east}" w:cs="{latin}"{suffix}/>')


def variants():
    for size in (12, 20, 24):
        for balance in (0, 1):
            for alignment in ('left', 'both'):
                parent = f'word-windows-edge-calibri-auto-size{size}-{alignment}-balance{balance}-compat15.docx'
                name = f'word-windows-edge-arial-balanced-auto-size{size}-{alignment}-balance{balance}-compat15.docx'
                yield name, parent, [(fonts('Calibri', 'MS Mincho'), fonts('Arial', 'MS Mincho'))]
    for family in ('Arial', 'Calibri'):
        prefix = 'arial-balanced-auto' if family == 'Arial' else 'calibri-auto'
        for size in (12, 20, 24):
            for alignment in ('left', 'both'):
                parent = f'word-windows-edge-{prefix}-size{size}-{alignment}-balance1-compat15.docx'
                for hint in (None, 'eastAsia'):
                    name = (f'word-windows-edge-default-hint-{family.lower()}-size{size}-'
                            f'{alignment}-hint{hint or "none"}-balance1-compat15.docx')
                    yield name, parent, [(fonts(family, 'MS Mincho'), fonts(family, family, hint))]


def generate(output):
    sources = ROOT / 'tests/word-oracle/windows/observations/sources'
    output.mkdir(parents=True, exist_ok=False)
    count = 0
    for name, parent, edits in variants():
        parts = read_parts(sources / parent)
        document = parts['word/document.xml'].decode()
        for before, after in edits:
            if document.count(before) != 1:
                raise ValueError('Expected unchanged parent font slots')
            document = document.replace(before, after)
        parts['word/document.xml'] = document.encode()
        write_parts(output / name, parts)
        count += 1
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} font-slot controls')
