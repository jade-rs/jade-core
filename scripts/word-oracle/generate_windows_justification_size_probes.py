"""Reproduce 12 size-only variants of captured manual-break controls."""
import argparse
from pathlib import Path

from generate_windows_paragraph_shading_probes import ROOT
from generate_windows_table_style_probes import read_parts, write_parts


def generate(output):
    output.mkdir(parents=True, exist_ok=False)
    count = 0
    for size in (12, 18, 22):
        for alignment in ('left', 'both'):
            for balance in (0, 1):
                suffix = f'cjk3-space3-{alignment}-balance{balance}-compat15'
                source = ROOT / 'tests/word-oracle/windows/observations/sources' / (
                    'word-windows-edge-justify-tiers-yu20-' + suffix + '.docx')
                parts = read_parts(source)
                document = parts['word/document.xml'].decode()
                if (document.count('<w:sz w:val="40"/>') != 1
                        or document.count('<w:szCs w:val="40"/>') != 1):
                    raise ValueError('Parent must have one explicit 20pt run')
                document = document.replace('<w:sz w:val="40"/>', f'<w:sz w:val="{size * 2}"/>')
                document = document.replace('<w:szCs w:val="40"/>', f'<w:szCs w:val="{size * 2}"/>')
                parts['word/document.xml'] = document.encode()
                name = f'word-windows-edge-justify-size-yu{size}-{suffix}.docx'
                write_parts(output / name, parts)
                count += 1
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} size-only controls')
