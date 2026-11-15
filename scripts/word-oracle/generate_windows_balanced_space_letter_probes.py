"""Reproduce Word controls that distinguish balanced font-average cells from pair kerning."""
import argparse
from pathlib import Path
from generate_windows_calibri_fallback_style_probes import ROOT
from generate_windows_table_style_probes import read_parts, write_parts


def variants():
    for family in ('arial', 'calibri'):
        for style in ('regular', 'bold', 'italic', 'bolditalic'):
            if style == 'regular':
                parent = ('word-windows-edge-cross-slot-arial-calibri-size24-left-balance1-compat15.docx'
                          if family == 'arial' else
                          'word-windows-edge-default-hint-calibri-size24-left-hintnone-balance1-compat15.docx')
            else:
                parent = f'word-windows-edge-calibri-fallback-autooff-{family}-ea-calibri-size24-left-{style}-balance1-compat15.docx'
            for letter in ('B', 'T'):
                name = f'word-windows-edge-space-face-{family}-{style}-next{letter}-size24-balance1-compat15.docx'
                yield name, parent, letter, style == 'regular'


def generate(output):
    sources = ROOT / 'tests/word-oracle/windows/observations/sources'
    output.mkdir(parents=True, exist_ok=False)
    count = 0
    for name, parent, letter, disable_auto in variants():
        parts = read_parts(sources / parent)
        xml = parts['word/document.xml'].decode()
        if disable_auto:
            tag = '<w:jc w:val="left"/>'
            if xml.count(tag) != 1 or '<w:autoSpaceDE' in xml:
                raise ValueError('Unexpected parent paragraph')
            xml = xml.replace(tag, '<w:autoSpaceDE w:val="0"/>' + tag)
        if xml.count('Ag日本語 ABCABC') != 1:
            raise ValueError('Unexpected parent text')
        parts['word/document.xml'] = xml.replace('Ag日本語 ABCABC', f'Ag日本語 {letter}BCABC').encode()
        write_parts(output / name, parts)
        count += 1
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} balanced space letter controls')
