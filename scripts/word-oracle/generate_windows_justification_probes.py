"""Reproduce 36 captured Word mixed-script controls without reference positions."""
import argparse
import hashlib
from pathlib import Path
import re

from generate_windows_paragraph_shading_probes import ROOT, SOURCE_SHA256, mixed_script, shaded_bases
from generate_windows_table_style_probes import read_parts, write_parts

TEXT_VARIANTS = (
    ('cjk3-no-space', 'Ag日本語ABCxyz0123'),
    ('cjk3-space1', 'Ag日本語 ABCxyz0123'),
    ('cjk6-space1', 'Ag日本語日本語 ABCxyz0123'),
    ('cjk3-space2', 'Ag日本語 ABCxyz0123 ABC'),
    ('cjk3-space3', 'Ag日本語 ABCxyz0123 ABC ABC'),
    ('cjk3-latin-cjk', 'Ag日本語 ABC 日本語'),
)


def generate(output):
    source = ROOT / 'public/fixtures/2col.docx'
    if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA256:
        raise ValueError('Original two-column source changed')
    base = shaded_bases(read_parts(source))['yu20']
    output.mkdir(parents=True, exist_ok=False)
    count = 0
    for alignment in ('left', 'both'):
        for balance in (False, True):
            suffix = f'{alignment}-balance{int(balance)}'
            for threshold in (0, 20, 198):
                parts = mixed_script(base, alignment, balance, 'normal')
                document = parts['word/document.xml'].decode()
                if '<w:kern' in document or document.count('</w:rPr>') != 1:
                    raise ValueError('Unexpected source run properties')
                parts['word/document.xml'] = document.replace(
                    '</w:rPr>', f'<w:kern w:val="{threshold}"/></w:rPr>').encode()
                name = f'word-windows-edge-justify-kern-yu20-{suffix}-kern{threshold}-compat15'
                write_parts(output / (name + '.docx'), parts)
                count += 1
            for label, text in TEXT_VARIANTS:
                parts = mixed_script(base, alignment, balance, 'normal')
                document, replaced = re.subn(
                    r'<w:t xml:space="preserve">.*?</w:t>',
                    '<w:t xml:space="preserve">' + text + '</w:t><w:br/><w:t>X</w:t>',
                    parts['word/document.xml'].decode(), flags=re.S)
                if replaced != 1:
                    raise ValueError('Unexpected source text')
                parts['word/document.xml'] = document.encode()
                settings, replaced = re.subn(
                    r'<w:doNotExpandShiftReturn\b[^>]*/>',
                    '<w:doNotExpandShiftReturn w:val="0"/>',
                    parts['word/settings.xml'].decode())
                if replaced != 1:
                    raise ValueError('Missing manual-break expansion setting')
                parts['word/settings.xml'] = settings.encode()
                name = f'word-windows-edge-justify-tiers-yu20-{label}-{suffix}-compat15'
                write_parts(output / (name + '.docx'), parts)
                count += 1
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print(f'Generated {generate(parser.parse_args().output)} justification controls')
