"""Reproduce versioned styled controls with Word autoSpaceDE explicitly disabled."""
import argparse
from pathlib import Path

from generate_windows_calibri_fallback_style_probes import ROOT, variants as styled_variants
from generate_windows_table_style_probes import read_parts, write_parts


def variants():
    for parent, _, _ in styled_variants():
        name = parent.replace('-fallback-style-', '-fallback-autooff-')
        yield name, parent


def generate(output):
    sources = ROOT / 'tests/word-oracle/windows/observations/sources'
    output.mkdir(parents=True, exist_ok=False)
    count = 0
    for name, parent in variants():
        parts = read_parts(sources / parent)
        document = parts['word/document.xml'].decode()
        tag = '<w:jc w:val="left"/>'
        if document.count(tag) != 1 or '<w:autoSpaceDE' in document:
            raise ValueError('Expected one left-aligned paragraph without autoSpaceDE')
        parts['word/document.xml'] = document.replace(tag, '<w:autoSpaceDE w:val="0"/>' + tag).encode()
        write_parts(output / name, parts)
        count += 1
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} auto-spacing-disabled controls')
