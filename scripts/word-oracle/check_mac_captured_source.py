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
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-1': (
        'b0656953e16a6b5ebe488dfcc588e1f84f1ea1c0cea116832a9e9b7a2e3cbd06',
        '9b2573c47db38b50d404ddb5316127eda1303c759f2b9df2653bcd811afd3945',
        '9912015a19baf8ce41cc72029c85928c47628ce86fcdb17e135546675d543f7b', 8, 1014),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-2': (
        'eca02091b6b3faa96fd2f247dbbb4d3f5b8b609c9871d2a5f01b31c2223fcf0c',
        'f77d146d0cb47096eca983cbf22cbb907a4fa681f6d6936248e554e680053918',
        'b706f77852872b3296247a1ad5053d8aeff048484cddd82c380584fe3ed438d6', 8, 1011),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-3': (
        '08637e628e39e2b1172d45327d0b1275fa2ea8fb67316be49a4520a0a1e9f401',
        '1bdb9fdb483ee80dc038eb6fa9bfb1a466e07afa2a8f25527ee74ca87352342e',
        'eaa65938597042ea3949ad276fd01fbccde3043d56d17307b252c128d88b80e8', 8, 1011),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-1': (
        '53d1c76343ca732fb20e04d0c3a03086b9e8f7e2865c9a9fbd983c94eb546b7f',
        'b0a86a767ebf25ec7ddb41964a73599b8b1bced5b4f813d9df0c02b0715d9f72',
        '7588e47a75fb0e63a40738744694ae650630e25df66e594e055cd8a5dcf8cfb6', 9, 1023),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-2': (
        '1c9e71b0420b4f2b250a7343f62e129fceca8e9021ff6afc31936f69e107a2ba',
        'eaa6c983da8ddceabddb6ceee0c249b354205ecdb1512241af923294299efdde',
        '48b68add3efca03ad611ee276bb42b8561186a4699a060d16a1ba4acdb32a400', 9, 1020),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-3': (
        '6e8247c12a218ddf0e56c0cd7db669a7aa3e5d17cf96ffb1bedf2e58fe25c11e',
        'f05ca5d9e001eed87ec444b82d7555544d232f1f6a581ad499a0250ede25c3f5',
        'dc63ceca066d4ee001b00f659561763196d00545bd378e111b196283e86b0ca9', 9, 1020),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-1': (
        'cab1e89ef0cb109481dcac0f75f177215c3328eb2abb20b1fe20f86ee04d7791',
        'b2daaef79e146772f56e363dd2535bba6f44536f0e87639393d453a06636e391',
        'ed6b0f6043f2648aca44e28916467e11e4ef8854b992a6274ad0855328c2b395', 10, 1035),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-2': (
        '451e0512b957693bb77e6b529a5f915b4da5b048a15ad8c867a3f74d5d5d3b28',
        '472c82e406d96c64f7e398e855184bca80a782d7e1ed5e807cb39eb1c4e67eaf',
        'fb5600a150dc90413f2639c3c98cf799639d964f749b914c33bb64c21c02847d', 10, 1032),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-3': (
        'f2e3d8e8dc3045b45ca7d6c626554c769f8faf1a382c51828f139450d80b21ec',
        '21ec0c533aeab78fd8ce355824217ea687da5dd136d071983137346a74478743',
        '22652b26374b18c269457e992f05a63ae653671dc9dca10fb1441cc176ec9f98', 10, 1029),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-1': (
        'db45dbc9ca43ba222e6e56da73a368603ed4960875b025204c04fe054409132a',
        '902578ce2fb7212f75e4643d64fc34b4db4115f57f722b4eb87e337114d4276d',
        '8df64b90a35328cad5fa891e708736d040d10b2d589dd2d75e60962fccfb3feb', 6, 1002),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-2': (
        '94136fa4c50691a4266f40bf15d84d080999210c8fb18e1bb91625b2b49dc12c',
        '25597147aac3e15215bd9f63849c17902bd8402654c0c842d3c2d561b81554b8',
        '7d701b6c02574184ee241f760beba96eff8ff4655c4332caeee67948b052f149', 6, 999),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-3': (
        '31b20a177813df009a6b1745ad8cfd425781e5e7652939b059c77e3e892909ae',
        '81e5aac0c19e661dd056453c5c7bb3170918e12f1cc65d7a3c6af14ca0d63d0f',
        'c96ce48e94adcac630300bdb159517c203f4b60390b151a0d7dc2e5bc9bc6522', 7, 999),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-1': (
        '16f73bae7e7e6cfedcc9913c9025a85d507b894e6bb35521086fc769339c0c6a',
        '1bbe47750aa9f0c54f6fece685fa1de3885a9fca08a372aef263e880185e02c4',
        '047c9cae4ab889ee0c18818a4e53752f067e75bd58562ec7fc85731a298c2149', 12, 1047),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-2': (
        '3cbd2ac87bf90fed62071dd17462ae30888b6737f34440a8ae5d9d129734db67',
        'c066fe63f8a9eae9e9d3b554e53ba622aa374df5242e49bcbbbd173f984a9647',
        'f895721387dbfd0d5d49ea81b1f2f101fe161aa42344ce729e0c8cd2875b143b', 12, 1047),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-3': (
        '6886271736c086c2510908223f58f967c9c4579f126bd2cb7e039c94131385b8',
        '1dcfb94d52ce40b73e164aeee12e9376ca173e6ab046fef1ee9e53df0a51b911',
        '03e9c64dd5aea1add4addf76eddbe90a34a719e8a7a494111ed41d851063d209', 12, 1044),
    'probe-fixed-auto-table-grid-2000-cell-pct-1250': (
        'be95c64a1a69e75986bf913c8f5dab2875b8522024b7c6be69b0bc09f392eb5d',
        'fcfede5701e593c8c0e25df865fcb3b6c310262ec2f3256061e8d816b96f54de',
        '38c12e496292da1d999e69ae621e127d9fe70d5f4a203b24bff8576a38927a9b', 3, 107),
    'probe-fixed-auto-table-grid-1000-cell-pct-2500': (
        '2aa4101101d3b6fd8ab7c99736cebcda3278088e786b8e916574ca790b28add5',
        '17c2be32321b8ef24c4fbe9401c0f4ca027de239efe5ec4d0788d0ea3b545674',
        'd6606df22b93edfd37faedda873a7b175152228e2e715aea49cc8a29e16cbbd6', 3, 59),
    'probe-fixed-auto-table-grid-2000-cell-pct-5000': (
        '5bc7d88f7f89d23ce3e4bd986c38ae1a34c29b8c7e2fd16c953f947908e72767',
        '3d661bacb46f4f70792afdedaa2ae34fc7dc9d69aebdb61e3844c1708a8b2a12',
        'b404095f8e702faf86225ff49e25ca7627b0d74e1af24479d26df6ffc0ed5d3e', 3, 56),
    'probe-fixed-auto-table-grid-4000-cell-pct-2500': (
        '4b7ee04b8912d2e03b26013309a7a2a96de50ff8ed52bc4748587473b1047c2b',
        '0b68b4021f26f75ebafd0da04ae7252e6d91f771f35c50ccc6eabe8b15addc62',
        'd6606df22b93edfd37faedda873a7b175152228e2e715aea49cc8a29e16cbbd6', 3, 59),
    'probe-unequal-variant-narrow-first-2-9-dxa': (
        'c1ff95bbbba66d57c63e1deaac29a8d5eee89c09d5c684ef49285b93d33c7e97',
        'd19f7e75200d0c7d835dc9c41b05a1eacddc2b14ce3a9505fc7d361bdc365330',
        '8566c1efacd37b11c5ae71e5ad3bc36a9e6cc1b40a4aa9518140a503bebc7b99', 2, 96),
    'probe-width-equal-auto-0-pct-2500': (
        'b5b8e4255b28f3848752fec0e32a8a51ea104449ef3b234c5d3490f1d0823fdf',
        'cab1903237094d01f3e164a0692876239d94df78ab4b524385cdb14447514927',
        '9db8f29647e323c8d8bdded50b9103d64097013356ff46473c99db26a4cd3b3b', 2, 59),
    'probe-width-single-auto-0-pct-2500': (
        '507021382dc70224b5842454c13b3a28c4685824e65feb02bbf5257c7c52cfea',
        '919a01910562695afa8d76c491ea0364dcc2f116743c42f27b67f84529f08d97',
        'd6606df22b93edfd37faedda873a7b175152228e2e715aea49cc8a29e16cbbd6', 3, 59),
    'probe-unequal-autofit-wrapped-header': (
        'bfd024d72540b3dcc0be838e37713438e17502e9b83e70c990efadafb96ee597',
        '84641c8a20c2ab6b87dcb2863d7c3083716268c0e8bf36f2200587c81dd042ff',
        'e257cf3ee6c0c3c99601b3cb99244cd32a9854d91e29d8dbf3844662924a7101', 8, 1709),
    'probe-unequal-center-narrow-first-2-9': (
        '095b0e6682e3dadcd39d03a610e1fd23c0a384574db01145882a1266c704c561',
        'f49db3f04a8c32081ad0cfc3b553ce7f36037bdc07e31183ef2b39a2471abf71',
        'd6bdef4bc1aaf164d25ad63d5eedd19a503fcb9e0c4bca70d180a59a311090f5', 2, 152),
    'probe-unequal-center-three-10': (
        '357abec2fde6a5121d936be7e9cdc07e2aeb5b0846ccf358a51d766c97fb38c1',
        '3bbb637c143cc3cd9678e0c4b3b54ff762f07b495a4e3fc8559a17202072a6d5',
        '8b3d72dbd72fcdebf464605a95d75798d7e5237dc529dbd082efc1cf4cb374c0', 1, 115),
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
        arguments = [str(executable), str(source), str(actual_file), 'mac', str(painted),
                     '--fonts-directory', str(fonts)]
        if manifest.get('capturePaintedGlyphs') and manifest.get('clipPaintedGlyphs'):
            arguments.append('--painted-table-headers')
        subprocess.run(arguments, check=True)
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
                            glyphs=glyphs, page_count=expected['page_count'],
                            native_table_glyph_projection=actual['table_glyph_projection']))
    require(len(results) == 31 and sum(r['glyphs'] for r in results) == 19243,
            'Shared Mac original coverage changed')
    print('31 shared Mac originals19243: layout and painted-PDF PASS', flush=True)
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
