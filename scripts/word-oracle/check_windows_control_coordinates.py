"""Assert retained Windows Word control-character paint and editable origins."""
import argparse
import collections
import copy
import gzip
import hashlib
import json
import math
from pathlib import Path
import subprocess
import sys
import xml.etree.ElementTree as ET
from zipfile import ZipFile

from check_platform_coordinates import (
    ROOT, archive_reference, checked_file, compare as compare_coordinates, extract_fonts, pdf_origins,
    read_json, repo_file, require,
)
from check_windows_font_coordinates import require_original_font_programs_registered

INDEX = 'tests/word-oracle/windows/control-characters.json'
REQUIRED = {('probe-latin-join-controls-line-metrics', '16.0.19127.20936'):
            'f85f726496ea2f379b4f212c310774e5ee586c258ca7269e76c6f54edbc87de9'}
CONTROLS = frozenset('\u200c\u200d')


def compare(expected, actual, label, *, font_metrics=False):
    # This original is a plain table with matching physical reading order.
    # Keep an explicit order assertion in addition to the shared XY matcher,
    # whose general floating-text cases permit equivalent extraction order.
    texts = [[g[:2] for g in model['glyphs'] if not g[1].isspace()]
             for model in (expected, actual)]
    require(texts[0] == texts[1], label + ': text/page/order changed')
    return compare_coordinates(expected, actual, label, font_metrics=font_metrics)


def control_counts(text):
    return collections.Counter(ch for ch in text if ch in CONTROLS)


def source_control_counts(source):
    with ZipFile(source) as bundle:
        root = ET.fromstring(bundle.read('word/document.xml'))
    text = ''.join(node.text or '' for node in root.iter(
        '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}t'))
    return control_counts(text)


def model_control_counts(model):
    return control_counts(''.join(g[1] for g in model['glyphs']))


def editable_origins(model, expected_counts):
    """Project editable format characters only; never alter painted PDF text.

    The original source keeps32 controls but Word emits none in PDF/EMF.
    This additional check asserts visible logical origins without changing
    the existing general comparator or any physical output assertion.
    """
    require(model_control_counts(model) == expected_counts,
            'Editable source controls were lost, duplicated or changed')
    result = copy.deepcopy(model)
    glyphs = []
    for original in result['glyphs']:
        glyph = list(original)
        text = glyph[1]
        if any(ch in CONTROLS for ch in text):
            require(all(ch in CONTROLS for ch in text)
                    or (text[0].isascii() and text[0].isalpha()
                        and all(ch in CONTROLS for ch in text[1:])),
                    'Control projection cannot split complex or combining text')
            glyph[1] = ''.join(ch for ch in text if ch not in CONTROLS)
        if glyph[1]:
            glyphs.append(glyph)
    result['glyphs'] = glyphs
    return result


def assert_blank_regions(word_pdf, native_pdf, regions):
    """Reject even unsearchable/faint extra ink in original blank control slots."""
    import pymupdf
    with pymupdf.open(word_pdf) as word, pymupdf.open(native_pdf) as actual:
        require(len(word) == len(actual), 'Blank-slot page count changed')
        for region in regions:
            page = region['page'] - 1
            rect = region['rect']
            require(0 <= page < len(word) and len(rect) == 4
                    and all(math.isfinite(v) for v in rect)
                    and rect[0] < rect[2] and rect[1] < rect[3], 'Invalid blank slot')
            clip = pymupdf.Rect(rect)
            require(word[page].rect.contains(clip) and actual[page].rect.contains(clip),
                    'Blank slot outside page')
            pixels = [doc[page].get_pixmap(matrix=pymupdf.Matrix(4, 4), clip=clip,
                       colorspace=pymupdf.csGRAY, alpha=False) for doc in (word, actual)]
            require(pixels[0].width == pixels[1].width
                    and pixels[0].height == pixels[1].height, 'Blank slot dimensions changed')
            for label, pixel in zip(('original Word', 'native'), pixels):
                require(bool(pixel.samples) and min(pixel.samples) == 255,
                        f'{label}: extra ink in control-only slot on page{page + 1}')


