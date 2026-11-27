"""Assert every original Windows Word font-spacing control without pending exemptions."""
import argparse
import json
from pathlib import Path
from zipfile import ZipFile
import xml.etree.ElementTree as ET

from check_platform_coordinates import ROOT, checked_file, read_json, require
from check_windows_font_family_case import render, sha
from check_windows_font_family_extended_case import observe_word, require_promoted_word
from check_windows_font_coordinates import require_original_font_programs_registered
from extract_windows_emf import extract
from windows_xps_fonts import extract_xps_fonts
from check_windows_mincho_sizes import verify_source

INDEX = Path('tests/word-oracle/windows/font-spacing-cases.json')
INDEX_SHA256 = '4538ea047642f0a22f73ec606505448c451f61ee35dceeb3553786f0415afd3d'
W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def verify_context(source, case):
    with ZipFile(source) as package:
        document = ET.fromstring(package.read('word/document.xml'))
        settings = ET.fromstring(package.read('word/settings.xml'))
    balance = settings.find(W + 'compat/' + W + 'balanceSingleByteDoubleByteWidth')
    actual_balance = balance is not None and balance.get(W + 'val', '1') not in ('0', 'false', 'off')
    require(type(case['balanced']) is bool and actual_balance == case['balanced'],
            'Font spacing authored balance changed')
    body = document.find(W + 'body')
    paragraphs = body.findall(W + 'p')
    require(case['line_ending'] in ('manual', 'paragraph') and len(paragraphs) == 1,
            'Font spacing authored ending changed')
    require(len(list(paragraphs[0].iter(W + 'br')))
            == (1 if case['line_ending'] == 'manual' else 0),
            'Font spacing authored break changed')
    section = body.find(W + 'sectPr')
    page = section.find(W + 'pgSz')
    margins = section.find(W + 'pgMar')
    indent = paragraphs[0].find(W + 'pPr/' + W + 'ind')
    width = int(page.get(W + 'w')) - int(margins.get(W + 'left')) - int(margins.get(W + 'right'))
    if indent is not None:
        width -= int(indent.get(W + 'left', '0')) + int(indent.get(W + 'right', '0'))
    require(width == case['line_width_twips'], 'Font spacing authored width changed')


def verify(root=ROOT):
    require(sha(root / INDEX) == INDEX_SHA256, 'Font spacing size index changed')
    index = read_json(root / INDEX)
    require(index['schema'] == 1 and len(index['fixtures']) == 256, 'Font spacing size coverage removed')
    coverage, sources, cases = set(), set(), []
    for case in index['fixtures']:
        paths = {key: checked_file(root, case[key]) for key in ('source', 'capture', 'pdf', 'xps')}
        capture = read_json(paths['capture'])
        require(capture['word'] == index['word'] and capture['status'] == 'complete'
                and capture['platform'] == 'windows', 'Font spacing Word identity changed')
        document, = [d for d in capture['documents'] if d['name'] == case['name']]
        require(document['sha256'] == case['source']['sha256']
                and document['open_mode'] == 'normal-read-only'
                and document['compatibility_mode'] == case['compatibility_mode'],
                'Font spacing original source/open mode changed')
        for key in ('pdf', 'xps'):
            require(paths[key] == paths['capture'].parent / document[key]
                    and case[key]['sha256'] == document[key + '_sha256'],
                    'Font spacing original output changed')
        verify_source(paths['source'], case)
        verify_context(paths['source'], case)
        sources.add(case['source']['sha256'])
        coverage.add((case['family'], case['style'], case['font_size_pt'], case['compatibility_mode'],
                      case['line_ending'], case['line_width_twips'], case['balanced']))
        cases.append((case, paths))
    styles = ('regular', 'bold', 'italic', 'bolditalic')
    expected = {(family, style, size, mode, 'manual', 8504, False)
                for family in ('Yu Mincho', 'MS Mincho') for style in styles
                for size in (8, 10, 14, 16, 19, 24) for mode in (14, 15)}
    expected |= {(family, style, size, mode, 'paragraph', 8504, True)
                 for family in ('Century', 'Bookman Old Style') for style in styles
                 for size in (8, 10, 14, 16, 19, 24) for mode in (14, 15)}
    expected |= {(family, style, 12, mode, ending, width, True)
                 for family in ('Century', 'Bookman Old Style') for style in styles
                 for mode in (14, 15) for ending in ('manual', 'paragraph') for width in (3000, 8504)}
    require(coverage == expected and len(sources) == 256, 'Font spacing context matrix removed')
    return cases


def check(executable, output, root=ROOT):
    cases = verify(root)
    native_sha = sha(executable)
    output.mkdir(parents=True, exist_ok=False)
    emfs, rows = {}, []
    for number, (case, paths) in enumerate(cases):
        target = output / f'c{number:03d}'
        fonts = target / 'fonts'
        extract_xps_fonts(paths['xps'], fonts, output / 'font-cache')
        native = render(executable.resolve(), paths['source'], target / 'rendered', fonts, root)
        require(native['input_sha256'] == case['source']['sha256'], 'Font spacing rendered wrong input')
        require_original_font_programs_registered(fonts, native)
        if paths['capture'] not in emfs:
            emfs[paths['capture']] = {c['name']: c for c in extract(paths['capture'])['fixtures']}
        observed = observe_word(case, paths, native, target / 'rendered/native.pdf',
                                emfs[paths['capture']][case['name']])
        require_promoted_word(dict(case, required_word_coordinates=True), observed)
        rows.append(dict(name=case['name'], source_sha256=case['source']['sha256'],
                         original_Word=observed, strict_text_page_order_finite='PASS'))
        print(case['name'] + ': all original Word coordinate planes PASS', flush=True)
    require(len(rows) == 256 and sha(executable) == native_sha, 'Font spacing native/coverage changed')
    (output / 'results.json').write_bytes((json.dumps(dict(native_sha256=native_sha, cases=rows),
                                                     indent=2) + '\n').encode())
    return rows


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output', type=Path, default=Path('target/windows-font-spacing'))
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable, args.output)
    else:
        print(f'Verified {len(verify())} versioned original Word spacing controls')
