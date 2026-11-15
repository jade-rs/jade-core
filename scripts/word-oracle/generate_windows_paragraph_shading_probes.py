"""Reproduce 45 original Word shading/script controls without reference positions."""
import argparse
import hashlib
from pathlib import Path
import re

from generate_windows_table_style_probes import read_parts, write_parts

ROOT = Path(__file__).resolve().parents[2]
SOURCE_SHA256 = '02e7f6a0da7e08ab377c854bb0fc1b1085298900266c063a473bcab134e20ff8'


def shaded_bases(parts):
    original = parts['word/document.xml'].decode()
    section, = re.findall(r'<w:sectPr\b.*?</w:sectPr>', original, re.S)
    variants = [('base', {}), ('yu12', dict(size=12)), ('yu20', dict(size=20)),
        ('arial12', dict(face='Arial', size=12)), ('arial20', dict(face='Arial', size=20)),
        ('no-grid', dict(grid=False)),
        ('exact24', dict(ppr='<w:spacing w:line="480" w:lineRule="exact"/>')),
        ('space-before12-after6', dict(ppr='<w:spacing w:before="240" w:after="120"/>')),
        ('indent-left12-right18', dict(ppr='<w:ind w:left="240" w:right="360"/>')),
        ('first12', dict(ppr='<w:ind w:firstLine="240"/>')),
        ('hanging12', dict(ppr='<w:ind w:hanging="240"/>')),
        ('margin72', dict(margin=1440)), ('center', dict(ppr='<w:jc w:val="center"/>')),
        ('cols2', dict(columns=2))]
    result = {}
    for label, changes in variants:
        config = dict(face='Yu Mincho', size=10.5, grid=True, columns=1, margin=1701, ppr='')
        config.update(changes)
        font = ('<w:rFonts ' + ' '.join(f'w:{key}="{config["face"]}"'
                for key in ['ascii', 'hAnsi', 'eastAsia', 'cs']) + '/>'
                + f'<w:sz w:val="{int(config["size"] * 2)}"/>'
                + f'<w:szCs w:val="{int(config["size"] * 2)}"/>')
        text = 'Ag日本語 ABCxyz0123 paragraph shading geometry. ' * 8
        paragraph = ('<w:p><w:pPr><w:shd w:val="clear" w:color="auto" w:fill="D9EAD3"/>'
            + config['ppr'] + '</w:pPr><w:r><w:rPr>' + font
            + '</w:rPr><w:t xml:space="preserve">' + text + '</w:t></w:r></w:p>')
        sec = section.replace('w:num="2"', f'w:num="{config["columns"]}"')
        sec = sec.replace('w:left="1701"', f'w:left="{config["margin"]}"')
        sec = sec.replace('w:right="1701"', f'w:right="{config["margin"]}"')
        if not config['grid']:
            sec = re.sub(r'<w:docGrid\b[^>]*/>', '', sec)
        document, count = re.subn(r'(?<=<w:body>).*?(?=</w:body>)', paragraph + sec, original, flags=re.S)
        assert count == 1
        changed = dict(parts)
        changed['word/document.xml'] = document.encode()
        result[label] = changed
    return result


def mixed_script(parts, alignment, balance, extra):
    changed = dict(parts)
    document = changed['word/document.xml'].decode()
    assert '<w:jc ' not in document
    ppr = f'<w:jc w:val="{alignment}"/>'
    if extra == 'mark20':
        ppr += '<w:rPr><w:sz w:val="40"/><w:szCs w:val="40"/></w:rPr>'
    document = document.replace('</w:pPr>', ppr + '</w:pPr>')
    if extra == 'no-grid':
        document, count = re.subn(r'<w:docGrid\b[^>]*/>', '', document)
        assert count == 1
    changed['word/document.xml'] = document.encode()
    settings, count = re.subn(r'<w:balanceSingleByteDoubleByteWidth\b[^>]*/>',
        f'<w:balanceSingleByteDoubleByteWidth w:val="{int(balance)}"/>',
        changed['word/settings.xml'].decode())
    assert count == 1
    changed['word/settings.xml'] = settings.encode()
    if extra == 'defaults20':
        styles = changed['word/styles.xml'].decode()
        start = styles.index('<w:docDefaults>')
        end = styles.index('</w:docDefaults>') + len('</w:docDefaults>')
        defaults = styles[start:end].replace('<w:sz w:val="21"/>', '<w:sz w:val="40"/>')
        defaults = defaults.replace('<w:szCs w:val="24"/>', '<w:szCs w:val="40"/>')
        changed['word/styles.xml'] = (styles[:start] + defaults + styles[end:]).encode()
    return changed


def generate(output):
    source = ROOT / 'public/fixtures/2col.docx'
    if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA256:
        raise ValueError('Original two-column document changed')
    parts = read_parts(source)
    output.mkdir(parents=True, exist_ok=False)
    for color, fill in [('green', 'D9EAD3'), ('yellow', 'FFF2CC')]:
        for columns in (1, 2):
            document = parts['word/document.xml'].decode()
            shading = f'<w:pPr><w:shd w:val="clear" w:color="auto" w:fill="{fill}"/></w:pPr>'
            document, count = re.subn(r'(<w:p\b[^>]*>)', lambda match: match[0] + shading,
                document, count=1)
            assert count == 1
            document = document.replace('w:num="2"', f'w:num="{columns}"')
            changed = dict(parts)
            changed['word/document.xml'] = document.encode()
            write_parts(output / f'word-windows-edge-column-background-{color}-cols{columns}-compat15.docx', changed)
    bases = shaded_bases(parts)
    for label, changed in bases.items():
        write_parts(output / f'word-windows-edge-shading-{label}-compat15.docx', changed)
    variants = [(base, align, balance, 'normal') for base in ['base', 'yu20', 'arial12']
        for align in ['left', 'center', 'both'] for balance in (False, True)]
    variants += [('yu20', 'left', True, extra) for extra in ['no-grid', 'mark20', 'defaults20']]
    for base, align, balance, extra in variants:
        name = f'word-windows-edge-mixed-{base}-{align}-balance{int(balance)}-{extra}-compat15'
        write_parts(output / (name + '.docx'), mixed_script(bases[base], align, balance, extra))
    variants = [('arial12', True, extra) for extra in ['defaults20', 'mark20', 'no-grid']]
    variants += [('arial20', False, 'normal'), ('arial20', True, 'normal'), ('arial20', True, 'defaults20')]
    for base, balance, extra in variants:
        name = f'word-windows-edge-arial-space-{base}-balance{int(balance)}-{extra}-compat15'
        write_parts(output / (name + '.docx'), mixed_script(bases[base], 'left', balance, extra))
    return 45


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    print('Generated', generate(parser.parse_args().output), 'Word shading controls')
