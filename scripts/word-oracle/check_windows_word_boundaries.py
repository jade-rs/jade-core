"""Keep Windows Word screen, PDF and XPS assertions independent for word boundaries."""
import argparse
import hashlib
import json
import math
from pathlib import Path
from zipfile import ZipFile

from check_platform_coordinates import ROOT, checked_file, pdf_origins, read_json, require
from check_windows_aptos_nominal import render_cases
from check_windows_xps import assert_model
from extract_windows_emf import extract
import generate_windows_word_boundary_probes as producer

INDEX = Path('tests/word-oracle/windows/word-boundary-observations.json')
INDEX_SHA256 = 'cda0b6bd6c5eb84b0f85bb1d1ad45acd4955a71a95bbdea2639c0bab92b5b8ef'
PRINT_NAMES_SHA256 = '5c8cb736ed3c6f1ef11c28dc0fdc9e4192dde8312282699ccf6f3a6f936223a1'


def locked_promotions(index):
    rows = index['fixtures']
    require(index['schema'] == 1 and len(rows) == len({r['name'] for r in rows}) == 16,
            'Word-boundary source coverage changed')
    required = sorted(r['name'] for r in rows
                      if r['coordinate_gate_promotion'] == 'word-boundary-print')
    require(len(required) == 4 and hashlib.sha256(
        json.dumps(required, separators=(',', ':')).encode()).hexdigest() == PRINT_NAMES_SHA256,
        'Required word-boundary print assertions changed')
    require(all(r['coordinate_gate_promotion'] in ('word-boundary-print', 'pending') for r in rows),
            'Unknown word-boundary promotion')
    require(all(r['screen_gate_promotion'] == 'word-boundary-screen'
                and r['xps_gate_promotion'] == 'word-boundary-xps' for r in rows),
            'Required word-boundary screen/XPS assertion removed')
    return frozenset(r['name'] for r in rows if r['coordinate_gate_promotion'] == 'pending')


def assert_pending_pdf_text(expected, actual, label):
    require(expected['page_count'] == actual['page_count'], label + ': page count changed')
    require(all(math.isfinite(g[axis]) for g in actual['glyphs'] for axis in (2, 3)),
            label + ': nonfinite glyph origin')
    words = [g[:2] for g in expected['glyphs'] if not g[1].isspace()]
    painted = [g[:2] for g in actual['glyphs'] if not g[1].isspace()]
    require(words == painted and bool(words), label + ': text/page/order changed')


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Original word-boundary registry changed')
    index = read_json(root / INDEX); pending = locked_promotions(index)
    mac = read_json(checked_file(root, index['mac_parent_manifest']))
    require(mac['reference'] == index['mac_parent_reference'] and mac['reference']['platform'] == 'mac',
            'Original Mac reference changed')
    fonts = read_json(checked_file(root, index['font_facts']))
    require(fonts['word'] == index['word'] and len(fonts['fonts']) == 4
            and all(f['version'] == ['Version 2.01;O365'] for f in fonts['fonts']),
            'Original word-boundary font facts changed')
    generated = {name: (data, spec) for name, data, spec in producer.generate(root)}
    for _, spec in generated.values():
        require(hashlib.sha256((root / spec['mac_parent']).read_bytes()).hexdigest()
                == spec['mac_parent_sha256'] == mac['sourceSha256'], 'Original Mac source changed')
    first = index['fixtures'][0]; manifest = checked_file(root, first['capture']); capture = read_json(manifest)
    require(capture['status'] == 'complete' and capture['platform'] == 'windows'
            and capture['word'] == index['word'] and capture['capture_id'] == index['capture_id']
            and capture['word']['product_version'] == '16.0.19127.20936'
            and capture['word']['executable_sha256'] ==
            '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312',
            'Original word-boundary Word identity changed')
    require(len(capture['documents']) == 16, 'Original word-boundary capture coverage changed')
    documents = {d['name']: d for d in capture['documents']}
    require(len(documents) == 16, 'Duplicate original word-boundary document')
    drawings = {d['name']: d for d in extract(manifest)['fixtures']}
    cases, xps_models = [], {}
    with ZipFile(checked_file(root, first['archive'])) as archive:
        require(archive.read('capture.json') == manifest.read_bytes(), 'Original word-boundary metadata changed')
        for case in index['fixtures']:
            name = case['name']; data, spec = generated[name]; source = checked_file(root, case['source'])
            require(case['capture'] == first['capture'] and case['archive'] == first['archive']
                    and case['word'] == capture['word'] and case['capture_id'] == capture['capture_id'],
                    'Word-boundary reference substituted')
            require(source.read_bytes() == data == archive.read('inputs/' + name + '.docx')
                    and case['input_specification'] == spec, 'Word-boundary input not reproducible')
            document = documents[name]
            require(document['sha256'] == case['source']['sha256']
                    and document['open_mode'] == 'normal-read-only' and document['compatibility_mode'] == spec['compatibility_mode'],
                    'Wrong original word-boundary input/mode')
            for key in ('pdf', 'xps'):
                path = manifest.parent / document[key]
                require(path.read_bytes() == archive.read(document[key])
                        and hashlib.sha256(path.read_bytes()).hexdigest() == document[key + '_sha256'],
                        'Original word-boundary PDF/XPS changed')
            drawing = drawings[name]
            original = manifest.parent / document['pdf']; xps = manifest.parent / document['xps']
            require(drawing['page_count'] == document['pages'] == 1 and len(drawing['glyphs']) == 960,
                    'Word-boundary screen page/glyph coverage changed')
            model = pdf_origins(xps, True)
            require(model['page_count'] == 1 and len(model['glyphs']) == case['glyph_count'] == 960,
                    'Word-boundary XPS page/glyph coverage changed')
            xps_models[name] = model
            cases.append((case, source, original, drawing, pdf_origins(original), pdf_origins(original, True)))
    require(len(cases) == 16, 'Word-boundary coverage removed')
    return cases, pending, xps_models


def check(executable, output, root=ROOT):
    cases, pending, xps_models = verify(root)
    render_cases(executable, output, cases, pending)
    results = []
    for case, _, original, *_ in cases:
        name = case['name']; required = True
        for unclipped in (False, True):
            assert_pending_pdf_text(pdf_origins(original, unclipped),
                pdf_origins(output / name / 'native.pdf', unclipped), name + '/PDF')
        planes = {plane: assert_model(xps_models[name], read_json(output / name / (plane + '.json')),
                                     name + '/XPS/' + plane, required) for plane in ('native', 'screen')}
        results.append(dict(name=name, required_xps=required, planes=planes))
    (output / 'xps-results.json').write_bytes((json.dumps(results, indent=2) + '\n').encode())
    print('Checked 16 word-boundary EMF cases,4 required physical PDF cases,16 dual-plane XPS cases; '
          '12 pending PDF coordinate differences retain strict text/page/order assertions')
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'target/windows-word-boundaries')
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable.resolve(), args.output_directory.resolve())
    else:
        print('Verified', len(verify()[0]), 'versioned original Word word-boundary controls')
