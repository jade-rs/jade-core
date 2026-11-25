"""Assert versioned Windows control lines and independent original Mac coordinates.

Every source character survives in layout. Windows PDF assertions cover all
visible and fully clipped glyphs with original font programs, names and ems.
Mac retains its own raw coordinate capture; original Mac PDF bytes were not
retained, so this additional gate claims logical coordinates, not PDF parity.
"""
import argparse
import collections
import gzip
import hashlib
import json
import math
from pathlib import Path
import subprocess
import sys
from zipfile import ZipFile

from check_platform_coordinates import (
    ROOT, archive_reference, checked_file, compare as compare_coordinates,
    extract_fonts, pdf_origins, read_json, require,
)
from check_windows_control_coordinates import (
    assert_blank_regions, compare, editable_origins, model_control_counts,
    source_control_counts,
)
from check_windows_font_coordinates import require_original_font_programs_registered

REFERENCE = 'tests/word-oracle/windows/word-16.0.19127.20936/zwnj-table-line-metrics.json'
REFERENCE_SHA256 = 'bb692d002cee7c2e289e4e5c9559c1232ff9024488de99376fb87bb436206e20'
SOURCE_SHA256 = 'f6694aeebff9e705c22e84a2436f46d090c78641d4aca21227947a4120526b63'
EXPECTED = dict(pages=8, controls={'\u200c': 8}, emf_glyphs=358,
                visible_glyphs=356, unclipped_glyphs=358, clipped_glyphs=2,
                mac_glyphs=358)


def verify(root=ROOT):
    case = read_json(checked_file(root, dict(file=REFERENCE, sha256=REFERENCE_SHA256)))
    require(case['schema'] == 1 and case['mac_case'] == 'zwnj-table-line-metrics'
            and case['compatibility_mode'] == 14 and case['expected'] == EXPECTED,
            'ZWNJ source/coverage policy changed')
    prefix = 'tests/word-oracle/mac-elsa-synthetic/zwnj-table-line-metrics/'
    require(case['source']['file'] == prefix + 'source.docx'
            and case['source']['sha256'] == SOURCE_SHA256, 'Wrong ZWNJ source')
    source = checked_file(root, case['source'])
    require(source_control_counts(source) == collections.Counter(EXPECTED['controls']),
            'Original editable ZWNJ controls changed')
    mac = read_json(checked_file(root, case['mac_manifest']))
    require(case['mac_manifest']['file'] == prefix + 'manifest.json'
            and mac['sourceSha256'] == SOURCE_SHA256
            and mac['reference']['platform'] == 'mac'
            and mac['reference']['application'] == 'Microsoft Word', 'Mac original changed')
    raw = checked_file(root, dict(file=prefix + 'pages.json.gz',
                                 sha256=mac['coordinatesSha256'])).read_bytes()
    pages = json.loads(gzip.decompress(raw))
    mac_model = dict(page_count=len(pages),
                     glyphs=[[i + 1, *g] for i, page in enumerate(pages) for g in page['body']])
    require(mac_model['page_count'] == mac['pages'] == EXPECTED['pages']
            and len(mac_model['glyphs']) == EXPECTED['mac_glyphs'], 'Mac glyph coverage changed')
    require(len(case['windows']) == 1, 'Original Windows version capture required')
    windows, = case['windows']
    require(windows['word_product_version'] == '16.0.19127.20936', 'Wrong Windows Word version')
    archive, capture, document, emf = archive_reference(root, case, windows)
    require(document['pages'] == EXPECTED['pages']
            and len(emf['glyphs']) == EXPECTED['emf_glyphs']
            and not model_control_counts(emf), 'Original EMF glyph coverage changed')
    require([r['page'] for r in case['blank_regions']] == list(range(1, 9)),
            'Every original blank control line must be checked')
    return case, source, archive, capture, document, emf, mac, mac_model


def assert_windows_planes(emf, word_visible, word_all, source_visible, source_all,
                          painted_visible, painted_all):
    # Source order is mandatory for all glyphs, including the two clipped Bs.
    compare(emf, source_all, 'ZWNJ/source-EMF')
    compare(word_all, source_all, 'ZWNJ/source-PDF-unclipped')
    compare(word_visible, source_visible, 'ZWNJ/source-PDF-visible')
    compare(word_visible, painted_visible, 'ZWNJ/painted-visible', font_metrics=True)
    # Skia's retained fully clipped commands use a separate invisible layer.
    # Its stream interleaving differs; every physical glyph still needs its
    # own exact text/page/font/em partner at the original XY. Visible PDF
    # order and complete logical source order remain asserted above.
    compare_coordinates(word_all, painted_all, 'ZWNJ/painted-unclipped', font_metrics=True)
    compare_coordinates(emf, painted_all, 'ZWNJ/painted-EMF-unclipped')


