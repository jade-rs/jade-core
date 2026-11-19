"""Assert versioned Word repeated-y lengths independently for screen, PDF and XPS."""
import argparse
import hashlib
import json
from pathlib import Path
from zipfile import ZipFile

import fitz

from check_platform_coordinates import ROOT, checked_file, pdf_origins, read_json, require
from check_windows_aptos_nominal import render_cases
from check_windows_xps import assert_model
from extract_windows_emf import extract
import generate_windows_y_length_probes as producer

INDEX = Path('tests/word-oracle/windows/y-length-observations.json')
INDEX_SHA256 = '3b2a1cc6387b8644dbe6d0216585b0a963c71ef0f4939d824317c25368c26305'
PRINT_NAMES_SHA256 = 'dba72ea8eaea4f943b2fa0c39564231bed3706207ed4f966193beed9ce2c4929'


def locked_promotions(index):
    rows = index['fixtures']
    require(index['schema'] == 1 and len(rows) == len({r['name'] for r in rows}) == 4,
            'Repeated-y source coverage changed')
    required = sorted(r['name'] for r in rows if r['coordinate_gate_promotion'] == 'y-length-print')
    require(len(required) == 2 and hashlib.sha256(
        json.dumps(required, separators=(',', ':')).encode()).hexdigest() == PRINT_NAMES_SHA256,
        'Required repeated-y print assertions changed')
    require(all(r['coordinate_gate_promotion'] in ('y-length-print', 'pending')
                and r['screen_gate_promotion'] == 'y-length-screen'
                and r['xps_gate_promotion'] == 'y-length-xps' for r in rows),
            'Required repeated-y screen/XPS assertion removed')
    return frozenset(r['name'] for r in rows if r['coordinate_gate_promotion'] == 'pending')


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Original repeated-y registry changed')
    index = read_json(root / INDEX)
    pending = locked_promotions(index)
    mac = read_json(checked_file(root, index['mac_parent_manifest']))
    require(mac['reference'] == index['mac_parent_reference'] and mac['reference']['platform'] == 'mac',
            'Original Mac reference changed')
    generated = {name: (data, spec) for name, data, spec in producer.generate(root)}
    for _, spec in generated.values():
        require(hashlib.sha256((root / spec['mac_parent']).read_bytes()).hexdigest()
                == spec['mac_parent_sha256'] == mac['sourceSha256'], 'Original Mac source changed')
    first = index['fixtures'][0]
    manifest = checked_file(root, first['capture'])
    capture = read_json(manifest)
    require(capture['schema'] == 2 and capture['status'] == 'complete'
            and capture['platform'] == 'windows' and capture['word'] == index['word']
            and capture['capture_id'] == index['capture_id']
            and capture['word']['product_version'] == '16.0.19127.20936'
            and capture['word']['executable_sha256'] ==
            '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312',
            'Original repeated-y Word identity changed')
    documents = {d['name']: d for d in capture['documents']}
    require(len(documents) == len(capture['documents']) == 4, 'Repeated-y capture coverage changed')
    drawings = {d['name']: d for d in extract(manifest)['fixtures']}
    fonts = {d['name']: d for d in read_json(checked_file(root, index['font_facts']))}
    require(set(fonts) == set(generated) == set(documents), 'Repeated-y font/source coverage changed')
    cases, xps_models = [], {}
    with ZipFile(checked_file(root, first['archive'])) as archive, \
            ZipFile(checked_file(root, index['original_capture'])) as original:
        require(len(archive.namelist()) == len(set(archive.namelist())) == 17
                and len(original.namelist()) == len(set(original.namelist())) == 21,
                'Original repeated-y archive coverage changed')
        for member in archive.namelist():
            require(archive.read(member) == original.read(member), 'Original capture output substituted')
        require(archive.read('capture.json') == manifest.read_bytes()
                and original.read('word-output-font-facts.json') == checked_file(root, index['font_facts']).read_bytes(),
                'Original repeated-y metadata changed')
        for case in index['fixtures']:
            name = case['name']
            data, spec = generated[name]
            source = checked_file(root, case['source'])
            require(case['capture'] == first['capture'] and case['archive'] == first['archive']
                    and case['word'] == capture['word'] and case['capture_id'] == capture['capture_id'],
                    'Repeated-y reference substituted')
            require(source.read_bytes() == data == archive.read('inputs/' + name + '.docx')
                    and case['input_specification'] == spec, 'Repeated-y input not reproducible')
            document = documents[name]
            require(document['sha256'] == case['source']['sha256']
                    and document['open_mode'] == 'normal-read-only'
                    and document['compatibility_mode'] == spec['compatibility_mode'] == 14,
                    'Wrong original repeated-y input/mode')
            for key in ('pdf', 'xps'):
                path = manifest.parent / document[key]
                require(path.read_bytes() == archive.read(document[key])
                        and hashlib.sha256(path.read_bytes()).hexdigest() == document[key + '_sha256'],
                        'Original repeated-y PDF/XPS changed')
            original_pdf = manifest.parent / document['pdf']
            xps = manifest.parent / document['xps']
            facts = fonts[name]
            require(facts['word'] == capture['word'] and facts['source_sha256'] == document['sha256'],
                    'Original repeated-y font identity changed')
            with fitz.open(original_pdf) as pdf:
                for font in facts['pdf_fonts']:
                    require(hashlib.sha256(pdf.extract_font(font['xref'])[3]).hexdigest()
                            == font['program_sha256'], 'Original PDF font program changed')
                aptos = [f for f in facts['pdf_fonts'] if f['family'] == 'Aptos Display']
                require(len(aptos) == 2 and all(f['version'] == 'Version 2.01;O365' for f in aptos),
                        'Original Aptos version changed')
            with ZipFile(xps) as package:
                for font in facts['xps_font_members']:
                    require(hashlib.sha256(package.read(font['file'])).hexdigest() == font['sha256'],
                            'Original XPS font program changed')
            count = sum(p['length'] for p in spec['paragraphs'])
            drawing = drawings[name]
            xps_model = pdf_origins(xps, True)
            require(drawing['page_count'] == xps_model['page_count'] == document['pages'] == 1
                    and len(drawing['glyphs']) == len(xps_model['glyphs']) == case['glyph_count'] == count,
                    'Repeated-y page/glyph coverage changed')
            cases.append((case, source, original_pdf, drawing, pdf_origins(original_pdf),
                          pdf_origins(original_pdf, True)))
            xps_models[name] = xps_model
    require(len(cases) == 4, 'Repeated-y source coverage removed')
    return cases, pending, xps_models


def check(executable, output, root=ROOT):
    cases, pending, xps_models = verify(root)
    render_cases(executable, output, cases, pending)
    results = []
    for case, *_ in cases:
        name = case['name']
        planes = {plane: assert_model(xps_models[name], read_json(output / name / (plane + '.json')),
                                     name + '/XPS/' + plane, True) for plane in ('native', 'screen')}
        results.append(dict(name=name, required_pdf=name not in pending, required_xps=True, planes=planes))
    (output / 'xps-results.json').write_bytes((json.dumps(results, indent=2) + '\n').encode())
    print('Checked 4 repeated-y EMF cases /2 required physical PDF cases /4 dual-plane XPS cases; '
          '2 pending PDF coordinate differences retain strict text/page/order/finite assertions')
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'target/windows-y-lengths')
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable.resolve(), args.output_directory.resolve())
    else:
        print('Verified', len(verify()[0]), 'versioned original Word repeated-y controls')
