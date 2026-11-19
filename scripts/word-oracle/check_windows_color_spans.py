"""Keep Windows Word screen, PDF and XPS assertions independent for color spans."""
import argparse
import hashlib
import json
from pathlib import Path
from zipfile import ZipFile

from check_platform_coordinates import ROOT, checked_file, pdf_origins, read_json, require
from check_windows_aptos_nominal import render_cases
from check_windows_xps import assert_model
from extract_windows_emf import extract
import generate_windows_color_span_probes as producer

INDEX = Path('tests/word-oracle/windows/color-span-observations.json')
INDEX_SHA256 = '81ac37b040147b4213b5333d151935de815abf929bc8549a5f10c31a13d4ac97'
PRINT_NAMES_SHA256 = 'baf39e761451c71b0d8c6c532b4601b5d4fc382b18b83cf7137bb7d5611cd15d'


def locked_promotions(index):
    rows = index['fixtures']
    require(index['schema'] == 1 and len(rows) == len({r['name'] for r in rows}) == 52,
            'Color-span source coverage changed')
    for field, promotion in [('coordinate_gate_promotion', 'color-span-print'),
                             ('xps_gate_promotion', 'color-span-xps')]:
        required = sorted(r['name'] for r in rows if r[field] == promotion)
        require(len(required) == 13 and hashlib.sha256(
            json.dumps(required, separators=(',', ':')).encode()).hexdigest() == PRINT_NAMES_SHA256,
            'Required color-span print assertions changed')
        require(all(r[field] in (promotion, 'pending') for r in rows), 'Unknown color-span promotion')
    require(all(r['screen_gate_promotion'] == 'color-span-screen' for r in rows),
            'Required color-span screen assertion removed')
    return frozenset(r['name'] for r in rows if r['coordinate_gate_promotion'] == 'pending')


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Original color-span registry changed')
    index = read_json(root / INDEX); pending = locked_promotions(index)
    mac = read_json(checked_file(root, index['mac_parent_manifest']))
    require(mac['reference'] == index['mac_parent_reference'] and mac['reference']['platform'] == 'mac',
            'Original Mac reference changed')
    fonts = read_json(checked_file(root, index['font_facts']))
    require(fonts['word'] == index['word'] and len(fonts['fonts']) == 4
            and all(f['version'] == ['Version 2.01;O365'] for f in fonts['fonts']),
            'Original color-span font facts changed')
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
            'Original color-span Word identity changed')
    require(len(capture['documents']) == 52, 'Original color-span capture coverage changed')
    documents = {d['name']: d for d in capture['documents']}
    require(len(documents) == 52, 'Duplicate original color-span document')
    drawings = {d['name']: d for d in extract(manifest)['fixtures']}
    cases, xps_models = [], {}
    with ZipFile(checked_file(root, first['archive'])) as archive:
        require(archive.read('capture.json') == manifest.read_bytes(), 'Original color-span metadata changed')
        for case in index['fixtures']:
            name = case['name']; data, spec = generated[name]; source = checked_file(root, case['source'])
            require(case['capture'] == first['capture'] and case['archive'] == first['archive']
                    and case['word'] == capture['word'] and case['capture_id'] == capture['capture_id'],
                    'Color-span reference substituted')
            require(source.read_bytes() == data == archive.read('inputs/' + name + '.docx')
                    and case['input_specification'] == spec, 'Color-span input not reproducible')
            document = documents[name]
            require(document['sha256'] == case['source']['sha256']
                    and document['open_mode'] == 'normal-read-only' and document['compatibility_mode'] == 14,
                    'Wrong original color-span input/mode')
            for key in ('pdf', 'xps'):
                path = manifest.parent / document[key]
                require(path.read_bytes() == archive.read(document[key])
                        and hashlib.sha256(path.read_bytes()).hexdigest() == document[key + '_sha256'],
                        'Original color-span PDF/XPS changed')
            drawing = drawings[name]
            original = manifest.parent / document['pdf']; xps = manifest.parent / document['xps']
            require(drawing['page_count'] == document['pages'] == 1 and len(drawing['glyphs']) == 280,
                    'Color-span screen page/glyph coverage changed')
            model = pdf_origins(xps, True)
            require(model['page_count'] == 1 and len(model['glyphs']) == case['glyph_count'] == 280,
                    'Color-span XPS page/glyph coverage changed')
            xps_models[name] = model
            cases.append((case, source, original, drawing, pdf_origins(original), pdf_origins(original, True)))
    require(len(cases) == 52, 'Color-span coverage removed')
    return cases, pending, xps_models


def check(executable, output, root=ROOT):
    cases, pending, xps_models = verify(root)
    render_cases(executable, output, cases, pending)
    results = []
    for case, *_ in cases:
        name = case['name']; required = case['xps_gate_promotion'] == 'color-span-xps'
        planes = {plane: assert_model(xps_models[name], read_json(output / name / (plane + '.json')),
                                     name + '/XPS/' + plane, required) for plane in ('native', 'screen')}
        results.append(dict(name=name, required_xps=required, planes=planes))
    (output / 'xps-results.json').write_bytes((json.dumps(results, indent=2) + '\n').encode())
    print('Checked 52 color-span EMF cases,13 required physical PDF cases,13 dual-plane XPS cases; '
          '39 pending PDF/XPS differences retained')
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'target/windows-color-spans')
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable.resolve(), args.output_directory.resolve())
    else:
        print('Verified', len(verify()[0]), 'versioned original Word color-span controls')
