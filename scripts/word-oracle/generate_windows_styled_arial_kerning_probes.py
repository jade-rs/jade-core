"""Reproduce original Word controls for every observed styled Arial kern pair."""
import argparse
import json
from pathlib import Path
import re
from xml.sax.saxutils import escape

from generate_windows_table_style_probes import read_parts, write_parts

ROOT = Path(__file__).resolve().parents[2]


def generate(output):
    facts = json.loads((ROOT / 'tests/word-oracle/windows/styled-arial-kerning-font-facts.json').read_text(encoding='utf8'))
    pairs = sorted({key for face in facts['faces'] for key, _ in face['pairs']})
    original = read_parts(ROOT / 'tests/word-oracle/windows/observations/sources/word-windows-edge-language-arial-eastasia-ja-ja-ja-compat15.docx')
    document = original['word/document.xml'].decode()
    paragraphs = list(re.finditer(r'<w:p(?:\s[^>]*)?>.*?</w:p>', document, re.S))
    if len(paragraphs) != 1 or paragraphs[0].group().count('A税あB ') != 3:
        raise ValueError('Expected one captured parent paragraph with three runs')
    output.mkdir(parents=True, exist_ok=False)
    count = 0
    for plain in (False, True):
        for size in (12, 24):
            for style, tags in [('bold', '<w:b/>'), ('italic', '<w:i/>'), ('bolditalic', '<w:b/><w:i/>')]:
                template, font_count = re.subn(r'(<w:rFonts\b[^>]*/>)', lambda m: m[1] + tags, paragraphs[0].group())
                template, size_count = re.subn(r'(<w:sz(?:Cs)?\s+w:val=")\d+("/>)', lambda m: m[1] + str(size * 2) + m[2], template)
                if font_count != 3 or size_count < 3:
                    raise ValueError('Unexpected parent font slots or sizes')
                body = ''.join(template.replace('A税あB ', escape('Q' + chr(key >> 8) + chr(key & 255) + 'Q ')) for key in pairs)
                if plain:
                    body, shading_count = re.subn(r'<w:shd\b[^>]*/>', '', body)
                    if shading_count != len(pairs):
                        raise ValueError('Expected one original shading per pair paragraph')
                parts = dict(original)
                parts['word/document.xml'] = (document[:paragraphs[0].start()] + body + document[paragraphs[0].end():]).encode()
                suffix = 'plain-' if plain else ''
                write_parts(output / f'word-windows-styled-arial-pair-matrix-{suffix}size{size}-{style}-compat15.docx', parts)
                count += 1
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} source-exact styled Arial pair controls')