def verify(root=ROOT):
    index = read_json(repo_file(root, INDEX))
    require(index['schema'] == 1 and bool(index['fixtures']), 'Missing control fixtures')
    rows = []
    found = set()
    for case in index['fixtures']:
        name = case['mac_case']
        source = checked_file(root, case['source'])
        prefix = f'tests/word-oracle/mac-elsa-synthetic/{name}/'
        require(case['source']['file'] == prefix + 'source.docx', 'Wrong shared source')
        mac = read_json(checked_file(root, case['mac_manifest']))
        require(case['mac_manifest']['file'] == prefix + 'manifest.json'
                and mac['sourceSha256'] == case['source']['sha256']
                and mac['reference']['platform'] == 'mac'
                and mac['reference']['application'] == 'Microsoft Word', 'Mac original changed')
        pages = repo_file(root, prefix + 'pages.json.gz').read_bytes()
        require(hashlib.sha256(pages).hexdigest() == mac['coordinatesSha256']
                and len(json.loads(gzip.decompress(pages))) == mac['pages'], 'Mac coordinates changed')
        require(len(case['windows']) == 1, 'Control fixture requires its original version capture')
        windows, = case['windows']
        key = (name, windows['word_product_version'])
        require(key in REQUIRED and key not in found
                and case['source']['sha256'] == REQUIRED[key], 'Unknown/duplicate control source')
        found.add(key)
        archive, capture, document, drawing = archive_reference(root, case, windows)
        require(document['pages'] == 8 and len(drawing['glyphs']) == 572,
                'Original control fixture page/glyph coverage changed')
        require(not model_control_counts(drawing), 'Original Word unexpectedly paints controls')
        regions = read_json(checked_file(root, case['blank_regions']))
        require(regions['source_sha256'] == case['source']['sha256']
                and regions['capture_sha256'] == windows['capture_sha256']
                and regions['word_pdf_sha256'] == document['pdf_sha256']
                and regions['word'] == capture['word'], 'Blank regions lost original identity')
        require([r['page'] for r in regions['regions']] == list(range(1, 9)),
                'Every original control-only page must be checked')
        counts = source_control_counts(source)
        require(counts == collections.Counter({'\u200c': 8, '\u200d': 24}),
                'Original editable controls changed')
        rows.append((case, source, archive, capture, document, drawing, regions['regions'], counts))
    require(found == set(REQUIRED), 'Required versioned control fixture removed')
    return rows


def check_windows(executable, output, root=ROOT):
    require(sys.platform == 'win32', 'Windows control paint requires Windows')
    results = []
    for case, source, archive, capture, document, drawing, regions, counts in verify(root):
        windows, = case['windows']
        target = (output / ('word-' + windows['word_product_version'])
                  / windows['capture_id'] / case['mac_case'])
        target.mkdir(parents=True, exist_ok=False)
        with ZipFile(archive) as bundle:
            original = target / 'original-word.pdf'
            original.write_bytes(bundle.read(document['pdf']))
        fonts = target / 'fonts'
        extract_fonts(original, fonts)
        native_pdf = target / 'native.pdf'
        models = []
        for label, extra in [('native', [str(native_pdf), '--unclipped-tables']), ('screen', [])]:
            path = target / (label + '.json')
            subprocess.run([str(executable), str(source), str(path), 'windows', *extra,
                            '--fonts-directory', str(fonts)], check=True)
            model = read_json(path)
            require(model['input_sha256'] == case['source']['sha256'], 'Wrong control source rendered')
            require_original_font_programs_registered(fonts, model)
            models.append(editable_origins(model, counts))
        painted = pdf_origins(native_pdf)
        require(not model_control_counts(painted), 'Nonprinting controls leaked into native PDF')
        require(not model_control_counts(pdf_origins(original)), 'Word PDF has control text')
        require(native_pdf.read_bytes() != original.read_bytes(), 'Word self-comparison')
        compare(drawing, painted, case['mac_case'] + '/painted-EMF')
        compare(drawing, models[1], case['mac_case'] + '/editable-screen-EMF')
        for unclipped in (False, True):
            word = pdf_origins(original, unclipped, font_metrics=True)
            actual = pdf_origins(native_pdf, unclipped, font_metrics=True)
            compare(word, actual, case['mac_case'] + '/painted-fonts', font_metrics=True)
            compare(pdf_origins(original, unclipped), models[0 if unclipped else 1],
                    case['mac_case'] + '/editable-origins')
        assert_blank_regions(original, native_pdf, regions)
        results.append(dict(name=case['mac_case'],word=capture['word'],
                            capture_id=capture['capture_id'],
                            archive_sha256=windows['archive']['sha256'],
                            source_sha256=case['source']['sha256'],
                            mac_manifest_sha256=case['mac_manifest']['sha256'],
                            capture_sha256=windows['capture_sha256'],
                            word_pdf_sha256=document['pdf_sha256'],
                            native_pdf_sha256=hashlib.sha256(native_pdf.read_bytes()).hexdigest(),
                            pages=8,visible_glyphs=572,editable_controls=32,blank_slots=8))
        print(case['mac_case'], capture['word']['product_version'],
              'physical coordinates, editable controls and blank-slot ink: PASS', flush=True)
    (output / 'results.json').write_text(json.dumps(results, indent=2) + '\n', encoding='utf8')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'target/windows-control-coordinates')
    args = parser.parse_args()
    if args.native_executable:
        check_windows(args.native_executable.resolve(), args.output_directory.resolve())
    else:
        print(f'Verified{len(verify())} retained Windows control-character originals')
