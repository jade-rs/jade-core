"""Assert versioned original Windows Word PDF paper bounds and glyph origins."""
import argparse
import json
import math
from pathlib import Path
from zipfile import ZipFile
import xml.etree.ElementTree as ET

import pymupdf

from check_platform_coordinates import ROOT, checked_file, read_json, require
from check_windows_font_family_case import render, sha
from check_windows_font_family_extended_case import observe_word, require_promoted_word
from check_windows_font_coordinates import require_original_font_programs_registered
from check_windows_mincho_sizes import verify_source
from check_windows_font_spacing import verify_context
from extract_windows_emf import extract
from windows_xps_fonts import extract_xps_fonts

INDEX = Path('tests/word-oracle/windows/paper-geometry-cases.json')
INDEX_SHA256 = '54ba8458377bc99b921019592b79d6631ade9f7bcce29302e3cfede09170eff4'
W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def verify_page(source, case):
    with ZipFile(source) as package:
        page = ET.fromstring(package.read('word/document.xml')).find('.//' + W + 'sectPr/' + W + 'pgSz')
    require(page is not None and int(page.get(W + 'w')) == case['page_width_twips']
            and int(page.get(W + 'h')) == case['page_height_twips'], 'Authored paper dimensions changed')


def page_bounds(path):
    with pymupdf.open(path) as document:
        return [list(page.mediabox) for page in document]


def require_bounds(expected, actual, label):
    require(bool(expected) and len(expected) == len(actual), label + ': PDF paper page count changed')
    for before, after in zip(expected, actual):
        require(len(before) == len(after) == 4
                and all(math.isfinite(v) for v in before + after)
                and before[2] > before[0] and before[3] > before[1]
                and after[2] > after[0] and after[3] > after[1], label + ': invalid PDF paper bounds')
        # PDF stores these paper dimensions as decimal points; float32 readers
        # introduce less than .001pt. Glyph comparison tolerances are separate.
        require(all(abs(a - b) <= 0.001 for a, b in zip(before, after)),
                label + f': original Word PDF paper bounds differ: {before} != {after}')


def verify(root=ROOT):
    require(sha(root / INDEX) == INDEX_SHA256, 'Paper geometry index changed')
    index = read_json(root / INDEX)
    require(index['schema'] == 1 and len(index['fixtures']) == 17, 'Paper geometry coverage removed')
    coverage, sources, cases = set(), set(), []
    for case in index['fixtures']:
        paths = {key: checked_file(root, case[key]) for key in ('source', 'capture', 'pdf', 'xps')}
        capture = read_json(paths['capture'])
        require(capture['word'] == index['word'] and capture['status'] == 'complete'
                and capture['platform'] == 'windows'
                and paths['capture'].parent.parent.name == 'word-' + capture['word']['product_version'],
                'Paper geometry Word identity changed')
        document, = [d for d in capture['documents'] if d['name'] == case['name']]
        require(document['sha256'] == case['source']['sha256']
                and document['open_mode'] == 'normal-read-only'
                and document['compatibility_mode'] == case['compatibility_mode'],
                'Paper geometry original source changed')
        for key in ('pdf', 'xps'):
            require(paths[key] == paths['capture'].parent / document[key]
                    and case[key]['sha256'] == document[key + '_sha256'], 'Paper geometry original output changed')
        require((case['family'], case['style'], case['font_size_pt'], case['compatibility_mode'],
                 case['line_ending'], case['balanced']) == ('Yu Mincho', 'regular', 12, 15, 'paragraph', False),
                'Paper geometry control style changed')
        verify_source(paths['source'], case)
        verify_context(paths['source'], case)
        verify_page(paths['source'], case)
        coverage.add((case['page_width_twips'], case['page_height_twips']))
        sources.add(case['source']['sha256'])
        cases.append((case, paths))
    expected = {(width, 16838) for width in range(11902, 11911)}
    expected |= {(11906, height) for height in range(16834, 16843)}
    require(coverage == expected and len(sources) == 17, 'Paper geometry boundary matrix removed')
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
        require(native['input_sha256'] == case['source']['sha256'], 'Paper geometry rendered wrong source')
        require_original_font_programs_registered(fonts, native)
        painted = target / 'rendered/native.pdf'
        before, after = page_bounds(paths['pdf']), page_bounds(painted)
        require_bounds(before, after, case['name'])
        if paths['capture'] not in emfs:
            emfs[paths['capture']] = {r['name']: r for r in extract(paths['capture'])['fixtures']}
        observed = observe_word(case, paths, native, painted, emfs[paths['capture']][case['name']])
        require_promoted_word(dict(case, required_word_coordinates=True), observed)
        rows.append(dict(name=case['name'], source_sha256=case['source']['sha256'],
                         Word_PDF_paper_bounds=before, native_PDF_paper_bounds=after,
                         original_Word=observed, strict_text_page_order_finite='PASS'))
        print(case['name'] + ': original Word PDF paper and glyph geometry PASS', flush=True)
    require(len(rows) == 17 and sha(executable) == native_sha, 'Paper geometry/native coverage changed')
    (output / 'results.json').write_bytes((json.dumps(dict(native_sha256=native_sha, cases=rows),
                                                     indent=2) + '\n').encode())
    return rows


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output', type=Path, default=Path('target/windows-paper-geometry'))
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable, args.output)
    else:
        print(f'Verified {len(verify())} versioned original Word paper controls')
