"""Isolate glyph advances and word-boundary rounding in original Aptos parents."""
from copy import deepcopy
from io import BytesIO
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo
import hashlib
import json

from lxml import etree as E
import generate_windows_aptos_size_probes as parents

ROOT = Path(__file__).resolve().parents[2]
PATTERNS = (
    ('r', 'r' * 48), ('a', 'a' * 48), ('y', 'y' * 48),
    ('hyphen', '-' * 48), ('one', '1' * 48),
    ('r-space', 'r ' * 24), ('ra-space', 'ra ' * 16),
    ('ray-space', 'ray ' * 12), ('X-ray-space', 'X-ray ' * 8),
    ('X-ray-negative-space', 'X-ray -12 ' * 5),
    ('12-space', '12 ' * 16), ('Xray-12-space', 'Xray 12 ' * 6),
)
q = parents.q


def generate(root=ROOT):
    records = json.loads((root / 'tests/word-oracle/windows/aptos-font-size-observations.json')
                         .read_text(encoding='utf-8-sig'))
    original = {r['name']: r for r in records['fixtures']}
    for parent_name, data, spec in parents.generate(root):
        if spec['font_size_pt'] not in (14, 15):
            continue
        parent = original[parent_name]['source']
        if hashlib.sha256(data).hexdigest() != parent['sha256']:
            raise ValueError('Original size parent changed')
        with ZipFile(BytesIO(data)) as source:
            tree = E.fromstring(source.read('word/document.xml'))
            body, = tree.iter(q('body'))
            paragraphs = list(body.findall(q('p')))
            if len(paragraphs) != 4:
                raise ValueError('Expected four original paragraphs')
            insert = body.index(paragraphs[0])
            for paragraph in paragraphs:
                body.remove(paragraph)
            for threshold, template in ((200, paragraphs[0]), (2, paragraphs[2])):
                if [int(k.get(q('val'))) for k in template.iter(q('kern'))] != [threshold]:
                    raise ValueError('Original kerning threshold changed')
                for _, text in PATTERNS:
                    paragraph = deepcopy(template)
                    node, = paragraph.iter(q('t'))
                    node.text = text
                    node.set('{http://www.w3.org/XML/1998/namespace}space', 'preserve')
                    body.insert(insert, paragraph)
                    insert += 1
            page, = tree.iter(q('pgSz'))
            page.set(q('w'), '17280')
            page.set(q('orient'), 'landscape')
            xml = E.tostring(tree, encoding='utf-8', xml_declaration=True)
            buffer = BytesIO()
            with ZipFile(buffer, 'w', compression=ZIP_DEFLATED) as target:
                for info in source.infolist():
                    item = ZipInfo(info.filename, (2026, 11, 19, 0, 0, 0))
                    item.create_system = 0
                    item.compress_type = ZIP_DEFLATED
                    item.external_attr = info.external_attr
                    target.writestr(item, xml if info.filename == 'word/document.xml'
                                    else source.read(info.filename))
        name = (f"word-windows-word-boundary-{spec['style']}-s{int(spec['font_size_pt'])}"
                f"-m{spec['compatibility_mode']}")
        yield name, buffer.getvalue(), dict(
            derived_parent=parent, mac_parent=parents.PARENT,
            mac_parent_sha256=parents.PARENT_SHA256, font_family=spec['font_family'],
            font_size_pt=spec['font_size_pt'], style=spec['style'],
            compatibility_mode=spec['compatibility_mode'],
            patterns=[dict(label=label, text=text) for label, text in PATTERNS],
            kerning_thresholds_halfpoints=[200, 2], paragraphs=24,
            page_width_twips=17280, changed_parts=['word/document.xml'])
