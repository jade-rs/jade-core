"""Reproduce font-slot, terminal-space and Calibri auto-spacing controls."""
import argparse
from pathlib import Path

from generate_windows_paragraph_shading_probes import ROOT
from generate_windows_table_style_probes import read_parts, write_parts

SHORT_TEXT = 'Ag日本語 ABC ABC'
LONG_TEXT = 'Ag日本語 ABCxyz0123 paragraph shading geometry.'
YU_FONTS = '<w:rFonts w:ascii="Yu Mincho" w:hAnsi="Yu Mincho" w:eastAsia="Yu Mincho" w:cs="Yu Mincho"/>'


def changes(latin, east, size, text, alignment):
    return [
        (YU_FONTS, f'<w:rFonts w:ascii="{latin}" w:hAnsi="{latin}" w:eastAsia="{east}" w:cs="{latin}"/>'),
        ('<w:sz w:val="40"/>', f'<w:sz w:val="{size * 2}"/>'),
        ('<w:szCs w:val="40"/>', f'<w:szCs w:val="{size * 2}"/>'),
        (SHORT_TEXT, text),
        ('<w:jc w:val="left"/>', f'<w:jc w:val="{alignment}"/>'),
    ]


def parent(balance):
    return f'word-windows-edge-justify-width-yu20-left-balance{balance}-compat15.docx'


def variants():
    profiles = [
        ('arial-arial', 'Arial', 'Arial', 0),
        ('arial-yu', 'Arial', 'Yu Mincho', 0),
        ('yu-ms', 'Yu Mincho', 'MS Mincho', 0),
        ('calibri-ms', 'Calibri', 'MS Mincho', 0),
        ('arial-arial', 'Arial', 'Arial', 1),
        ('yu-ms', 'Yu Mincho', 'MS Mincho', 1),
    ]
    for profile, latin, east, balance in profiles:
        for alignment in ('left', 'both'):
            name = f'word-windows-edge-fallback-residual-{profile}-size12-{alignment}-balance{balance}-compat15.docx'
            yield name, parent(balance), changes(latin, east, 12, LONG_TEXT, alignment)
    for profile in ('arial-arial', 'arial-yu'):
        source = f'word-windows-edge-fallback-residual-{profile}-size12-left-balance0-compat15.docx'
        for ending, tail in [('ea', ' Ag日本語'), ('ea-space', ' Ag日本語 '), ('ea-latin', ' Ag日本語 A')]:
            for alignment in ('left', 'both'):
                name = f'word-windows-edge-terminal-script-{profile}-size12-{ending}-{alignment}-balance0-compat15.docx'
                yield name, source, [
                    (LONG_TEXT, LONG_TEXT + tail),
                    ('<w:jc w:val="left"/>', f'<w:jc w:val="{alignment}"/>'),
                ]
    for size in (12, 20, 24):
        for balance in (0, 1):
            for alignment in ('left', 'both'):
                name = f'word-windows-edge-calibri-auto-size{size}-{alignment}-balance{balance}-compat15.docx'
                yield name, parent(balance), changes('Calibri', 'MS Mincho', size, 'Ag日本語 ABCABC', alignment)


def generate(output):
    sources = ROOT / 'tests/word-oracle/windows/observations/sources'
    output.mkdir(parents=True, exist_ok=False)
    count = 0
    for name, source, edits in variants():
        parts = read_parts(sources / source)
        document = parts['word/document.xml'].decode()
        for before, after in edits:
            if document.count(before) != 1:
                raise ValueError('Expected unchanged parent text, fonts and alignment')
            document = document.replace(before, after)
        parts['word/document.xml'] = document.encode()
        write_parts(output / name, parts)
        count += 1
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} spacing controls')
