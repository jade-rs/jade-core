"""Replay the 2026-10-19 Word for Mac corpus with the installed Office fonts."""

import argparse
import csv
import hashlib
import json
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from check_platform_coordinates import ROOT, checked_file, compare, pdf_origins, read_json, require


CORPUS = 'tests/word-oracle/mac-glyph-corpus-20261019'
SOURCE_REVISION = '6a223d56f16f1e1deb8e4555565b49dd5d1739ef'
MANIFEST_SIZE = 118
EMPTY_REFERENCES = {'blank', 'table-lacked', 'table-nested-double', 'table-nested-single'}
ARCHIVED_REFERENCE = {
    'noto-jp-yokohama_page6': 'PDF extraction emits CJK radicals for ideographs',
    'system-dev-contract': 'PDF extraction emits CJK radicals for ideographs',
    'table-row-height-overflow': 'PDF extraction includes 50 additional glyphs',
    'tabs': 'PDF extraction emits a CJK radical for an ideograph',
    'toc': 'PDF extraction emits CJK radicals for ideographs',
}
# The third column extends past the page edge. The PDF paint trace includes
# those literal characters and origins without expanding or moving the page.
PDF_PAINT_TRACE = {'yokohama_table'}
# Keep both comparisons visible while these cases are investigated in #222.
PENDING = {
    'char-effects': 'Word paints outline, shadow, emboss and imprint differently',
    'math': 'PDF character mapping and equation layout need separate checks',
    'table-object-edge-cases': 'Word PDF emits Arabic presentation forms and glyph origins differ',
    'toc-tc-field-edge-cases': 'Empty simple TC fields reorder visible text in Word',
    'unicode-segmentation': 'Word paints a joined family emoji and reports different Unicode text units',
}


def manifest():
    with (ROOT / CORPUS / 'SHA256.tsv').open(encoding='utf-8', newline='') as stream:
        rows = list(csv.DictReader(stream, delimiter='\t'))
    names = {row['fixture'] for row in rows}
    require(len(rows) == len(names) == MANIFEST_SIZE, 'Unexpected corpus size or duplicate fixture')
    require(EMPTY_REFERENCES | ARCHIVED_REFERENCE.keys() | PDF_PAINT_TRACE | PENDING.keys() <= names,
            'Unknown fixture in corpus exceptions')
    require(all(Path(name).name == name and name not in ('', '.', '..') for name in names),
            'Invalid fixture name')
    return rows


def fixture_source(name, digest, output):
    relative = f'public/fixtures/{name}.docx'
    current = ROOT / relative
    if current.is_file() and hashlib.sha256(current.read_bytes()).hexdigest() == digest:
        return current
    data = subprocess.check_output(['git', 'show', f'{SOURCE_REVISION}:{relative}'], cwd=ROOT)
    require(hashlib.sha256(data).hexdigest() == digest, f'{name}: original input hash changed')
    source = output / f'{name}.docx'
    source.write_bytes(data)
    return source


def archived_reference(path, row):
    raw = read_json(path)
    for field in ('docx_sha256', 'pdf_sha256'):
        require(raw[field] == row[field], f'{row["fixture"]}: archived {field} changed')
    require((raw['word_version'], raw['word_build']) == ('16.114', '16.114.26101123'),
            f'{row["fixture"]}: unexpected Word build')
    return {
        'page_count': raw['page_count'],
        'glyphs': [
            [page['page'], glyph['text'], glyph['x_pt'],
             glyph.get('baseline_pt', line['baseline_pt'])]
            for page in raw['pages'] for line in page['rows'] for glyph in line['characters']
        ],
    }


def pdf_paint_origins(path):
    import pymupdf

    with pymupdf.open(path) as document:
        return {
            'page_count': len(document),
            'glyphs': [[number, chr(char[0]), *char[2]]
                       for number, page in enumerate(document, 1)
                       for span in page.get_texttrace() for char in span['chars']],
        }


def compare_capture(name, expected, actual):
    word_glyphs = sum(not glyph[1].isspace() for glyph in expected['glyphs'])
    jade_glyphs = sum(not glyph[1].isspace() for glyph in actual['glyphs'])
    error = None
    try:
        if name in EMPTY_REFERENCES:
            require(word_glyphs == jade_glyphs == 0, f'{name}: expected a text-free capture')
            require(actual['page_count'] == expected['page_count'], f'{name}: page count')
        else:
            compare(expected, actual, name)
    except (ValueError, OverflowError) as exception:
        error = str(exception)
    return {'word_glyphs': word_glyphs, 'jade_glyphs': jade_glyphs, 'error': error}


def replay(row, executable, output):
    name = row['fixture']
    archived = checked_file(ROOT, {'file': f'{CORPUS}/{name}.json', 'sha256': row['json_sha256']})
    pdf = checked_file(ROOT, {'file': f'{CORPUS}/pdf/{name}.pdf', 'sha256': row['pdf_sha256']})
    source = fixture_source(name, row['docx_sha256'], output)
    capture = output / f'{name}.json'
    subprocess.run([executable, source, capture, 'mac', '--final-view',
                    '--without-formatting-marks', '--without-page-image'], check=True, timeout=180)
    actual = read_json(capture)
    require(actual['input_sha256'] == actual['render_input_sha256'] == row['docx_sha256'],
            f'{name}: rendered source hash changed')
    # U+FFFC is the layout engine's inline object placeholder.
    actual['glyphs'] = [glyph for glyph in actual['glyphs'] if glyph[1] != '\ufffc']
    use_trace = name in PDF_PAINT_TRACE
    raw = pdf_paint_origins(pdf) if use_trace else pdf_origins(pdf, unclipped=True)
    reference = archived_reference(archived, row)
    require(raw['page_count'] == reference['page_count'] == actual['page_count'],
            f'{name}: page count changed')
    if name not in EMPTY_REFERENCES:
        require(all(any(not glyph[1].isspace() for glyph in layout['glyphs'])
                    for layout in (raw, reference, actual)), f'{name}: empty text capture')
    pdf_result = compare_capture(name, raw, actual)
    archive_result = compare_capture(name, reference, actual)
    selected = 'archive' if name in ARCHIVED_REFERENCE else 'pdf'
    result = archive_result if selected == 'archive' else pdf_result
    return {
        'fixture': name,
        'pages': raw['page_count'],
        'gate': 'pending' if name in PENDING else selected,
        'reason': PENDING.get(name) or ARCHIVED_REFERENCE.get(name),
        'pdf_method': 'paint-trace' if use_trace else 'rawdict',
        'pdf': pdf_result,
        'archive': archive_result,
        'passed': result['error'] is None,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('executable', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    require(sys.platform == 'darwin', 'The corpus needs macOS and its installed Word fonts')
    executable = args.executable.resolve(strict=True)
    args.output.mkdir(parents=True, exist_ok=False)
    rows = manifest()
    with ThreadPoolExecutor(max_workers=2) as pool:
        results = list(pool.map(lambda row: replay(row, executable, args.output), rows))
    report = args.output / 'report.json'
    report.write_text(json.dumps(results, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    for row in results:
        state = 'pass' if row['passed'] else 'diff'
        print(f'{row["fixture"]}\t{row["pages"]}\t{row["gate"]}\t{state}')
    failures = [row['fixture'] for row in results if row['gate'] != 'pending' and not row['passed']]
    print(f'{len(results)} fixtures, {len(failures)} gate failures, {len(PENDING)} pending')
    if failures:
        print(f'Failed fixtures: {", ".join(failures)}', file=sys.stderr)
    return bool(failures)


if __name__ == '__main__':
    sys.exit(main())
