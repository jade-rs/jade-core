"""Isolate Word language selection and adjacent-run fallback cache behavior."""
import argparse
from pathlib import Path
import re

from generate_windows_table_style_probes import read_parts, write_parts

ROOT = Path(__file__).resolve().parents[2]


def generate(output):
    parent = ROOT / ('tests/word-oracle/windows/observations/sources/'
                     'word-windows-edge-cross-slot-calibri-calibri-size12-left-balance0-compat15.docx')
    original = read_parts(parent)
    # Inherited East Asian language would mask the w:val-only controls.
    for name, value in original.items():
        if name.endswith('.xml'):
            original[name] = re.sub(rb'<w:lang\b[^>]*/>', b'', value)
    document = original['word/document.xml'].decode()
    prefix = document[:document.index('<w:body>')]
    section, = re.findall(r'<w:sectPr\b.*?</w:sectPr>', document, re.S)
    paragraph, = re.findall(r'<w:pPr\b.*?</w:pPr>', document, re.S)
    # Use authored run properties throughout; the language is the only change
    # within each face/attribute matrix. No reference geometry is consulted.
    run_properties, = re.findall(r'<w:r><w:rPr>(.*?)</w:rPr>', document, re.S)
    run_properties = re.sub(r'<w:lang\b[^>]*/>', '', run_properties)
    output.mkdir(parents=True, exist_ok=False)
    count = 0
    languages = {'ja': 'ja-JP', 'zh': 'zh-TW', 'ko': 'ko-KR'}
    for face in ('Calibri', 'Arial'):
        properties = run_properties.replace('Calibri', face)
        for attribute in ('val', 'eastAsia'):
            for sequence in ('ja-ja-ja', 'zh-zh-zh', 'ko-ko-ko', 'ja-zh-ko', 'ko-zh-ja'):
                runs = ''
                for language in sequence.split('-'):
                    tag = (f'<w:lang w:val="{languages[language]}"/>' if attribute == 'val'
                           else f'<w:lang w:val="en-US" w:eastAsia="{languages[language]}"/>')
                    runs += ('<w:r><w:rPr>' + properties + tag
                             + '</w:rPr><w:t xml:space="preserve">A税あB </w:t></w:r>')
                parts = dict(original)
                parts['word/document.xml'] = (prefix + '<w:body><w:p>' + paragraph
                    + runs + '</w:p>' + section + '</w:body></w:document>').encode()
                name = f'word-windows-edge-language-{face.lower()}-{attribute.lower()}-{sequence}-compat15.docx'
                write_parts(output / name, parts)
                count += 1
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} language controls')
