"""Reproducible source-only controls for Word's device advance distribution."""
from io import BytesIO
import hashlib
import json
from pathlib import Path
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED
from lxml import etree as E
from check_platform_coordinates import ROOT

LENGTHS = (1, 2, 3, 4, 7, 8, 12, 15, 16, 17, 31, 32, 33, 47, 48, 49, 64, 65)
PARENTS = (
    ('positive-kerning-observations.json', 'word-windows-positive-kern-italic-s14-Z48-m14', 'Z', 'italic', 14),
    ('y-length-observations.json', 'word-windows-aptos-y-isolated48-regular-s15-m14', 'y', 'regular', 15),
)
q = lambda s: '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}' + s


def generate(root=ROOT):
    for index, parent_name, character, style, points in PARENTS:
        entries = json.loads((root / 'tests/word-oracle/windows' / index).read_bytes())['fixtures']
        parent, = [c for c in entries if c['name'] == parent_name]
        data = (root / parent['source']['file']).read_bytes()
        if hashlib.sha256(data).hexdigest() != parent['source']['sha256']:
            raise ValueError('Original device-remainder parent changed')
        for length in LENGTHS:
            with ZipFile(BytesIO(data)) as source:
                document = E.fromstring(source.read('word/document.xml'))
                texts = list(document.iter(q('t')))
                if len(texts) != (4 if style == 'italic' else 2) or any(t.text != character * 48 for t in texts):
                    raise ValueError('Original device-remainder text structure changed')
                for text in texts:
                    text.text = character * length
                changed = E.tostring(document, encoding='utf-8', xml_declaration=True, standalone=True)
                output = BytesIO()
                with ZipFile(output, 'w', compression=ZIP_DEFLATED) as target:
                    for item in source.infolist():
                        info = ZipInfo(item.filename, (2026, 11, 20, 0, 0, 0))
                        info.create_system = 0
                        info.compress_type = ZIP_DEFLATED
                        info.external_attr = item.external_attr
                        target.writestr(info, changed if item.filename == 'word/document.xml' else source.read(item.filename))
            name = f'word-windows-device-remainder-{style}-s{points}-{character}{length}-m14'
            specification = dict(derived_parent=parent['source'], parent_name=parent_name,
                                 font_family='Aptos Display', font_style=style, font_size_pt=points,
                                 compatibility_mode=14, character=character, length=length, paragraphs=len(texts),
                                 expected_non_whitespace_glyphs=length * len(texts), changed_parts=['word/document.xml'])
            yield name, output.getvalue(), specification
