"""Source-only controls for paragraph-order effects on Word PDF text spacing."""
from copy import deepcopy
from io import BytesIO
import hashlib
import json
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

from lxml import etree as E
from check_platform_coordinates import ROOT

PARENT_INDEX = Path('tests/word-oracle/windows/y-length-observations.json')
ORDERS = (('off', 'on'), ('on', 'off'), ('on',), ('off',),
          ('on', 'on'), ('on', 'off', 'on'))
q = lambda name: '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}' + name


def generate(root=ROOT):
    index = json.loads((root / PARENT_INDEX).read_bytes())
    for style in ('regular', 'bold'):
        parent, = [r for r in index['fixtures']
                   if r['name'] == f'word-windows-aptos-y-isolated48-{style}-s15-m14']
        data = (root / parent['source']['file']).read_bytes()
        if hashlib.sha256(data).hexdigest() != parent['source']['sha256']:
            raise ValueError('Original repeated-y parent changed')
        for mode in (14, 15):
            for order in ORDERS:
                with ZipFile(BytesIO(data)) as source:
                    document = E.fromstring(source.read('word/document.xml'))
                    settings = E.fromstring(source.read('word/settings.xml'))
                    body, = document.iter(q('body'))
                    old = body.findall(q('p'))
                    if len(old) != 2:
                        raise ValueError('Original repeated-y paragraph structure changed')
                    templates = dict(zip(('off', 'on'), old))
                    for label, paragraph in templates.items():
                        if [n.text for n in paragraph.iter(q('t'))] != ['y' * 48]:
                            raise ValueError('Original repeated-y text changed')
                        kern, = paragraph.iter(q('kern'))
                        if int(kern.get(q('val'))) != (200 if label == 'off' else 2):
                            raise ValueError('Original repeated-y kerning threshold changed')
                    insert = body.index(old[0])
                    for paragraph in old:
                        body.remove(paragraph)
                    for label in order:
                        body.insert(insert, deepcopy(templates[label]))
                        insert += 1
                    compatibility, = [n for n in settings.iter(q('compatSetting'))
                                      if n.get(q('name')) == 'compatibilityMode']
                    compatibility.set(q('val'), str(mode))
                    changed = {
                        name: E.tostring(tree, encoding='utf-8', xml_declaration=True, standalone=True)
                        for name, tree in [('word/document.xml', document), ('word/settings.xml', settings)]
                    }
                    output = BytesIO()
                    with ZipFile(output, 'w', compression=ZIP_DEFLATED) as target:
                        for item in source.infolist():
                            info = ZipInfo(item.filename, (2026, 11, 20, 0, 0, 0))
                            info.create_system = 0
                            info.compress_type = ZIP_DEFLATED
                            info.external_attr = item.external_attr
                            target.writestr(info, changed.get(item.filename, source.read(item.filename)))
                name = f'word-windows-y-pdf-context-{"-".join(order)}-{style}-s15-m{mode}'
                specification = dict(
                    derived_parent=parent['source'], parent_name=parent['name'],
                    font_family='Aptos Display', font_style=style, font_size_pt=15,
                    compatibility_mode=mode, line_ending='paragraph',
                    paragraphs=[dict(kerning=k, threshold_half_points=200 if k == 'off' else 2,
                                     text='y' * 48, length=48) for k in order],
                    expected_non_whitespace_glyphs=48 * len(order),
                    changed_parts=list(changed))
                yield name, output.getvalue(), specification
