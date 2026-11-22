"""Assert the exact captured Mac DOCX on an actual macOS renderer.

The historical malformed original and its normalized Windows input are distinct.
Keep both; compare each Mac render with the unchanged Word-for-Mac capture.
Original Windows Word PDF fonts supply deterministic font programs, not positions.
"""
import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path
from zipfile import ZipFile
from check_platform_coordinates import (ROOT, archive_reference, checked_file, compare,
                                       extract_fonts, pdf_origins, read_json, require)
from check_windows_font_coordinates import require_original_font_programs_registered

ORIGINAL = 'crates/jade-docx-native-render/tests/fixtures/word-mac-probe-floating-order-malformed.docx'
ORIGINAL_SHA256 = 'eb417a0f7dc283af99425b43a28a356f90ed2ba5a53112e93508b763996b6ba7'
CONTROL = 'public/fixtures/word-mac-probe-floating-order.docx'
CONTROL_SHA256 = '32e7c583bddf915d4a036bc87222064b835993347fc52b0f9eabc8c912c6db96'
REFERENCE = 'tests/word-oracle/mac-probes-20261019/word-mac-probe-floating-order.json'
REFERENCE_SHA256 = 'cb175d41ea402a4d627322a067593deb8a62a9a11b079e75dbb8aded5c045504'


def require_mac_host(required):
    require(not required or sys.platform == 'darwin', 'Captured-original CI requires macOS')


def originals(root=ROOT):
    original = checked_file(root, dict(file=ORIGINAL, sha256=ORIGINAL_SHA256))
    control = checked_file(root, dict(file=CONTROL, sha256=CONTROL_SHA256))
    reference = checked_file(root, dict(file=REFERENCE, sha256=REFERENCE_SHA256))
    raw = read_json(reference)
    require(raw['docx_sha256'] == ORIGINAL_SHA256, 'Mac original capture input changed')
    require(raw['word_version'] == '16.114' and raw['word_build'] == '16.114.26101123',
            'Mac original Word version/build changed')
    require(raw['pdf_sha256'] == '079f2ff60c9a8731869fedd7c155b9c3e5870793d185aab7a3d0321761f5dd96',
            'Mac original PDF identity changed')
    require(original.read_bytes() != control.read_bytes(), 'Historical distinct inputs were collapsed')
    expected = dict(page_count=raw['page_count'], glyphs=[
        [page['page'], glyph['text'], glyph['x_pt'], row['baseline_pt']]
        for page in raw['pages'] for row in page['rows'] for glyph in row['characters']])
    require(expected['page_count'] == 1 and sum(not g[1].isspace() for g in expected['glyphs']) == 1070,
            'Mac original glyph/page coverage changed')
    return original, control, raw, expected


def assert_layout(expected, actual, source_sha256, label):
    require(actual['input_sha256'] == source_sha256, label + ': wrong rendered input')
    return compare(expected, actual, label + '/original-Mac-layout')


def check_native(executable, output, root=ROOT, *, require_macos=False):
    require_mac_host(require_macos)
    original, control, raw, expected = originals(root)
    index = read_json(root/'tests/word-oracle/windows/font-coordinates.json')
    provider, = [c for c in index['fixtures'] if c['name'] ==
                 'word-windows-floating-order-compat14-word16.0.19127.20936']
    require(provider['source']['sha256'] == CONTROL_SHA256
            and provider['word_product_version'] == '16.0.19127.20936'
            and provider['capture']['sha256'] == 'c4af57ec9ebdc3bc52b38791f01fc5a941f0177623b05aac670b3baebf51f8c4',
            'Original font-program provider changed')
    archive, capture, document, _ = archive_reference(
        root, {'source': provider['source']}, dict(provider, archive=provider['capture']))
    output.mkdir(parents=True, exist_ok=False)
    font_pdf = output/'original-Windows-font-provider.pdf'
    with ZipFile(archive) as bundle:
        data = bundle.read(document['pdf'])
    require(hashlib.sha256(data).hexdigest() == document['pdf_sha256'], 'Font provider PDF changed')
    font_pdf.write_bytes(data)
    fonts = output/'fonts'
    extract_fonts(font_pdf, fonts)
    results = []
    for tag, source, digest in [('captured-original', original, ORIGINAL_SHA256),
                                ('normalized-control', control, CONTROL_SHA256)]:
        actual_file = output/(tag+'.json')
        painted_pdf = actual_file.with_suffix('.pdf')
        subprocess.run([str(executable), str(source), str(actual_file), 'mac', str(painted_pdf),
                        '--fonts-directory', str(fonts)], check=True)
        actual = read_json(actual_file)
        require_original_font_programs_registered(fonts, actual)
        count = assert_layout(expected, actual, digest, tag)
        require(count == 1070, 'Mac original coordinate coverage removed')
        for unclipped in (False, True):
            require(compare(expected, pdf_origins(painted_pdf, unclipped),
                            tag+'/original-Mac-painted-PDF') == 1070, 'Mac PDF glyphs removed')
        results.append(dict(input=tag,source_sha256=digest,glyphs=count,page_count=actual['page_count']))
    a, b = [read_json(output/(tag+'.json')) for tag in ('captured-original','normalized-control')]
    require(a['glyphs'] == b['glyphs'] and a['page_count'] == b['page_count'],
            'Schema normalization changed Mac origins/pages')
    require((output/'captured-original.png').read_bytes() ==
            (output/'normalized-control.png').read_bytes(), 'Schema normalization changed Mac pixels')
    result = dict(host=sys.platform,require_macos=require_macos,Mac_Word_version=raw['word_version'],
                  Mac_Word_build=raw['word_build'],Mac_capture_reference_sha256=REFERENCE_SHA256,
                  font_program_provider_Windows_Word=capture['word'],font_program_provider_archive=provider['capture'],
                  actual_native_executable_sha256=hashlib.sha256(executable.read_bytes()).hexdigest(),results=results,
                  scope='Actual distinct original and normalized Mac renders, each against1070 unchanged original Word glyphs plus real painted PDF on both clipping paths. Font programs from retained Windows Word PDF; original Mac PDF bytes not retained, original raw coordinates/digest preserved. No Word launch, fabricated expected positions, or snapshot update.')
    (output/'result.json').write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
    print('Captured Mac original1070 and normalized control1070: layout/painted-PDF/PNG PASS',flush=True)
    return result


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable',type=Path,required=True)
    parser.add_argument('--output-directory',type=Path,required=True)
    parser.add_argument('--require-macos',action='store_true')
    args=parser.parse_args()
    check_native(args.native_executable.resolve(),args.output_directory.resolve(),require_macos=args.require_macos)
