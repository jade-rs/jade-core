"""Reproduce the independently captured Windows legacy text-effect controls."""
import argparse
import hashlib
from pathlib import Path
import re

from generate_windows_table_style_probes import read_parts, write_parts

ROOT = Path(__file__).resolve().parents[2]
SOURCE_SHA256 = '9483a3b8154cde513b6d77fe91f25883404f80ba1de5ed1eedef4b88919b631d'
EFFECTS = ('plain', 'shadow', 'outline', 'emboss', 'imprint')


def generate(output):
    source = ROOT / 'public/fixtures/char-effects.docx'
    assert hashlib.sha256(source.read_bytes()).hexdigest() == SOURCE_SHA256
    original = read_parts(source)
    document = original['word/document.xml'].decode()
    section, = re.findall(r'<w:sectPr\b.*?</w:sectPr>', document, re.S)
    output.mkdir(parents=True, exist_ok=False)
    count = 0

    def write(name, parts):
        nonlocal count
        write_parts(output / (name + '.docx'), parts)
        count += 1

    for color, sizes in [('auto', [12, 20, 40]), ('FF0000', [20])]:
        for size in sizes:
            color_xml = '' if color == 'auto' else f'<w:color w:val="{color}"/>'
            normal = f'{color_xml}<w:sz w:val="{size * 2}"/>'
            for effect in EFFECTS:
                tag = '' if effect == 'plain' else f'<w:{effect}/>'
                paragraph = (f'<w:p><w:r><w:rPr>{normal}</w:rPr>'
                             '<w:t xml:space="preserve">AVToGlyph0123 </w:t></w:r>'
                             f'<w:r><w:rPr>{tag}{normal}</w:rPr>'
                             '<w:t xml:space="preserve">AVToGlyph0123</w:t></w:r></w:p>')
                changed, replacements = re.subn(r'(?<=<w:body>).*?(?=</w:body>)',
                                                paragraph + section, document, flags=re.S)
                assert replacements == 1
                parts = dict(original)
                parts['word/document.xml'] = changed.encode()
                write(f'word-windows-edge-graphic-{effect}-size{size}-color{color}-compat15', parts)

    for effect in EFFECTS:
        parts20 = read_parts(output / f'word-windows-edge-graphic-{effect}-size20-colorauto-compat15.docx')
        for face, label in [('Arial', 'arial'), ('Calibri', 'calibri'), ('Yu Mincho', 'yu-mincho')]:
            parts = dict(parts20)
            fonts = (f'<w:rFonts w:ascii="{face}" w:hAnsi="{face}" '
                     f'w:eastAsia="{face}" w:cs="{face}"/>')
            text = parts['word/document.xml'].decode()
            assert text.count('<w:rPr>') == 2
            parts['word/document.xml'] = text.replace('<w:rPr>', '<w:rPr>' + fonts).encode()
            write(f'word-windows-edge-graphic-{effect}-{label}-size20-compat15', parts)
        for mode in [12, 14]:
            parts = dict(parts20)
            settings, replacements = re.subn(
                r'(<w:compatSetting\b[^>]*w:name="compatibilityMode"[^>]*w:val=")15("[^>]*/>)',
                rf'\g<1>{mode}\2', parts['word/settings.xml'].decode())
            assert replacements == 1
            parts['word/settings.xml'] = settings.encode()
            write(f'word-windows-edge-graphic-{effect}-size20-colorauto-compat{mode}', parts)

    parts = dict(original)
    text, replacements = re.subn(rb'<w:(shadow|outline|emboss|imprint)\s*/>', b'',
                                 parts['word/document.xml'])
    assert replacements == 4
    parts['word/document.xml'] = text
    write('word-windows-edge-graphic-original-plain-font-control', parts)
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-directory', type=Path, required=True)
    args = parser.parse_args()
    print('Generated', generate(args.output_directory), 'Windows graphic text controls')
