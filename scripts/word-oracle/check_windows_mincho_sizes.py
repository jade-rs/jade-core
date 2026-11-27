"""Assert all original Windows Word Mincho size controls, without pending exemptions."""
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

INDEX = Path('tests/word-oracle/windows/mincho-size-cases.json')
INDEX_SHA256 = 'bc6d00880efcc88cccce0bc13a23aec6914c337886361b4ab35833c528b92c4b'
W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def verify_source(source, case):
    with ZipFile(source) as package:
        document = ET.fromstring(package.read('word/document.xml'))
        settings = ET.fromstring(package.read('word/settings.xml'))
    runs = [r for r in document.iter(W + 'r')
            if r.find(W + 'rPr/' + W + 'rFonts') is not None]
    require(len(runs) == 2, 'Mincho authored run coverage changed')
    for run in runs:
        properties = run.find(W + 'rPr')
        fonts = properties.find(W + 'rFonts')
        require(fonts.get(W + 'ascii') == fonts.get(W + 'hAnsi') == case['family'],
                'Mincho authored family changed')
        size = properties.find(W + 'sz')
        require(size is not None and size.get(W + 'val') == str(case['font_size_pt'] * 2),
                'Mincho authored size changed')
        for flag, enabled in [('b', 'bold' in case['style']), ('i', 'italic' in case['style'])]:
            node = properties.find(W + flag)
            actual = node is not None and node.get(W + 'val', '1') not in ('0', 'false', 'off')
            require(actual == enabled, 'Mincho authored style changed')
    modes = [n.get(W + 'val') for n in settings.iter(W + 'compatSetting')
             if n.get(W + 'name') == 'compatibilityMode']
    require(modes == [str(case['compatibility_mode'])], 'Mincho authored compatibility changed')


def verify(root=ROOT):
    require(sha(root / INDEX) == INDEX_SHA256, 'Mincho size index changed')
    index = read_json(root / INDEX)
    require(index['schema'] == 1 and len(index['fixtures']) == 96, 'Mincho size coverage removed')
    coverage, sources, cases = set(), set(), []
    for case in index['fixtures']:
        paths = {key: checked_file(root, case[key]) for key in ('source', 'capture', 'pdf', 'xps')}
        capture = read_json(paths['capture'])
        require(capture['word'] == index['word'] and capture['status'] == 'complete'
                and capture['platform'] == 'windows', 'Mincho Word identity changed')
        document, = [d for d in capture['documents'] if d['name'] == case['name']]
        require(document['sha256'] == case['source']['sha256']
                and document['open_mode'] == 'normal-read-only'
                and document['compatibility_mode'] == case['compatibility_mode'],
                'Mincho original source/open mode changed')
        for key in ('pdf', 'xps'):
            require(paths[key] == paths['capture'].parent / document[key]
                    and case[key]['sha256'] == document[key + '_sha256'],
                    'Mincho original output changed')
        verify_source(paths['source'], case)
        sources.add(case['source']['sha256'])
        coverage.add((case['family'], case['style'], case['font_size_pt'], case['compatibility_mode']))
        cases.append((case, paths))
    expected = {(family, style, size, mode)
                for family in ('Yu Mincho', 'MS Mincho')
                for style in ('regular', 'bold', 'italic', 'bolditalic')
                for size in (8, 10, 14, 16, 19, 24) for mode in (14, 15)}
    require(coverage == expected and len(sources) == 96, 'Mincho size/style matrix removed')
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
        require(native['input_sha256'] == case['source']['sha256'], 'Mincho rendered wrong input')
        require_original_font_programs_registered(fonts, native)
        if paths['capture'] not in emfs:
            emfs[paths['capture']] = {c['name']: c for c in extract(paths['capture'])['fixtures']}
        observed = observe_word(case, paths, native, target / 'rendered/native.pdf',
                                emfs[paths['capture']][case['name']])
        require_promoted_word(dict(case, required_word_coordinates=True), observed)
        rows.append(dict(name=case['name'], source_sha256=case['source']['sha256'],
                         original_Word=observed, strict_text_page_order_finite='PASS'))
        print(case['name'] + ': all original Word coordinate planes PASS', flush=True)
    require(len(rows) == 96 and sha(executable) == native_sha, 'Mincho native/coverage changed')
    (output / 'results.json').write_bytes((json.dumps(dict(native_sha256=native_sha, cases=rows),
                                                     indent=2) + '\n').encode())
    return rows


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output', type=Path, default=Path('target/windows-mincho-sizes'))
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable, args.output)
    else:
        print(f'Verified {len(verify())} versioned original Word Mincho size controls')
