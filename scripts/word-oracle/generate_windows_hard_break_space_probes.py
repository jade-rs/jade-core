"""Reproduce ordinary trailing-space controls from immutable Word parents."""
from copy import deepcopy
import hashlib
from io import BytesIO
from pathlib import Path
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

from lxml import etree as E

ROOT = Path(__file__).resolve().parents[2]
PARENTS = {'Arial': {'name': 'word-windows-edge-terminal-script-arial-arial-size12-ea-space-both-balance0-compat15', 'source_sha256': 'c840f9ff889e25bc96852a29f5b5f80e1e206bdafe46bfb7782050a3d284f55e', 'capture_id': '20261115T131012751Z-8f0a8c24', 'archive': {'file': 'tests/word-oracle/windows/observations/word-16.0.19127.20936/20261115T131012751Z-8f0a8c24.zip', 'sha256': '1366aa5a48b325e3d35cd289acbda131673b14bd6e972418e9a1897023a4b7b8'}}, 'Yu Mincho': {'name': 'word-windows-edge-terminal-script-arial-yu-size12-ea-space-both-balance0-compat15', 'source_sha256': '79fc9c50c63200f902a5e569e61a9694ea2b8b0f2c9d7695cde0d20fb6da0587', 'capture_id': '20261115T131030534Z-92fd3521', 'archive': {'file': 'tests/word-oracle/windows/observations/word-16.0.19127.20936/20261115T131030534Z-92fd3521.zip', 'sha256': '2ab8e232fa3dc8f6f780fe5568638d160b96a70bc923d604b5abba40872a32bd'}}}
q = lambda n: '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}' + n


def generate(root=ROOT):
    mixed = {}
    for family, parent in PARENTS.items():
        archive = (root / parent['archive']['file']).read_bytes()
        if hashlib.sha256(archive).hexdigest() != parent['archive']['sha256']:
            raise ValueError('Original manual-break parent archive changed')
        with ZipFile(BytesIO(archive)) as bundle:
            data = bundle.read('inputs/' + parent['name'] + '.docx')
        if hashlib.sha256(data).hexdigest() != parent['source_sha256']:
            raise ValueError('Original manual-break parent input changed')
        for mode in (14, 15):
            for ending in ('manual', 'paragraph'):
                for count in (0, 1, 3):
                    with ZipFile(BytesIO(data)) as original:
                        document = E.fromstring(original.read('word/document.xml'))
                        settings = E.fromstring(original.read('word/settings.xml'))
                        paragraph, = document.iter(q('p'))
                        run, = paragraph.iter(q('r'))
                        first, second = list(run.iter(q('t')))
                        br, = run.iter(q('br'))
                        if not first.text.endswith('Ag日本語 ') or second.text != 'X':
                            raise ValueError('Original manual-break paragraph changed')
                        text = first.text.rstrip(' ') + ' ' * count
                        first.text = text
                        if ending == 'paragraph':
                            run.remove(br)
                            run.remove(second)
                        compat, = (c for c in settings.iter(q('compatSetting'))
                                   if c.get(q('name')) == 'compatibilityMode')
                        if compat.get(q('val')) != '15':
                            raise ValueError('Original compatibility mode changed')
                        compat.set(q('val'), str(mode))
                        changed = {'word/document.xml': E.tostring(document, encoding='utf-8',
                                   xml_declaration=True, standalone=True)}
                        if mode == 14:
                            changed['word/settings.xml'] = E.tostring(settings, encoding='utf-8',
                                xml_declaration=True, standalone=True)
                        if mode == 15 and ending == 'manual' and count == 1:
                            result, changed = data, {}
                        else:
                            buffer = BytesIO()
                            with ZipFile(buffer, 'w', compression=ZIP_DEFLATED) as target:
                                for info in original.infolist():
                                    item = ZipInfo(info.filename, (2026, 11, 19, 0, 0, 0))
                                    item.create_system = 0
                                    item.compress_type = ZIP_DEFLATED
                                    item.external_attr = info.external_attr
                                    target.writestr(item, changed.get(info.filename,
                                                                    original.read(info.filename)))
                            result = buffer.getvalue()
                    label = 'arial' if family == 'Arial' else 'yu'
                    name = f'word-windows-hard-break-ea-{label}-s12-m{mode}-{ending}-spaces{count}'
                    spec = dict(derived_parent=parent, character_set='mixed', east_asia_font=family,
                        font_family='Arial', font_size_pt=12, compatibility_mode=mode,
                        line_ending=ending, trailing_ascii_spaces=count, text=text,
                        following_text='X' if ending == 'manual' else '',
                        expected_non_whitespace_glyphs=sum(not c.isspace() for c in text)
                            + (ending == 'manual'), changed_parts=sorted(changed),
                        unchanged_parent_bytes=(result == data))
                    mixed[name] = (result, spec)
                    yield name, result, spec
    for mode in (14, 15):
        for ending in ('manual', 'paragraph'):
            parent_name = f'word-windows-hard-break-ea-arial-s12-m{mode}-{ending}-spaces0'
            data, _ = mixed[parent_name]
            for count in (0, 3):
                with ZipFile(BytesIO(data)) as original:
                    document = E.fromstring(original.read('word/document.xml'))
                    first = next(document.iter(q('t')))
                    text = 'AA BB' + ' ' * count
                    first.text = text
                    xml = E.tostring(document, encoding='utf-8', xml_declaration=True, standalone=True)
                    buffer = BytesIO()
                    with ZipFile(buffer, 'w', compression=ZIP_DEFLATED) as target:
                        for info in original.infolist():
                            target.writestr(deepcopy(info), xml if info.filename == 'word/document.xml'
                                            else original.read(info.filename))
                name = f'word-windows-hard-break-ascii-s12-m{mode}-{ending}-spaces{count}'
                yield name, buffer.getvalue(), dict(
                    derived_parent=dict(name=parent_name, source_sha256=hashlib.sha256(data).hexdigest()),
                    character_set='ascii', east_asia_font='Arial', font_family='Arial', font_size_pt=12,
                    compatibility_mode=mode, line_ending=ending, trailing_ascii_spaces=count, text=text,
                    following_text='X' if ending == 'manual' else '',
                    expected_non_whitespace_glyphs=4 + (ending == 'manual'),
                    changed_parts=['word/document.xml'], unchanged_parent_bytes=False)
