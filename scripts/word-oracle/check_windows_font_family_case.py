"""Assert actual Word font-family case invariance without moving reference origins."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
from zipfile import ZipFile
import xml.etree.ElementTree as ET

from check_platform_coordinates import ROOT, checked_file, pdf_origins, read_json, require
from check_platform_coordinates import assert_pending_pdf_text
from check_windows_aptos_sizes import assert_physical_font_sizes
from check_windows_font_coordinates import require_original_font_programs_registered
from windows_xps_fonts import extract_xps_fonts

INDEX = Path('tests/word-oracle/windows/font-family-cases.json')
INDEX_SHA256 = '53d88417ab5c5b549c5b669959f99360a9dc6ea49b73159e37459de5403efb92'
Y_TOLERANCE_PT = .375  # Same absolute tolerance as the established coordinate gate.
W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_source_pair(source, canonical, family):
    with ZipFile(source) as alias, ZipFile(canonical) as control:
        require(alias.namelist() == control.namelist(), 'Font alias changed package members')
        require(all(alias.read(name) == control.read(name) for name in alias.namelist()
                    if name != 'word/document.xml'), 'Font alias changed another DOCX part')
        left = ET.fromstring(alias.read('word/document.xml'))
        right = ET.fromstring(control.read('word/document.xml'))
        alias_fonts, control_fonts = list(left.iter(W + 'rFonts')), list(right.iter(W + 'rFonts'))
        require(len(alias_fonts) == len(control_fonts) == 4, 'Font alias run coverage changed')
        for a, b in zip(alias_fonts, control_fonts):
            require(a.keys() == b.keys() and all(v == family for v in a.attrib.values())
                    and all(v == 'Aptos Display' for v in b.attrib.values()),
                    'Font alias source family changed')
            a.attrib.update(b.attrib)
        require(ET.tostring(left) == ET.tostring(right), 'Font alias changed other run properties')


def verify(root=ROOT):
    require(sha(root / INDEX) == INDEX_SHA256, 'Font-family case coverage changed')
    index = read_json(root / INDEX)
    require(index['schema'] == 1 and len(index['fixtures']) == 48, 'Font-family case coverage removed')
    names, controls, cases = set(), set(), []
    for case in index['fixtures']:
        require(case['name'] not in names, 'Duplicate font-family case')
        names.add(case['name'])
        require(case['family'] != 'Aptos Display'
                and case['family'].lower() == 'aptos display', 'Invalid family case variant')
        groups = []
        for record in (case, case['canonical']):
            paths = {key: checked_file(root, record[key])
                     for key in ('source', 'capture', 'pdf', 'xps')}
            capture = read_json(paths['capture'])
            require(capture['word'] == index['word'], 'Font-family case Word version changed')
            document, = [d for d in capture['documents'] if d['name'] == record['name']]
            require(document['sha256'] == record['source']['sha256']
                    and document['open_mode'] == 'normal-read-only', 'Wrong alias source/open mode')
            for key in ('pdf', 'xps'):
                require(paths[key] == paths['capture'].parent / document[key]
                        and record[key]['sha256'] == document[key + '_sha256'],
                        'Wrong alias original output')
            groups.append(paths)
        alias, canonical = groups
        verify_source_pair(alias['source'], canonical['source'], case['family'])
        require(pdf_origins(alias['pdf']) == pdf_origins(canonical['pdf']),
                'Original Word case invariance changed')
        controls.add(case['canonical']['source']['sha256'])
        cases.append((case, alias, canonical))
    require(len(controls) == 16, 'Canonical case-control coverage removed')
    return cases


def assert_vertical_origins(expected, actual, label):
    require(actual['page_count'] == expected['page_count'], label + ': page count')
    a = [g for g in actual['glyphs'] if not g[1].isspace()]
    b = [g for g in expected['glyphs'] if not g[1].isspace()]
    require(len(a) == len(b) == 240, label + ': glyph coverage')
    for index, (word, jade) in enumerate(zip(b, a)):
        require(word[:2] == jade[:2] and all(math.isfinite(jade[k]) for k in (2, 3)),
                f'{label}/{index}: text/page/order/nonfinite origin')
        require(abs(word[3] - jade[3]) <= Y_TOLERANCE_PT,
                f'{label}/{index}: Word vertical origin {word[3]} differs from {jade[3]}')


def render(executable, source, folder, fonts, root):
    folder.mkdir(parents=True, exist_ok=False)
    with (folder / 'renderer.log').open('x', encoding='utf-8') as log:
        subprocess.run([str(executable), str(source), str(folder / 'native.json'), 'windows',
                        str(folder / 'native.pdf'), '--fonts-directory', str(fonts)],
                       check=True, stdout=log, stderr=subprocess.STDOUT,
                       env=dict(os.environ, JADE_WORD_ORACLE_ROOT=str(root)))
    return read_json(folder / 'native.json')


def check(executable, output, root=ROOT):
    cases = verify(root)
    executable = executable.resolve()
    native_sha = sha(executable)
    output.mkdir(parents=True, exist_ok=False)
    controls, results = {}, []
    for index, (case, alias, canonical) in enumerate(cases):
        folder = output / f'c{index:02d}'
        folder.mkdir()
        fonts = folder / 'fonts'
        extract_xps_fonts(alias['xps'], fonts, output / 'font-cache')
        programs = tuple(sorted(sha(p) for p in fonts.iterdir()
                                if p.suffix.lower() in ('.ttf', '.otf', '.ttc')))
        require(bool(programs), case['name'] + ': missing original fonts')
        key = case['canonical']['source']['sha256'], programs
        if key not in controls:
            control_folder = output / f't{len(controls):02d}'
            control = render(executable, canonical['source'], control_folder, fonts, root)
            require(control['input_sha256'] == key[0], 'Canonical renderer source changed')
            require_original_font_programs_registered(fonts, control)
            controls[key] = control, control_folder
        control, control_folder = controls[key]
        actual = render(executable, alias['source'], folder / 'alias', fonts, root)
        require(actual['input_sha256'] == case['source']['sha256'], 'Alias renderer source changed')
        require_original_font_programs_registered(fonts, actual)
        require(actual['page_count'] == control['page_count']
                and actual['glyphs'] == control['glyphs'],
                case['name'] + ': native family-case origins differ from the canonical family')
        require((folder / 'alias/native.png').read_bytes()
                == (control_folder / 'native.png').read_bytes(),
                case['name'] + ': family case changed screen paint')
        painted = folder / 'alias/native.pdf'
        require(pdf_origins(painted) == pdf_origins(control_folder / 'native.pdf'),
                case['name'] + ': family case changed physical PDF origins')
        assert_physical_font_sizes(alias['pdf'], painted, case['name'])
        assert_pending_pdf_text(pdf_origins(alias['pdf']), pdf_origins(painted), case['name'])
        assert_vertical_origins(pdf_origins(alias['pdf']), actual, case['name'])
        assert_vertical_origins(pdf_origins(alias['pdf']), pdf_origins(painted), case['name'])
        results.append(dict(name=case['name'],source_sha256=case['source']['sha256'],
                            Word_font_size_and_vertical_origins='PASS',
                            native_case_glyphs_screen_PDF_exact=True))
        print(case['name'] + ': Word size/Y and native case invariance PASS', flush=True)
    require(len(results) == 48 and sha(executable) == native_sha, 'Incomplete case/native identity')
    (output / 'results.json').write_text(json.dumps(dict(native_sha256=native_sha,cases=results),
                                                   indent=2) + '\n', encoding='utf-8')
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output', type=Path, default=Path('target/windows-font-family-cases'))
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable, args.output)
    else:
        print(f'Verified {len(verify())} original Word family-case references')
