"""Assert28 original Word screen/PDF cases with immutable source and font identity."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
from zipfile import ZipFile
from extract_windows_emf import extract

import pymupdf
from check_platform_coordinates import (ROOT, checked_file, compare, extract_fonts,
                                        pdf_origins, read_json, require)
from check_windows_font_coordinates import require_original_font_programs_registered
from check_windows_observations import INDEX, INDEX_SHA256

METADATA = 'tests/word-oracle/windows/top-bottom-observations.json'
SEGMENT_METADATA = 'tests/word-oracle/windows/pdf-segment-observations.json'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()


def verify(root=ROOT):
    require(sha(root / INDEX) == INDEX_SHA256, 'Original observation coverage changed')
    index = read_json(root / INDEX)
    datasets = []
    for relative, count in ((METADATA, 24), (SEGMENT_METADATA, 4)):
        fact, = [r for r in index['font_facts'] if r['file'] == relative]
        metadata = read_json(checked_file(root, fact))
        require(len(metadata['fixtures']) == count, 'Original source coverage changed')
        datasets.append(metadata)
    parent = read_json(checked_file(root, datasets[0]['mac_parent_manifest']))
    require(parent['reference'] == datasets[0]['mac_parent_reference']
            and parent['reference']['platform'] == 'mac', 'Mac original provenance changed')
    require(datasets[1]['mac_parent_sha256'] == parent['sourceSha256'],
            'Segment controls lost their Mac parent')
    captures = {}; cases = []; identities = set()
    for metadata in datasets:
        for row in metadata['fixtures']:
            require(row['coordinate_gate_promotion'] == 'required'
                    and row['physical_pdf_gate'] == 'required'
                    and row['output_context'] == 'independent-screen-print',
                    'Required screen or physical PDF gate removed')
            source = checked_file(root, row['source'])
            require(row['input_specification']['source_parent_sha256'] == parent['sourceSha256'],
                    'Source lost its unchanged Mac parent')
            require(row['source']['sha256'] not in identities, 'Duplicate source')
            identities.add(row['source']['sha256'])
            manifest = checked_file(root, row['capture'])
            archive = checked_file(root, row['archive'])
            if manifest not in captures:
                capture = read_json(manifest)
                require(capture['schema'] == 2 and capture['status'] == 'complete'
                        and capture['platform'] == 'windows', 'Incomplete original Word capture')
                require(manifest.parent.parent.name == 'word-' + capture['word']['product_version'],
                        'Wrong Word version directory')
                with ZipFile(archive) as bundle:
                    require(bundle.read('capture.json') == manifest.read_bytes(),
                            'Capture differs from archive')
                    for doc in capture['documents']:
                        require(hashlib.sha256(bundle.read('inputs/' + doc['name'] + '.docx')).hexdigest()
                                == doc['sha256'], 'Archived original input changed')
                        require(hashlib.sha256(bundle.read(doc['pdf'])).hexdigest()
                                == doc['pdf_sha256'], 'Archived original Word PDF changed')
                        for page in doc['metafiles']:
                            require(hashlib.sha256(bundle.read(page['file'])).hexdigest()
                                    == page['sha256'], 'Archived original Word EMF changed')
                captures[manifest] = capture
            capture = captures[manifest]
            require(capture['word'] == row['word'] and capture['capture_id'] == row['capture_id'],
                    'Word version or executable identity changed')
            document, = [d for d in capture['documents'] if d['name'] == row['name']]
            require(document['open_mode'] == 'normal-read-only'
                    and document['sha256'] == row['source']['sha256'], 'Original input identity changed')
            pdf = manifest.parent / document['pdf']
            require(sha(pdf) == document['pdf_sha256'], 'Original Word PDF changed')
            for page in document['metafiles']:
                require(sha(manifest.parent / page['file']) == page['sha256'],
                        'Original Word EMF changed')
            with pymupdf.open(pdf) as original:
                programs = {hashlib.sha256(original.extract_font(xref)[3]).hexdigest()
                            for xref in {f[0] for page in original for f in page.get_fonts(full=True)}
                            if original.extract_font(xref)[3]}
            require(programs == {f['program_sha256'] for f in row['original_word_font_programs']},
                    'Original Word font programs changed')
            cases.append((row, source, pdf, pdf_origins(pdf)))
    require(len(cases) == len(identities) == 28 and len(captures) == 2,
            'Required original geometry coverage changed')
    return cases


def check_windows(executable, output, root=ROOT):
    cases = verify(root); output.mkdir(parents=True, exist_ok=True)
    isolation = output / 'isolated-localappdata'; isolation.mkdir(exist_ok=True)
    environment = dict(os.environ, LOCALAPPDATA=str(isolation.resolve()))
    environment.pop('JADE_OFFICE_FONT_DIR', None)
    rows = []; failures = []; drawing_references = {}
    for row, source, original, expected in cases:
        target = output / row['name']; target.mkdir(exist_ok=False)
        fonts = target / 'fonts'; extract_fonts(original, fonts)
        native = target / 'native.json'; screen = target / 'screen.json'; pdf = target / 'native.pdf'
        for destination, extra in ((native, [str(pdf)]), (screen, [])):
            subprocess.run([str(executable), str(source), str(destination), 'windows', *extra,
                            '--fonts-directory', str(fonts)], check=True, env=environment,
                           stdout=subprocess.DEVNULL)
        a, b = read_json(native), read_json(screen)
        for model in (a, b):
            require(model['input_sha256'] == row['source']['sha256'], 'Wrong rendered input')
            require_original_font_programs_registered(fonts, model)
        require(a['glyphs'] == b['glyphs'] and a['page_count'] == b['page_count'],
                row['name'] + ': export changed live origins')
        require(native.with_suffix('.png').read_bytes() == screen.with_suffix('.png').read_bytes(),
                row['name'] + ': export changed live pixels')
        require(pdf.read_bytes() != original.read_bytes(), 'Word PDF self-comparison')
        capture_path = checked_file(root, row['capture'])
        if capture_path not in drawing_references:
            drawing_references[capture_path] = {
                d['name']: d for d in extract(capture_path)['fixtures']}
        drawing = drawing_references[capture_path][row['name']]
        result = 'PASS'; planes = {}
        for label, reference, actual in (
                ('screen-EMF', drawing, b), ('native-layout-EMF', drawing, a),
                ('physical-PDF', expected, pdf_origins(pdf))):
            try:
                compare(reference, actual, row['name'] + '/' + label)
                planes[label] = 'PASS'
            except ValueError as error:
                planes[label] = str(error); result = 'FAIL'
                failures.append(dict(name=row['name'], plane=label, error=str(error)))
        rows.append(dict(name=row['name'], source_sha256=row['source']['sha256'],
                         word=row['word'], capture_id=row['capture_id'], result=result, planes=planes,
                         word_pdf_sha256=sha(original), native_pdf_sha256=sha(pdf),
                         live_origins_and_pixels_preserved=True))
        (output / 'results.json').write_bytes((json.dumps(dict(
            executable_sha256=sha(executable), verifier_sha256=sha(Path(__file__)),
            reference_index_sha256=INDEX_SHA256,
            metadata_sha256={p: sha(root / p) for p in (METADATA, SEGMENT_METADATA)},
            cases=rows, failures=failures,
            scope='All28 original Word cases required on independent screen/post-export/PDF comparisons. Original font identity, live glyphs/PNG and fixed0.375pt per-axis tolerance retained.'), indent=2) + '\n').encode())
        print(row['name'], 'original Word screen/PDF geometry:', planes, flush=True)
    require(len(rows) == 28 and not failures, 'Arial/top-bottom geometry differs from original Word')
    print('All28 original Word Arial/top-bottom cases PASS on independent screen/post-export/PDF comparisons', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'target/windows-top-bottom-clearance')
    args = parser.parse_args()
    if args.native_executable:
        check_windows(args.native_executable.resolve(), args.output_directory.resolve())
    else:
        print('Verified', len(verify()), 'required original Word screen/PDF cases')
