"""Reproduce original bold/italic Word controls with a Calibri East Asian slot."""
import argparse
from pathlib import Path
import re

from generate_windows_paragraph_shading_probes import ROOT
from generate_windows_table_style_probes import read_parts, write_parts


def variants():
    for latin in ('Arial', 'Calibri'):
        for size in (12, 20, 24):
            for balance in (0, 1):
                if latin == 'Arial':
                    parent = f'word-windows-edge-cross-slot-arial-calibri-size{size}-left-balance{balance}-compat15.docx'
                elif balance == 0:
                    parent = f'word-windows-edge-cross-slot-calibri-calibri-size{size}-left-balance0-compat15.docx'
                else:
                    parent = f'word-windows-edge-default-hint-calibri-size{size}-left-hintnone-balance1-compat15.docx'
                for style, tags in [('bold', '<w:b/>'), ('italic', '<w:i/>'), ('bolditalic', '<w:b/><w:i/>')]:
                    name = (f'word-windows-edge-calibri-fallback-style-{latin.lower()}-ea-calibri-size{size}-'
                            f'left-{style}-balance{balance}-compat15.docx')
                    yield name, parent, tags


def generate(output):
    sources = ROOT / 'tests/word-oracle/windows/observations/sources'
    output.mkdir(parents=True, exist_ok=False)
    count = 0
    for name, parent, tags in variants():
        parts = read_parts(sources / parent)
        document = parts['word/document.xml'].decode()
        fonts, = re.findall(r'<w:rFonts\b[^>]*/>', document)
        if re.search(r'<w:(?:b|i)(?:\s|/|>)', document):
            raise ValueError('Expected regular parent run')
        parts['word/document.xml'] = document.replace(fonts, fonts + tags).encode()
        write_parts(output / name, parts)
        count += 1
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} Calibri fallback style controls')
