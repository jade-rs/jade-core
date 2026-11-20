"""Compare mixed Japanese/Latin family-case controls with original Word outputs."""
import argparse
import json
from pathlib import Path
from zipfile import ZipFile
import xml.etree.ElementTree as ET

from check_platform_coordinates import ROOT, checked_file, compare, pdf_origins, read_json, require
from check_platform_coordinates import assert_pending_pdf_text
from check_windows_font_family_case import render, sha
from check_windows_font_coordinates import require_original_font_programs_registered
from check_windows_xps import assert_model
from extract_windows_emf import extract
from paragraph_backgrounds import compare_paragraph_backgrounds
from windows_xps_fonts import extract_xps_fonts

INDEX = Path('tests/word-oracle/windows/font-family-cjk-cases.json')
INDEX_SHA256 = 'b9803ef8f03a29d69eb7c706382861a3a6c12e93e1eb1d30a700e38878e097a8'
W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def verify_source_pair(source, canonical, family):
    with ZipFile(source) as alias, ZipFile(canonical) as control:
        require(alias.namelist() == control.namelist(), 'Mixed alias changed package members')
        require(all(alias.read(n) == control.read(n) for n in alias.namelist()
                    if n != 'word/document.xml'), 'Mixed alias changed another DOCX part')
        left, right = [ET.fromstring(z.read('word/document.xml')) for z in (alias, control)]
        a, b = list(left.iter(W + 'rFonts')), list(right.iter(W + 'rFonts'))
        require(len(a) == len(b) == 2, 'Mixed alias run coverage changed')
        for first, second in zip(a, b):
            require(second.attrib == {W + k: v for k, v in (
                ('ascii', 'Aptos Display'), ('hAnsi', 'Aptos Display'),
                ('eastAsia', 'Arial'), ('cs', 'Aptos Display'))}
                    and first.attrib == {k: family if v == 'Aptos Display' else v
                                         for k, v in second.attrib.items()},
                    'Mixed alias requested font changed')
            first.attrib.update(second.attrib)
        require(ET.tostring(left) == ET.tostring(right), 'Mixed alias changed other source properties')


def verify(root=ROOT):
    require(sha(root / INDEX) == INDEX_SHA256, 'Mixed font-family case index changed')
    index = read_json(root / INDEX)
    require(index['schema'] == 1 and len(index['fixtures']) == 96, 'Mixed case coverage removed')
    names, controls, coverage, cases = set(), set(), set(), []
    for case in index['fixtures']:
        require(case['name'] not in names, 'Duplicate mixed case')
        names.add(case['name'])
        require(case['family'] in ('aptos display', 'APTOS DISPLAY', 'aPtOs dIsPlAy'),
                'Mixed font-family spelling changed')
        groups = []
        for record in (case, case['canonical']):
            paths = {key: checked_file(root, record[key])
                     for key in ('source', 'capture', 'pdf', 'xps')}
            capture = read_json(paths['capture'])
            require(capture['word'] == index['word'], 'Mixed case Word identity changed')
            document, = [d for d in capture['documents'] if d['name'] == record['name']]
            require(document['sha256'] == record['source']['sha256']
                    and document['open_mode'] == 'normal-read-only', 'Mixed alias source/open mode')
            for key in ('pdf', 'xps'):
                require(paths[key] == paths['capture'].parent / document[key]
                        and record[key]['sha256'] == document[key + '_sha256'],
                        'Mixed case original output changed')
            groups.append(paths)
        alias, canonical = groups
        verify_source_pair(alias['source'], canonical['source'], case['family'])
        require(pdf_origins(alias['pdf']) == pdf_origins(canonical['pdf']),
                'Original mixed Word coordinates differ across font-family spellings')
        controls.add(case['canonical']['source']['sha256'])
        coverage.add((case['family'], case['compatibility_mode'],
                      case['line_ending'], case['line_width_twips'], case['style']))
        cases.append((case, alias, canonical))
    require(len(controls) == 32 and coverage == {
        (f, m, e, w, s) for f in ('aptos display', 'APTOS DISPLAY', 'aPtOs dIsPlAy')
        for m in (14, 15) for e in ('manual', 'paragraph') for w in (3000, 8504)
        for s in ('regular', 'bold', 'italic', 'bold-italic')
    }, 'Mixed case compatibility/ending/width coverage removed')
    return cases


