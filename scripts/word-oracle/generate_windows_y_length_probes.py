"""Vary only repeated-y lengths in captured Windows Word boundary parents."""
from copy import deepcopy
import hashlib
from io import BytesIO
import json
from pathlib import Path
from zipfile import ZipFile, ZIP_DEFLATED

from lxml import etree as E

ROOT = Path(__file__).resolve().parents[2]
LENGTHS = (1, 2, 3, 4, 5, 6, 8, 12, 16, 24, 32, 48, 64)
q = lambda name: '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}' + name


def generate(root=ROOT):
    index = json.loads((root / 'tests/word-oracle/windows/word-boundary-observations.json')
                       .read_text(encoding='utf-8-sig'))
    for style in ('regular', 'bold'):
        parent, = (r for r in index['fixtures'] if r['input_specification']['style'] == style
                   and r['input_specification']['font_size_pt'] == 15
                   and r['input_specification']['compatibility_mode'] == 14)
        data = (root / parent['source']['file']).read_bytes()
        if hashlib.sha256(data).hexdigest() != parent['source']['sha256']:
            raise ValueError('Original word-boundary parent changed')
        for layout, lengths in (('lengths', LENGTHS), ('isolated48', (48,))):
            with ZipFile(BytesIO(data)) as source:
                tree = E.fromstring(source.read('word/document.xml'))
                body, = tree.iter(q('body'))
                old = body.findall(q('p'))
                if len(old) != 24:
                    raise ValueError('Original boundary paragraph coverage changed')
                insert = body.index(old[0])
                for paragraph in old:
                    body.remove(paragraph)
                paragraphs = []
                for kerning, threshold, template in (('off', 200, old[2]), ('on', 2, old[14])):
                    if [t.text for t in template.iter(q('t'))] != ['y' * 48] or \
                            [int(k.get(q('val'))) for k in template.iter(q('kern'))] != [threshold]:
                        raise ValueError('Original repeated-y text or kerning changed')
                    for length in lengths:
                        paragraph = deepcopy(template)
                        node, = paragraph.iter(q('t'))
                        node.text = 'y' * length
                        body.insert(insert, paragraph)
                        insert += 1
                        paragraphs.append(dict(kerning=kerning, length=length, text=node.text))
                xml = E.tostring(tree, encoding='utf-8', xml_declaration=True, standalone=True)
                buffer = BytesIO()
                with ZipFile(buffer, 'w', compression=ZIP_DEFLATED) as target:
                    for info in source.infolist():
                        target.writestr(deepcopy(info), xml if info.filename == 'word/document.xml'
                                        else source.read(info.filename))
            name = f'word-windows-aptos-y-{layout}-{style}-s15-m14'
            yield name, buffer.getvalue(), dict(
                derived_parent=parent['source'], mac_parent=parent['input_specification']['mac_parent'],
                mac_parent_sha256=parent['input_specification']['mac_parent_sha256'],
                style=style, font_family='Aptos Display', font_size_pt=15, compatibility_mode=14,
                layout=layout, paragraphs=paragraphs, changed_parts=['word/document.xml'])
