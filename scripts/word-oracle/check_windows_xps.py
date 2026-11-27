"""Assert Windows Word's original XPS origins independently of its PDF output."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import sys
from zipfile import ZipFile

from check_platform_coordinates import ROOT, checked_file, compare, read_json, require

INDEX = Path('tests/word-oracle/windows/xps-print-observations.json')
INDEX_SHA256 = '401518e4871b5d0b80aed9da631750691bb1807e96b66d2e2af603ac7f7afab9'
REQUIRED_NAMES_SHA256 = '490e01a1f077ed9f4882a483cf47ceebe44b6d8d2b79e5806a6de3df2aad3a5b'


def locked_promotions(index):
    rows = index['fixtures']
    require(index['schema'] == 1 and len(rows) == len({r['name'] for r in rows}) == 296,
            'XPS fixture coverage changed')
    require(Counter(r['group'] for r in rows) == dict(size64=64, nominal40=40, positive192=192),
            'XPS input matrix changed')
    required = sorted(r['name'] for r in rows if r['coordinate_gate_promotion'] == 'windows-xps-print')
    require(len(required) == 283 and hashlib.sha256(
        json.dumps(required, separators=(',', ':')).encode()).hexdigest() == REQUIRED_NAMES_SHA256,
        'Required XPS coordinate assertions changed')
    require(all(r['coordinate_gate_promotion'] in ('windows-xps-print', 'pending') for r in rows),
            'Unknown XPS assertion status')
    require(sum(r['glyph_count'] for r in rows) == 62464, 'XPS glyph coverage changed')
    return frozenset(required)


def xps_origins(raw):
    import pymupdf
    with pymupdf.open(stream=raw, filetype='xps') as document:
        glyphs = []
        for number, page in enumerate(document, 1):
            for block in page.get_text('rawdict', flags=pymupdf.TEXTFLAGS_RAWDICT & ~pymupdf.TEXT_CLIP)['blocks']:
                for line in block.get('lines', []):
                    for span in line['spans']:
                        glyphs.extend([number, char['c'], *char['origin']]
                                      for char in span['chars'] if not char['c'].isspace())
        return dict(page_count=len(document), glyphs=glyphs)


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Original XPS registry changed')
    index = read_json(root / INDEX); required = locked_promotions(index)
    parent = read_json(checked_file(root, index['mac_parent_manifest']))
    require(parent['reference'] == index['mac_parent_reference']
            and parent['reference']['platform'] == 'mac'
            and parent['sourceSha256'] == index['mac_parent']['sha256'],
            'Original Mac source/reference changed')
    checked_file(root, index['mac_parent'])
    archive = checked_file(root, index['archive']); cases = []
    with ZipFile(archive) as bundle:
        require(len(bundle.namelist()) == len(set(bundle.namelist())) == 596,
                'Original XPS archive member coverage changed')
        metadata = bundle.read('capture.json')
        require(hashlib.sha256(metadata).hexdigest() == index['capture_sha256'],
                'Original XPS capture metadata changed')
        capture = json.loads(metadata.decode('utf-8-sig'))
        require(capture['status'] == 'complete' and capture['platform'] == 'windows'
                and capture['capture_id'] == index['capture_id'] and capture['word'] == index['word']
                and capture['word']['product_version'] == '16.0.19127.20936'
                and capture['word']['executable_sha256'] ==
                '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312',
                'Recorded Windows Word version/executable/capture changed')
        require(len(capture['documents']) == 296, 'Original XPS capture coverage changed')
        font_facts = bundle.read(index['original_font_facts']['file'])
        require(hashlib.sha256(font_facts).hexdigest() == index['original_font_facts']['sha256']
                and json.loads(font_facts.decode('utf-8')) == index['original_fonts'],
                'Original Office font facts changed')
        require(len(index['original_fonts']) == 4 and {f['style'] for f in index['original_fonts']}
                == {'bold', 'bolditalic', 'italic', 'regular'}, 'Original Office font coverage changed')
        for font in index['original_fonts']:
            require(font['version'] == ['Version 2.01;O365'] and len(font['sha256']) == 64,
                    'Original Office font version changed')
        documents = {d['name']: d for d in capture['documents']}
        require(len(documents) == 296, 'Duplicate original XPS document')
        for row in index['fixtures']:
            name = row['name']; source = checked_file(root, row['source'])
            checked_file(root, row['word_pdf'])
            require(bundle.read('inputs/' + name + '.docx') == source.read_bytes(),
                    'XPS input differs from immutable original DOCX')
            doc = documents.get(name); spec = row['input_specification']
            require(doc is not None and doc['source_sha256'] == row['source']['sha256']
                    and doc['open_mode'] == 'normal-read-only'
                    and doc['compatibility_mode'] == spec['compatibility_mode'],
                    'Wrong original XPS input or compatibility mode')
            output, = doc['outputs']
            require(output['context'] == 'print-xps' and output['format'] == 18
                    and output['optimize_for'] == 0 and output['use_iso19005_1'] is False
                    and {k: output[k] for k in ('file', 'sha256')} == row['xps'],
                    'Wrong original XPS output context')
            raw = bundle.read(output['file'])
            require(hashlib.sha256(raw).hexdigest() == output['sha256'], 'Original Word XPS changed')
            model = xps_origins(raw)
            require(model['page_count'] == 1 and len(model['glyphs']) == row['glyph_count'],
                    'Original XPS text/page coverage changed')
            cases.append((row, model, name in required))
    return cases


def assert_model(expected, actual, label, required):
    glyphs = [g for g in actual['glyphs'] if not g[1].isspace()]
    require(actual['page_count'] == expected['page_count'], label + ': page count')
    require([g[:2] for g in glyphs] == [g[:2] for g in expected['glyphs']],
            label + ': missing/extra/reordered text')
    try:
        compare(expected, actual, label)
    except ValueError:
        if required:
            raise
        return 'DIFFERENCE'
    return 'PASS'


def check(directories, output, root=ROOT):
    require(sys.platform == 'win32', 'Native XPS comparisons require Windows rendering outputs')
    cases = verify(root); results = []
    for row, expected, required in cases:
        name = row['name']; group = ('printer16' if name.startswith('word-windows-printer-metrics-')
                                    else row['group'])
        folder = directories[group] / name; planes = {}
        for plane in ('native', 'screen'):
            actual = read_json(folder / (plane + '.json'))
            require(actual['input_sha256'] == row['source']['sha256'], 'Wrong native XPS input')
            planes[plane] = assert_model(expected, actual, name + '/' + plane, required)
        results.append(dict(name=name,required=required,planes=planes))
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes((json.dumps(results, indent=2) + '\n').encode())
    print('Asserted 283 required Windows Word XPS cases on both logical output planes; '
          '13 pending coordinate cases retain mandatory text/page assertions')
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compare-rendered', action='store_true')
    parser.add_argument('--sizes-directory', type=Path, default=ROOT / 'target/windows-Aptos-size-coordinates')
    parser.add_argument('--nominal-directory', type=Path, default=ROOT / 'target/windows-aptos-nominal')
    parser.add_argument('--printer-directory', type=Path)
    parser.add_argument('--positive-directory', type=Path, default=ROOT / 'target/windows-positive-kerning')
    parser.add_argument('--output', type=Path, default=ROOT / 'target/windows-word-xps/results.json')
    args = parser.parse_args()
    if args.compare_rendered:
        check(dict(size64=args.sizes_directory.resolve(), nominal40=args.nominal_directory.resolve(),
                   printer16=(args.printer_directory or args.nominal_directory).resolve(),
                   positive192=args.positive_directory.resolve()), args.output.resolve())
    else:
        print('Verified', len(verify()), 'immutable versioned Windows Word XPS references')