def assert_origins(case, alias, actual, painted, emf):
    name = case['name']
    compare(emf, actual, name + '/EMF')
    compare(pdf_origins(alias['pdf'], True), actual, name + '/PDF-unclipped')
    compare(pdf_origins(alias['pdf']), actual, name + '/PDF-visible')
    compare(pdf_origins(alias['pdf']), pdf_origins(painted), name + '/painted-PDF')
    assert_model(pdf_origins(alias['xps'], True), actual, name + '/XPS-native', True)
    assert_model(pdf_origins(alias['xps'], True), actual, name + '/XPS-visible', True)
    for unclipped in (False, True):
        assert_pending_pdf_text(pdf_origins(alias['pdf'], unclipped),
                                pdf_origins(painted, unclipped), name)
    compare_paragraph_backgrounds(alias['source'], alias['pdf'], painted, name)


def check(executable, output, root=ROOT):
    cases = verify(root)
    executable = executable.resolve()
    native_sha = sha(executable)
    output.mkdir(parents=True, exist_ok=False)
    controls, emfs, rows = {}, {}, []
    for number, (case, alias, canonical) in enumerate(cases):
        folder = output / f'c{number:02d}'
        folder.mkdir()
        fonts = folder / 'fonts'
        extract_xps_fonts(alias['xps'], fonts, output / 'font-cache')
        programs = tuple(sorted(sha(p) for p in fonts.iterdir()
                                if p.suffix.lower() in ('.ttf', '.otf', '.ttc')))
        require(bool(programs), case['name'] + ': original fonts absent')
        key = case['canonical']['source']['sha256'], programs
        if key not in controls:
            target = output / f't{len(controls):02d}'
            control = render(executable, canonical['source'], target, fonts, root)
            require(control['input_sha256'] == key[0], 'Mixed canonical source changed')
            require_original_font_programs_registered(fonts, control)
            controls[key] = control, target
        control, target = controls[key]
        actual = render(executable, alias['source'], folder / 'alias', fonts, root)
        require(actual['input_sha256'] == case['source']['sha256'], 'Mixed alias source changed')
        require_original_font_programs_registered(fonts, actual)
        require(actual['page_count'] == control['page_count']
                and actual['glyphs'] == control['glyphs'],
                case['name'] + ': mixed family-case origins differ from canonical family')
        require((folder / 'alias/native.png').read_bytes() == (target / 'native.png').read_bytes(),
                case['name'] + ': mixed family case changed screen paint')
        painted = folder / 'alias/native.pdf'
        require(pdf_origins(painted) == pdf_origins(target / 'native.pdf'),
                case['name'] + ': mixed family case changed painted origins')
        if alias['capture'] not in emfs:
            emfs[alias['capture']] = {r['name']: r for r in extract(alias['capture'])['fixtures']}
        assert_origins(case, alias, actual, painted, emfs[alias['capture']][case['name']])
        rows.append(dict(name=case['name'], source_sha256=case['source']['sha256'],
                         Word_original_EMF_PDF_XPS='PASS', native_case_glyphs_screen_PDF_exact=True))
        print(case['name'] + ': original Word coordinates and case invariance PASS', flush=True)
    require(len(rows) == 96 and sha(executable) == native_sha, 'Mixed case/native coverage changed')
    (output / 'results.json').write_text(json.dumps(dict(native_sha256=native_sha, cases=rows),
                                                   indent=2) + '\n', encoding='utf-8')
    return rows


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output', type=Path, default=Path('target/windows-font-family-cjk-cases'))
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable, args.output)
    else:
        print(f'Verified {len(verify())} original mixed Word font-family case controls')
