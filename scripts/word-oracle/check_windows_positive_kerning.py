"""Assert independently versioned Word positive-kern controls without rewriting goldens."""
import argparse
import hashlib
import json
from pathlib import Path
from zipfile import ZipFile
from check_platform_coordinates import ROOT, checked_file, read_json, require, pdf_origins
from check_windows_aptos_nominal import render_cases
from extract_windows_emf import extract
import generate_windows_positive_kerning_probes as producer

INDEX = Path('tests/word-oracle/windows/positive-kerning-observations.json')
INDEX_SHA256 = '4b71a6e8935246b8f1c19215b2fcda108c80c23481e3dedd6c88be49f7826f46'
REQUIRED_PDF_NAMES_SHA256 = 'ef342e71299955a7461cb63f9c1f27f1decff7c9dfa165836237808790d787bc'


def locked_promotions(index):
    rows = index['fixtures']
    require(index['schema'] == 1 and len(rows) == 192
            and len({r['name'] for r in rows}) == 192, 'Positive-kern coverage removed')
    required = sorted(r['name'] for r in rows if r['coordinate_gate_promotion'] == 'aptos-positive-kerning')
    digest = hashlib.sha256(json.dumps(required, separators=(',', ':')).encode()).hexdigest()
    require(len(required) == 92 and digest == REQUIRED_PDF_NAMES_SHA256,
            'Validated positive-kern PDF assertions changed')
    for row in rows:
        require(row['screen_gate_promotion'] == 'aptos-positive-screen', 'Required positive-kern screen assertion removed')
        require(row['coordinate_gate_promotion'] in ('aptos-positive-kerning', 'pending'),
                'Unknown positive-kern assertion status')
    return frozenset(r['name'] for r in rows if r['name'] not in required)


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Positive-kern original reference coverage changed')
    index = read_json(root / INDEX); pending = locked_promotions(index)
    mac = read_json(checked_file(root, index['mac_parent_manifest']))
    require(mac['reference'] == index['mac_parent_reference']
            and mac['reference']['platform'] == 'mac'
            and mac['sourceSha256'] == producer.parentgen.PARENT_SHA256, 'Original Mac reference changed')
    generated = {name: (data, spec) for name, data, spec in producer.generate(root)}
    require(len(generated) == 192 and set(generated) == {r['name'] for r in index['fixtures']},
            'Positive-kern generated source coverage changed')
    captures = {}; drawings = {}; cases = []
    for row in index['fixtures']:
        name = row['name']; source = checked_file(root, row['source'])
        require((source.read_bytes(), row['input_specification']) == generated[name],
                'Positive-kern captured source/specification changed')
        path = checked_file(root, row['capture'])
        if path not in captures:
            captures[path] = read_json(path)
            drawings[path] = {r['name']: r for r in extract(path)['fixtures']}
        capture = captures[path]
        require(capture['status'] == 'complete' and capture['platform'] == 'windows'
                and capture['word'] == row['word'] and capture['capture_id'] == path.parent.name
                and path.parent.parent.name == 'word-' + row['word']['product_version']
                and capture['print_environment'] == index['print_environment'],
                'Original positive-kern Word version/capture changed')
        doc, = [d for d in capture['documents'] if d['name'] == name]
        require(doc['sha256'] == row['source']['sha256'] and doc['open_mode'] == 'normal-read-only'
                and doc['compatibility_mode'] == row['input_specification']['compatibility_mode'],
                'Original positive-kern input/mode changed')
        with ZipFile(checked_file(root, row['archive'])) as archive:
            require(archive.read('capture.json') == path.read_bytes()
                    and archive.read('inputs/' + name + '.docx') == source.read_bytes(),
                    'Original positive-kern archive changed')
            for filename, digest in [(doc['pdf'], doc['pdf_sha256'])] + [
                    (p['file'], p['sha256']) for p in doc['metafiles']]:
                data = (path.parent / filename).read_bytes()
                require(hashlib.sha256(data).hexdigest() == digest and data == archive.read(filename),
                        'Original positive-kern output changed')
        drawing = drawings[path][name]
        require(drawing['page_count'] == doc['pages'] == 1 and len(drawing['glyphs']) == 192,
                'Positive-kern page/glyph coverage removed')
        original = path.parent / doc['pdf']
        cases.append((row, source, original, drawing, pdf_origins(original), pdf_origins(original, True)))
    require(sum(len(c[3]['glyphs']) for c in cases) == 36864 and len(pending) == 100,
            'Positive-kern screen/PDF coverage removed')
    return cases, pending


def check(executable, output, root=ROOT):
    cases, pending = verify(root)
    return render_cases(executable, output, cases, pending)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'target/windows-positive-kerning')
    args = parser.parse_args()
    if args.native_executable:
        print('Checked', check(args.native_executable.resolve(), args.output_directory.resolve()),
              'positive-kern screen cases /92 required PDF cases;100 PDF differences retained')
    else:
        print('Verified', len(verify()[0]), 'versioned original positive-kern controls')