def assert_mac_original(expected, actual, tolerance):
    require(math.isfinite(tolerance) and tolerance > 0, 'Invalid original Mac tolerance')
    require(expected['page_count'] == actual['page_count'], 'Mac page count changed')
    a, b = [[g for g in model['glyphs'] if not g[1].isspace()]
            for model in (expected, actual)]
    require(bool(a) and [g[:2] for g in a] == [g[:2] for g in b],
            'Mac text/page/order changed')
    for i, (word, rendered) in enumerate(zip(a, b)):
        require(all(math.isfinite(rendered[axis])
                    and abs(word[axis] - rendered[axis]) <= tolerance for axis in (2, 3)),
                f'Mac original axis mismatch at glyph{i}')


def check_native(executable, output, platform, require_macos=False, root=ROOT):
    require(platform in ('windows', 'mac'), 'Unknown ZWNJ platform')
    require(not require_macos or (platform == 'mac' and sys.platform == 'darwin'),
            'Actual macOS host required')
    require(platform != 'windows' or sys.platform == 'win32', 'Actual Windows host required')
    case, source, archive, capture, document, emf, mac, mac_model = verify(root)
    output.mkdir(parents=True, exist_ok=False)
    original = output / 'original-windows-word.pdf'
    with ZipFile(archive) as bundle:
        original.write_bytes(bundle.read(document['pdf']))
    fonts = output / 'fonts'
    extract_fonts(original, fonts)
    counts = collections.Counter(EXPECTED['controls'])
    models = {}
    native_pdf = output / 'native.pdf'
    for name, unclipped in [('unclipped', True), ('visible', False)]:
        if platform == 'mac' and not unclipped:
            continue
        path = output / (name + '.json')
        options = [str(native_pdf)] if unclipped else []
        if unclipped:
            options.append('--unclipped-tables')
        subprocess.run([str(executable), str(source), str(path), platform, *options,
                        '--fonts-directory', str(fonts)], check=True)
        actual = read_json(path)
        require(actual['input_sha256'] == SOURCE_SHA256, 'Wrong ZWNJ source rendered')
        require_original_font_programs_registered(fonts, actual)
        models[name] = editable_origins(actual, counts)
    if platform == 'windows':
        word_visible = pdf_origins(original, font_metrics=True)
        word_all = pdf_origins(original, True, font_metrics=True)
        painted_visible = pdf_origins(native_pdf, font_metrics=True)
        painted_all = pdf_origins(native_pdf, True, font_metrics=True)
        require(len(word_visible['glyphs']) == EXPECTED['visible_glyphs']
                and len(word_all['glyphs']) == EXPECTED['unclipped_glyphs']
                and len(word_all['glyphs']) - len(word_visible['glyphs']) == EXPECTED['clipped_glyphs'],
                'Original PDF visible/unclipped coverage changed')
        require(not model_control_counts(painted_all), 'Controls leaked into painted PDF text')
        require(native_pdf.read_bytes() != original.read_bytes(), 'Word self-comparison')
        assert_windows_planes(emf, word_visible, word_all, models['visible'], models['unclipped'],
                              painted_visible, painted_all)
        assert_blank_regions(original, native_pdf, case['blank_regions'])
    else:
        assert_mac_original(mac_model, models['unclipped'], mac['axisTolerancePt'])
    result = dict(platform=platform, host=sys.platform, require_macos=require_macos,
                  source_sha256=SOURCE_SHA256, fixture_sha256=REFERENCE_SHA256,
                  Mac_reference=mac['reference'], Mac_manifest_sha256=case['mac_manifest']['sha256'],
                  Mac_coordinates_sha256=mac['coordinatesSha256'],
                  font_program_provider_Windows_Word=capture['word'],
                  Windows_capture_id=capture['capture_id'], Windows_capture_sha256=case['windows'][0]['capture_sha256'],
                  Windows_archive_sha256=case['windows'][0]['archive']['sha256'],
                  Windows_original_PDF_sha256=document['pdf_sha256'],
                  native_PDF_sha256=hashlib.sha256(native_pdf.read_bytes()).hexdigest(),
                  pages=EXPECTED['pages'], editable_controls=8,
                  axis_tolerance_pt=mac['axisTolerancePt'] if platform == 'mac' else .375,
                  visible_glyphs=EXPECTED['visible_glyphs'] if platform == 'windows' else EXPECTED['mac_glyphs'],
                  unclipped_glyphs=EXPECTED['unclipped_glyphs'],
                  blank_slots=8 if platform == 'windows' else 0,
                  scope='Windows: all logical/visible/fully clipped physical origins, original fonts/em and blank ink. Mac: all unchanged original logical coordinates and source order; original Mac PDF bytes/version were not retained. No original references or tolerances changed.')
    (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n', encoding='utf8')
    print(platform, 'original ZWNJ table coordinates: PASS')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path, default=ROOT/'target/zwnj-table-coordinates')
    parser.add_argument('--platform', choices=('windows', 'mac'), default='windows')
    parser.add_argument('--require-macos', action='store_true')
    args = parser.parse_args()
    if args.native_executable:
        check_native(args.native_executable.resolve(), args.output_directory.resolve(),
                     args.platform, args.require_macos)
    else:
        verify()
        print('Verified retained Windows/Mac ZWNJ table original')
