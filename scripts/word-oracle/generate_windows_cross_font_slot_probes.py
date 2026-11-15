"""Reproduce original Word controls that change only the East Asian font slot."""
import argparse
from pathlib import Path

from generate_windows_paragraph_shading_probes import ROOT
from generate_windows_table_style_probes import read_parts, write_parts


def variants():
    for latin, east, balances in [('Arial', 'Arial', [0]), ('Calibri', 'Calibri', [0]),
                                  ('Calibri', 'Arial', [0, 1]), ('Arial', 'Calibri', [0, 1])]:
        prefix = 'arial-balanced-auto' if latin == 'Arial' else 'calibri-auto'
        for size in (12, 20, 24):
            for balance in balances:
                for alignment in ('left', 'both'):
                    parent = f'word-windows-edge-{prefix}-size{size}-{alignment}-balance{balance}-compat15.docx'
                    name = (f'word-windows-edge-cross-slot-{latin.lower()}-{east.lower()}-size{size}-'
                            f'{alignment}-balance{balance}-compat15.docx')
                    yield name, parent, [('w:eastAsia="MS Mincho"', f'w:eastAsia="{east}"')]


def generate(output):
    sources = ROOT / 'tests/word-oracle/windows/observations/sources'
    output.mkdir(parents=True, exist_ok=False)
    count = 0
    for name, parent, edits in variants():
        parts = read_parts(sources / parent)
        document = parts['word/document.xml'].decode()
        for before, after in edits:
            if document.count(before) != 1:
                raise ValueError('Expected unchanged East Asian font slot')
            document = document.replace(before, after)
        parts['word/document.xml'] = document.encode()
        write_parts(output / name, parts)
        count += 1
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} cross-font controls')
