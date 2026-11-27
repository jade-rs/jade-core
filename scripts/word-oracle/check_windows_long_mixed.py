"""Assert original Windows Word mixed long runs and retain PDF holdouts."""
import argparse
import json
from pathlib import Path
from zipfile import ZipFile
import xml.etree.ElementTree as ET

from check_platform_coordinates import ROOT, checked_file, read_json, require
from check_windows_font_family_case import render, sha
from check_windows_font_family_extended_case import observe_word
from check_windows_font_coordinates import has_visible_vector_ink, require_original_font_programs_registered
from extract_windows_emf import extract
from windows_xps_fonts import extract_xps_fonts

INDEX = Path('tests/word-oracle/windows/long-mixed-cases.json')
INDEX_SHA256 = '962d5e5491bd8b49fc3f3e001bbaa9799d7207fbb4c29dac94642f7838ca10d8'
W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
SCREEN_PLANES = {'EMF', 'XPS-native', 'XPS-visible'}
PDF_PLANES = {'PDF-unclipped', 'PDF-visible', 'painted-PDF'}


def verify_source(source, case):
    with ZipFile(source) as package:
        document = ET.fromstring(package.read('word/document.xml'))
        settings = ET.fromstring(package.read('word/settings.xml'))
    paragraphs = document.findall(W + 'body/' + W + 'p')
    require(not list(document.iter(W + 'shd')), 'Mixed long-run unexpected authored shading')
    require(len(paragraphs) == 4, 'Mixed long-run paragraph coverage changed')
    for paragraph, length in zip(paragraphs, (24, 48, 72, 96)):
        runs = paragraph.findall(W + 'r')
        require(len(runs) == 1, 'Mixed long-run authored runs changed')
        run = runs[0]
        expected = (case['pattern'] * length)[:length]
        require(''.join(t.text or '' for t in run.iter(W + 't')) == expected,
                'Mixed long-run authored text changed')
        properties = run.find(W + 'rPr')
        require(properties is not None, 'Mixed long-run run properties missing')
        def value(name):
            node = properties.find(W + name)
            return None if node is None else node.get(W + 'val')
        family = properties.find(W + 'rFonts')
        require(family is not None and family.get(W + 'ascii') == 'Aptos Display'
                and family.get(W + 'hAnsi') == 'Aptos Display'
                and value('sz') == str(case['font_size_pt'] * 2)
                and value('b') == ('1' if case['style'] == 'bold' else '0')
                and value('i') == '0' and value('kern') == '200',
                'Mixed long-run authored font/kerning changed')
        require(paragraph.find(W + 'pPr/' + W + 'jc') is None,
                'Mixed long-run paragraph alignment changed')
    page = document.find('.//' + W + 'sectPr/' + W + 'pgSz')
    require(page is not None and page.get(W + 'w') == '31680'
            and page.get(W + 'h') == '16838', 'Mixed long-run authored paper changed')
    modes = [n.get(W + 'val') for n in settings.iter(W + 'compatSetting')
             if n.get(W + 'name') == 'compatibilityMode']
    require(modes == ['14'], 'Mixed long-run authored compatibility changed')


def require_origins(case, observed):
    require(case['font_size_pt'] in (17, 19), 'Mixed long-run point-size profile changed')
    require(set(observed['planes']) == SCREEN_PLANES | PDF_PLANES,
            'Mixed long-run output coverage changed')
    required = SCREEN_PLANES | (PDF_PLANES if case['font_size_pt'] == 17 else set())
    for plane in sorted(required):
        require(observed['planes'][plane] == 'PASS',
                case['name'] + '/required ' + plane + ': ' + observed['planes'][plane])
    require(observed['paragraph_background'] == 'PASS',
            case['name'] + ': original paragraph background changed')
    return sorted(required)


def verify(root=ROOT):
    require(sha(root / INDEX) == INDEX_SHA256, 'Mixed long-run original index changed')
    index = read_json(root / INDEX)
    require(index['schema'] == 1 and len(index['fixtures']) == 8,
            'Mixed long-run original coverage removed')
    coverage, sources, cases = set(), set(), []
    for case in index['fixtures']:
        require((case['style'], case['font_size_pt'], case['pattern'], case['compatibility_mode'])
                in {(s, z, p, 14) for s in ('regular', 'bold') for z in (17, 19)
                    for p in ('@m', '@@@@mmmm')}, 'Mixed long-run control profile changed')
        paths = {key: checked_file(root, case[key]) for key in ('source', 'capture', 'pdf', 'xps')}
        capture = read_json(paths['capture'])
        require(capture['word'] == index['word'] and capture['status'] == 'complete'
                and capture['platform'] == 'windows'
                and paths['capture'].parent.parent.name == 'word-' + capture['word']['product_version'],
                'Mixed long-run original Word identity changed')
        document, = [d for d in capture['documents'] if d['name'] == case['name']]
        require(document['sha256'] == case['source']['sha256']
                and document['compatibility_mode'] == 14 and document['open_mode'] == 'normal-read-only',
                'Mixed long-run original input changed')
        for key in ('pdf', 'xps'):
            require(paths[key] == paths['capture'].parent / document[key]
                    and case[key]['sha256'] == document[key + '_sha256'],
                    'Mixed long-run original output changed')
        verify_source(paths['source'], case)
        coverage.add((case['style'], case['font_size_pt'], case['pattern']))
        sources.add(case['source']['sha256'])
        cases.append((case, paths))
    require(len(coverage) == len(sources) == 8, 'Mixed long-run boundary matrix removed')
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
        require(native['input_sha256'] == case['source']['sha256'], 'Mixed long-run rendered wrong source')
        require_original_font_programs_registered(fonts, native)
        if paths['capture'] not in emfs:
            emfs[paths['capture']] = {r['name']: r for r in extract(paths['capture'])['fixtures']}
        observed = observe_word(case, paths, native, target / 'rendered/native.pdf',
                                emfs[paths['capture']][case['name']])
        # These controls author no shading; the background helper used by the
        # shaded-paragraph gates requires a nonempty authored background.
        require(not has_visible_vector_ink(paths['pdf'])
                and not has_visible_vector_ink(target / 'rendered/native.pdf'),
                case['name'] + ': unexpected original/native background paint')
        observed['paragraph_background'] = 'PASS'
        required = require_origins(case, observed)
        rows.append(dict(name=case['name'], source_sha256=case['source']['sha256'],
                         original_Word=observed, required_planes=required,
                         strict_text_page_order_finite='PASS'))
        print(case['name'] + ': required original Word origins PASS; PDF '
              + ('required' if case['font_size_pt'] == 17 else 'observation retained'), flush=True)
    require(len(rows) == 8 and sha(executable) == native_sha, 'Mixed long-run native/coverage changed')
    (output / 'results.json').write_bytes((json.dumps(dict(native_sha256=native_sha, cases=rows),
                                                     indent=2) + '\n').encode())
    return rows


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output', type=Path, default=Path('target/windows-long-mixed'))
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable, args.output)
    else:
        print(f'Verified {len(verify())} versioned original Word mixed long runs')
