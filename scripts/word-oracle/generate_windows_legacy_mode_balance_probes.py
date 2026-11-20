"""Reproduce settings-only Windows Word legacy-mode and balanced controls."""
from pathlib import Path
from io import BytesIO
from zipfile import ZipFile
import hashlib
import json
from lxml import etree as E
from generate_windows_aptos_spacing_probes import changed_parts, xml, q
ROOT = Path(__file__).resolve().parents[2]
PARENTS = Path('tests/word-oracle/windows/legacy-justification-observations.json')
PARENTS_SHA256 = '2f0cba4117165e39fac724f4232216072263697861aa958072937c3051aabe31'


def generate(root=ROOT):
    data = (root / PARENTS).read_bytes()
    if hashlib.sha256(data).hexdigest() != PARENTS_SHA256:
        raise ValueError('Original legacy-mode parent registry changed')
    for row in json.loads(data)['fixtures']:
        spec = row['input_specification']
        if not row['name'].startswith('word-windows-legacy-font-') or spec['compatibility_mode'] != 14:
            continue
        settings_cases = [(12, False)]
        if spec['line_width_twips'] in (3000, 8504):
            settings_cases += [(14, True), (15, True)]
        for mode, balance in settings_cases:
            source = (root / row['source']['file']).read_bytes()
            if hashlib.sha256(source).hexdigest() != row['source']['sha256']:
                raise ValueError('Original legacy-mode parent source changed')
            with ZipFile(BytesIO(source)) as package:
                settings = E.fromstring(package.read('word/settings.xml'))
            compatibility, = [n for n in settings.iter(q('compatSetting')) if n.get(q('name')) == 'compatibilityMode']
            compatibility.set(q('val'), str(mode))
            balanced, = settings.iter(q('balanceSingleByteDoubleByteWidth'))
            balanced.set(q('val'), '1' if balance else '0')
            name = row['name'].replace('legacy-font', 'legacy-mode-balance').replace('-m14-', f'-m{mode}-balance{int(balance)}-')
            facts = dict(derived_parent=row['source'], parent_name=row['name'], font_family=spec['font_family'], font_size_pt=spec['font_size_pt'], bold=spec['bold'], compatibility_mode=mode, balance_single_byte_double_byte_width=balance, line_ending=spec['line_ending'], line_width_twips=spec['line_width_twips'], text=spec['text'], following_text=spec['following_text'], expected_non_whitespace_glyphs=spec['expected_non_whitespace_glyphs'], changed_parts=['word/settings.xml'])
            yield name, changed_parts(source, {'word/settings.xml': xml(settings)}), facts
