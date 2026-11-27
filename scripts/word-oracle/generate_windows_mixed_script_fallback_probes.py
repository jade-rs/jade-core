"""Measure mixed-script fallback within the same authored run."""
import argparse
from pathlib import Path

from generate_windows_table_style_probes import read_parts, write_parts

ROOT = Path(__file__).resolve().parents[2]


def generate(output):
    output.mkdir(parents=True, exist_ok=False)
    count = 0
    for face in ('arial', 'calibri'):
        for language in ('ja', 'zh', 'ko'):
            parent = ROOT / ('tests/word-oracle/windows/observations/sources/'
                f'word-windows-edge-language-{face}-eastasia-'
                f'{language}-{language}-{language}-compat15.docx')
            original = read_parts(parent)
            document = original['word/document.xml'].decode()
            if document.count('A税あB ') != 3:
                raise ValueError('Expected three identical captured language-control runs')
            for script, text in [('jpan-hang', '税한'), ('hang-jpan', '한税'),
                                 ('hans-hang', '汉한')]:
                parts = dict(original)
                parts['word/document.xml'] = document.replace('A税あB ', f'A{text}B ').encode()
                name = f'word-windows-edge-mixed-script-fallback-{face}-{script}-{language}-compat15.docx'
                write_parts(output / name, parts)
                count += 1
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} script/language fallback controls')
