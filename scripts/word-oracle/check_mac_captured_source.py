"""Assert the exact captured Mac DOCX on an actual macOS renderer.

The historical malformed original and its normalized Windows input are distinct.
Keep both; compare each Mac render with the unchanged Word-for-Mac capture.
Original Windows Word PDF fonts supply deterministic font programs, not positions.
"""
import argparse
import gzip
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

# Immutable imported Word-for-Mac captures. Windows PDFs supply font programs
# only; their coordinates never become Mac expectations. Missing version fields
# in the historical Mac manifests remain missing.
SHARED_ORIGINALS = {
    'courier-new-line-metrics': (
        'a17e0c561725568bcb95b5549cf1fd247af6e34ef34aeba411a38f9b32a08f27',
        'cefeb9d78d82d84607f634f545fcb86c0275a72d3cea4c493ab16ef39cd3b4fe',
        '3da277d5d09f772980616c41dc666a248176ee202cdbb4e01ef47944bfe3f21f', 4, 486),
    'courier-new-regular-adjacent': (
        '89ef3a72d6322925150d96932f407e839cbb1667052966442c76a0407a211465',
        'ec16d63256e3db4cbbd33a8cb982f41d28e6a00d4bbb93ae0a46648182a25032',
        '9e8f3e65417b9425a7d95cdb5e0c0f02f610ca357ea64332c53c34db6961fd14', 4, 390),
    'probe-column-fixed-dxa-4400': (
        '4ea5548ae2e68907aba9b8dc43e834ec07f293f0bb0c04de70a81fe567085b23',
        '96702526e1b4a5eba635d86473921d23c86fcbc87c1621b81029c16a296f54f3',
        'e59ebb510ceefa817f313a79cde0ffd50b5e6af9320165404d3b1bc12ac20deb', 2, 107),
    'probe-column-fixed-dxa-6800': (
        'be054cef36e47288907da7f29e4c135bf4bcd40adec5ddb6432790cbc4b0a861',
        '33193bf919586aba58541ceb9462e48c5f46c5e497dc32c1ae918f02d6e36ee5',
        '3e98f1772071be15003acb84ff297aac762245ab8e55d9f395748d41f6bb3c4e', 2, 107),
    'probe-width-single-dxa-2400-dxa-2000': (
        'a611a63890935fcc6a2a831fc50c19b2641846a8f7fda4a286ba4c033860a358',
        '265bb130473d95a71125d824cc15069f6276d02283d8af1be770cecca45b5475',
        'c150979819c2f84b61e629b95435559e8170b2d9920d757bc2aa97d41ecd8cb9', 3, 107),
    'probe-continuous-column-explicit-break': (
        '036d1a09b7eca1f005fd00fa800c6342951c0ff9fa32574606719d67f2f8c648',
        'ab464ad7e5e0893d4cd29a0ad55d0e083cf5cbd933d674d7fed76fe7fff66012',
        '1c8f463d72dcafad2635cca5c14e8575da8447ea1d8fb0edc4ed2006d6db576d', 1, 242),
}


def shared_originals(root=ROOT):
    registry = read_json(root/'tests/word-oracle/platform-coordinates.json')
    results = []
    for name, (source_sha, manifest_sha, coordinates_sha, pages, glyphs) in SHARED_ORIGINALS.items():
        cases = [c for c in registry['fixtures'] if c['mac_case'] == name]
        require(len(cases) == 1, name + ': missing or duplicate shared Mac original')
        case = cases[0]
        require(case['source']['sha256'] == source_sha
                and case['mac_manifest']['sha256'] == manifest_sha,
                name + ': shared Mac source or manifest identity changed')
        source = checked_file(root, case['source'])
        manifest = read_json(checked_file(root, case['mac_manifest']))
        raw = source.with_name('pages.json.gz').read_bytes()
        require(hashlib.sha256(raw).hexdigest() == coordinates_sha
                == manifest['coordinatesSha256'], name + ': shared Mac coordinates changed')
        require(manifest['sourceSha256'] == source_sha
                and manifest['reference']['platform'] == 'mac'
                and manifest['reference']['application'] == 'Microsoft Word',
                name + ': wrong Mac reference')
        captured = json.loads(gzip.decompress(raw))
        expected = dict(page_count=len(captured), glyphs=[
            [i + 1, *g[:3]] for i, page in enumerate(captured) for g in page['body']])
        require(len(captured) == manifest['pages'] == pages
                and sum(not g[1].isspace() for g in expected['glyphs']) == glyphs,
                name + ': shared Mac glyph/page coverage changed')
        require(len(case['windows']) == 1
                and case['windows'][0]['font_source'] == 'original-word-pdf',
                name + ': missing original font-program provider')
        results.append((name, case, source, manifest, expected, glyphs))
    return results


def check_shared_originals(executable, output, root=ROOT):
    output.mkdir(parents=True, exist_ok=False)
    results = []
    for name, case, source, manifest, expected, glyphs in shared_originals(root):
        provider = case['windows'][0]
        archive, capture, document, _ = archive_reference(root, case, provider)
        target = output/name
        target.mkdir()
        font_pdf = target/'original-Windows-font-provider.pdf'
        with ZipFile(archive) as bundle:
            data = bundle.read(document['pdf'])
        require(hashlib.sha256(data).hexdigest() == document['pdf_sha256'],
                name + ': font provider PDF changed')
        font_pdf.write_bytes(data)
        fonts = target/'fonts'
        extract_fonts(font_pdf, fonts)
        actual_file = target/'native.json'
        painted = target/'native.pdf'
        subprocess.run([str(executable), str(source), str(actual_file), 'mac', str(painted),
                        '--fonts-directory', str(fonts)], check=True)
        actual = read_json(actual_file)
        require_original_font_programs_registered(fonts, actual)
        require(assert_layout(expected, actual, case['source']['sha256'], name) == glyphs,
                name + ': shared Mac glyph coverage removed')
        for unclipped in (False, True):
            require(compare(expected, pdf_origins(painted, unclipped),
                            name + '/original-Mac-painted-PDF') == glyphs,
                    name + ': shared Mac PDF glyph coverage removed')
        results.append(dict(name=name, source_sha256=case['source']['sha256'],
                            Mac_reference=manifest['reference'],
                            Mac_coordinates_sha256=manifest['coordinatesSha256'],
                            font_program_provider_Windows_Word=capture['word'],
                            font_program_provider_archive=provider['archive'],
                            glyphs=glyphs, page_count=expected['page_count']))
    require(len(results) == 6 and sum(r['glyphs'] for r in results) == 1439,
            'Shared Mac original coverage changed')
    print('Six shared Mac originals1439: layout and painted-PDF PASS', flush=True)
    return results


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
    shared_results = check_shared_originals(executable, output/'shared-originals', root)
    result = dict(host=sys.platform,require_macos=require_macos,Mac_Word_version=raw['word_version'],
                  Mac_Word_build=raw['word_build'],Mac_capture_reference_sha256=REFERENCE_SHA256,
                  font_program_provider_Windows_Word=capture['word'],font_program_provider_archive=provider['capture'],
                  actual_native_executable_sha256=hashlib.sha256(executable.read_bytes()).hexdigest(),results=results,
                  shared_originals=shared_results,
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
