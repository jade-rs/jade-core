"""Keep text/font geometry while varying run boundaries and foreground color."""
from copy import deepcopy
import hashlib
from io import BytesIO
from zipfile import ZipFile, ZIP_DEFLATED

from lxml import etree as E
from check_platform_coordinates import ROOT, checked_file, read_json

STYLES = (('bold', 14), ('bolditalic', 15), ('italic', 14), ('regular', 14))
INTERVALS = (1, 2, 4, 8, 16, 32)
COLORS = ('explicit-black', 'alternating-nearblack')


def q(name):
    return '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}' + name


def split_runs(original, interval, color):
    with ZipFile(BytesIO(original)) as before:
        root = E.fromstring(before.read('word/document.xml'))
        paragraphs = list(root.iter(q('p')))
        assert len(paragraphs) == 4
        for paragraph in paragraphs:
            run, = paragraph.findall(q('r'))
            text, = run.iter(q('t'))
            assert {child.tag for child in run} <= {q('rPr'), q('t')}
            value = text.text
            assert value
            at = list(paragraph).index(run)
            paragraph.remove(run)
            for number, start in enumerate(range(0, len(value), interval)):
                chunk = deepcopy(run)
                t, = chunk.iter(q('t'))
                t.text = value[start:start + interval]
                t.set('{http://www.w3.org/XML/1998/namespace}space', 'preserve')
                properties = chunk.find(q('rPr'))
                if properties is None:
                    properties = E.Element(q('rPr')); chunk.insert(0, properties)
                for old in properties.findall(q('color')):
                    properties.remove(old)
                c = E.SubElement(properties, q('color'))
                c.set(q('val'), '000001' if color == 'alternating-nearblack' and number % 2 else '000000')
                paragraph.insert(at + number, chunk)
            assert ''.join(t.text or '' for t in paragraph.iter(q('t'))) == value
        changed = E.tostring(root, encoding='UTF-8', xml_declaration=True, standalone=True)
        output = BytesIO()
        with ZipFile(output, 'w', compression=ZIP_DEFLATED) as after:
            for entry in before.infolist():
                after.writestr(entry, changed if entry.filename == 'word/document.xml'
                               else before.read(entry.filename))
    return output.getvalue()


def generate(root=ROOT):
    parents = read_json(root / 'tests/word-oracle/windows/aptos-font-size-observations.json')['fixtures']
    result = []
    for style, size in STYLES:
        parent = next(c for c in parents if c['name'] ==
                      f'word-windows-aptos-size-control-display-{style}-size{size}-compat14')
        original = checked_file(root, parent['source']).read_bytes()
        for interval, color in [(0, 'control')] + [(i, c) for i in INTERVALS for c in COLORS]:
            name = f'word-windows-color-span-{style}-s{size}-m14-step{interval}-{color}'
            data = original if interval == 0 else split_runs(original, interval, color)
            facts = dict(name=name, source_sha256=hashlib.sha256(data).hexdigest(),
                         parent=parent['source'], parent_case=parent['name'],
                         mac_parent=parent['input_specification']['mac_parent'],
                         mac_parent_sha256=parent['input_specification']['mac_parent_sha256'],
                         style=style, font_size_pt=size, compatibility_mode=14, interval=interval,
                         color_mode=color, changed_parts=[] if interval == 0 else ['word/document.xml'],
                         scope='Originaltext/font/paragraph/packageparts preserved; onlyrunsegmentation '
                         'andexplicitfontcolor change. Intervals1/2/4/8/16/32 systematic, no '
                         'expectedcoordinates used. Step0byteexactcontrol.')
            result.append((name, data, facts))
    assert len(result) == 52
    return result
