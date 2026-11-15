"""Reproduce the Japanese legacy-effect inputs captured by Windows Word."""
import argparse
from pathlib import Path
import tempfile

from generate_windows_graphic_text_probes import generate as generate_base
from generate_windows_table_style_probes import read_parts, write_parts

ROOT = Path(__file__).resolve().parents[2]
TEXT = '日本語漢字かなカナＡＶ１２３'
EFFECTS = ('plain', 'shadow', 'outline', 'emboss', 'imprint')


def generate(output):
    output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory() as directory:
        controls = Path(directory) / 'base'
        assert generate_base(controls) == 46
        for size in [12, 20, 40]:
            for effect in EFFECTS:
                source = controls / f'word-windows-edge-graphic-{effect}-size{size}-colorauto-compat15.docx'
                parts = read_parts(source)
                text = parts['word/document.xml'].decode()
                assert text.count('AVToGlyph0123') == 2 and text.count('<w:rPr>') == 2
                text = text.replace('AVToGlyph0123', TEXT)
                fonts = ('<w:rFonts w:ascii="Yu Mincho" w:hAnsi="Yu Mincho" '
                         'w:eastAsia="Yu Mincho" w:cs="Yu Mincho"/>')
                parts['word/document.xml'] = text.replace('<w:rPr>', '<w:rPr>' + fonts).encode()
                write_parts(output / (
                    f'word-windows-edge-graphic-ja-yu-mincho-{effect}-size{size}-compat15.docx'), parts)
    return 15


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-directory', type=Path, required=True)
    args = parser.parse_args()
    print('Generated', generate(args.output_directory), 'Japanese graphic text controls')
