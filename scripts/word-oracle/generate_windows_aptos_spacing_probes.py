"""Reproduce original Windows Word Aptos and mixed-size controls."""
from pathlib import Path
from io import BytesIO
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED
import copy
import hashlib
import json
from lxml import etree as E

ROOT = Path(__file__).resolve().parents[2]
PARENTS = Path('tests/word-oracle/windows/legacy-justification-observations.json')
PARENTS_SHA256 = '2f0cba4117165e39fac724f4232216072263697861aa958072937c3051aabe31'
q = lambda name: '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}' + name


def changed_parts(source, replacements):
    output = BytesIO()
    with ZipFile(BytesIO(source)) as package, ZipFile(output, 'w', compression=ZIP_DEFLATED) as target:
        for info in package.infolist():
            item = ZipInfo(info.filename, (2026, 11, 19, 0, 0, 0))
            item.create_system = 0
            item.compress_type = ZIP_DEFLATED
            item.external_attr = info.external_attr
            target.writestr(item, replacements.get(info.filename, package.read(info.filename)))
    return output.getvalue()


def xml(root):
    return E.tostring(root, encoding='utf-8', xml_declaration=True, standalone=True)


def generate(root=ROOT):
    data = (root / PARENTS).read_bytes()
    if hashlib.sha256(data).hexdigest() != PARENTS_SHA256:
        raise ValueError('Original Aptos/size parent registry changed')
    parents = {row['name']: row for row in json.loads(data)['fixtures']}

    def parent(mode, ending, width):
        row = parents[f'word-windows-legacy-font-calibri-s12-m{14 if mode == 12 else mode}-{ending}-w{width}']
        source = (root / row['source']['file']).read_bytes()
        if hashlib.sha256(source).hexdigest() != row['source']['sha256']:
            raise ValueError('Original Aptos/size parent bytes changed')
        with ZipFile(BytesIO(source)) as package:
            document = E.fromstring(package.read('word/document.xml'))
        return row, source, document

    faces = [('Arial', 'Calibri', 'arial-calibri'), ('Arial', 'Times New Roman', 'arial-times'), ('Calibri', 'Times New Roman', 'calibri-times')]
    aptos = [('Aptos', 'Aptos', 'aptos'), ('Aptos Display', 'Aptos Display', 'aptos-display'), ('Aptos', 'Arial', 'aptos-arial'), ('Aptos Display', 'Calibri', 'aptos-display-calibri')]
    for group, pairs, sizes, widths in [('mixed-size', faces, ((10, 16), (16, 10)), (8504,)), ('aptos-mixed-face', aptos, ((12, 12),), (3000, 8504))]:
        for left, right, label in pairs:
            for leftsize, rightsize in sizes:
                for mode in (14, 15):
                    for ending in ('manual', 'paragraph'):
                        for width in widths:
                            row, source, document = parent(mode, ending, width)
                            paragraph, = document.iter(q('p'))
                            second, = paragraph.iter(q('r'))
                            first = copy.deepcopy(second)
                            for node in list(first):
                                if node.tag != q('rPr'):
                                    first.remove(node)
                            text = E.Element(q('t'))
                            text.set('{http://www.w3.org/XML/1998/namespace}space', 'preserve')
                            text.text = 'Ag日本語 '
                            first.append(text)
                            next(second.iter(q('t'))).text = 'ABC Ag日本語'
                            for run, family, size in ((first, left, leftsize), (second, right, rightsize)):
                                properties = run.find(q('rPr'))
                                fonts = properties.find(q('rFonts'))
                                for slot in ('ascii', 'hAnsi', 'cs'):
                                    fonts.set(q(slot), family)
                                if group == 'mixed-size':
                                    for tag in ('sz', 'szCs'):
                                        properties.find(q(tag)).set(q('val'), str(size * 2))
                            paragraph.insert(list(paragraph).index(second), first)
                            size_label = f'{leftsize}-{rightsize}' if group == 'mixed-size' else '12'
                            name = f'word-windows-{group}-{label}-s{size_label}-m{mode}-{ending}-w{width}'
                            spec = dict(derived_parent=row['source'], left_font_family=left, right_font_family=right, compatibility_mode=mode, line_ending=ending, line_width_twips=width, left_text='Ag日本語 ', right_text='ABC Ag日本語', following_text='X' if ending == 'manual' else '', expected_non_whitespace_glyphs=14 if ending == 'manual' else 13, changed_parts=['word/document.xml'])
                            if group == 'mixed-size':
                                spec.update(left_font_size_pt=leftsize, right_font_size_pt=rightsize)
                            else:
                                spec['font_size_pt'] = 12
                            yield name, changed_parts(source, {'word/document.xml': xml(document)}), spec
    for family, label in [('Aptos', 'aptos'), ('Aptos Display', 'aptos-display')]:
        for mode in (12, 14, 15):
            for balance in (0, 1):
                for bold, italic, style in [(False, False, 'regular'), (True, False, 'bold'), (False, True, 'italic'), (True, True, 'bolditalic')]:
                    for ending in ('manual', 'paragraph'):
                        row, source, document = parent(mode, ending, 8504)
                        with ZipFile(BytesIO(source)) as package:
                            settings = E.fromstring(package.read('word/settings.xml'))
                        run, = document.iter(q('r'))
                        properties = run.find(q('rPr'))
                        fonts = properties.find(q('rFonts'))
                        for slot in ('ascii', 'hAnsi', 'cs'):
                            fonts.set(q(slot), family)
                        at = 1
                        for enabled, tags in ((bold, ('b', 'bCs')), (italic, ('i', 'iCs'))):
                            for tag in tags:
                                if properties.find(q(tag)) is not None:
                                    raise ValueError('Original style changed')
                                if enabled:
                                    properties.insert(at, E.Element(q(tag)))
                                    at += 1
                        balanced, = settings.iter(q('balanceSingleByteDoubleByteWidth'))
                        balanced.set(q('val'), str(balance))
                        compatibility, = [n for n in settings.iter(q('compatSetting')) if n.get(q('name')) == 'compatibilityMode']
                        compatibility.set(q('val'), str(mode))
                        name = f'word-windows-aptos-style-{label}-{style}-s12-m{mode}-balance{balance}-{ending}'
                        spec = dict(derived_parent=row['source'], font_family=family, font_size_pt=12, bold=bold, italic=italic, style=style, compatibility_mode=mode, balance_single_byte_double_byte_width=bool(balance), line_ending=ending, line_width_twips=8504, text='Ag日本語 ABC Ag日本語', following_text='X' if ending == 'manual' else '', expected_non_whitespace_glyphs=14 if ending == 'manual' else 13, changed_parts=['word/document.xml', 'word/settings.xml'])
                        yield name, changed_parts(source, {'word/document.xml': xml(document), 'word/settings.xml': xml(settings)}), spec
