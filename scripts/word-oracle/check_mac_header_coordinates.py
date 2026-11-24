"""Assert original Word for Mac header/body/footer origins independently of Windows."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import subprocess
import sys
from zipfile import ZipFile

from check_platform_coordinates import (
    ROOT, INDEX, archive_reference, checked_file, compare, extract_fonts,
    pdf_origins, read_json, require,
)
from check_mac_captured_source import require_mac_host
from check_windows_font_coordinates import require_original_font_programs_registered

REFERENCES = {'header-tab-overflow-short-mac': ('4fa200bd6280c49b2b0b545b360e038b96fd0f4d9d396dc7efceeb7e39ac4a3c', 'be331583d32a80177fe068f60c202e2899bf8c2f27640d84c9a1a77bbc807f59', '981ef973f87e73468356b5591d25bc05bb5e1eddc0aaa419c80401ad9c9f2762', 1, 55, '20261114T020256534Z-0729bfd7'), 'header-tab-overflow-long-mac': ('fff19b66f4026fc9ebd07e3d6882784fd43f316306b753f5a0ef817f7688f3c2', '7fa9487cb5ac478486c28cf441cd3922ff43601fa413418c206ea533300488b1', '9f8eca775c1197dd2cafe36b74c592a50de39580ab5a1398f32907087b848f26', 1, 58, '20261114T020253083Z-9dcfef94')}


def verify(root=ROOT):
    registry = read_json(root / INDEX)
    rows = []
    for name, (source_sha, manifest_sha, coordinates_sha, pages, glyphs, capture_id) in REFERENCES.items():
        matches = [c for c in registry['fixtures'] if c['mac_case'] == name]
        require(len(matches) == 1, name + ': missing or duplicate header original')
        case = matches[0]
        require(case['source']['sha256'] == source_sha and case['mac_manifest']['sha256'] == manifest_sha,
                name + ': header source or manifest changed')
        source = checked_file(root, case['source'])
        manifest = read_json(checked_file(root, case['mac_manifest']))
        require(manifest['sourceSha256'] == source_sha and manifest['coordinatesSha256'] == coordinates_sha,
                name + ': header input/coordinate identity changed')
        reference = manifest['reference']
        require(reference['application'] == 'Microsoft Word' and reference['platform'] == 'mac'
                and reference['version'] == '16.114' and reference['build'] == '16.114.26101123',
                name + ': original Mac Word version changed')
        raw = source.with_name('pages.json.gz').read_bytes()
        require(hashlib.sha256(raw).hexdigest() == coordinates_sha, name + ': header origins changed')
        captured = json.loads(gzip.decompress(raw))
        expected = dict(page_count=len(captured), glyphs=[
            [i + 1, *g[:3]] for i, page in enumerate(captured)
            for story in ('header', 'body', 'footer') for g in page.get(story, [])])
        require(len(captured) == manifest['pages'] == pages
                and sum(not g[1].isspace() for g in expected['glyphs']) == glyphs,
                name + ': header/body/footer coverage changed')
        require(len(case['windows']) == 1, name + ': font provider coverage changed')
        provider = case['windows'][0]
        require(provider['font_source'] == 'original-word-pdf'
                and provider['word_product_version'] == '16.0.19127.20936'
                and provider['word_executable_sha256'] ==
                '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312'
                and provider['capture_id'] == capture_id, name + ': original font provider changed')
        rows.append((name, case, source, reference, expected, glyphs))
    require(len(rows) == 2 and sum(r[5] for r in rows) == 113, 'Header coverage removed')
    return rows


def check_native(executable, output, root=ROOT, *, require_macos=False):
    require_mac_host(require_macos)
    rows = verify(root)
    output.mkdir(parents=True, exist_ok=False)
    results = []
    for name, case, source, reference, expected, glyphs in rows:
        archive, capture, document, _ = archive_reference(root, case, case['windows'][0])
        target = output/name
        target.mkdir()
        with ZipFile(archive) as bundle:
            data = bundle.read(document['pdf'])
        require(hashlib.sha256(data).hexdigest() == document['pdf_sha256'], name + ': font PDF changed')
        provider_pdf = target/'original-Windows-font-provider.pdf'
        provider_pdf.write_bytes(data)
        fonts = target/'fonts'
        extract_fonts(provider_pdf, fonts)
        actual_file = target/'native.json'
        painted = actual_file.with_suffix('.pdf')
        subprocess.run([str(executable), str(source), str(actual_file), 'mac', str(painted),
                        '--fonts-directory', str(fonts)], check=True)
        actual = read_json(actual_file)
        require(actual['input_sha256'] == case['source']['sha256'], name + ': rendered input changed')
        require_original_font_programs_registered(fonts, actual)
        require(compare(expected, actual, name+'/original-Mac-logical') == glyphs,
                name + ': original header glyphs removed')
        for unclipped in (False, True):
            require(compare(expected, pdf_origins(painted, unclipped), name+'/original-Mac-PDF') == glyphs,
                    name + ': original painted header glyphs removed')
        results.append(dict(name=name, source_sha256=case['source']['sha256'], Mac_reference=reference,
                            Windows_font_provider=capture['word'], capture_id=capture['capture_id'],
                            archive_sha256=case['windows'][0]['archive']['sha256'],
                            original_font_PDF_sha256=document['pdf_sha256'], glyphs=glyphs,
                            native_PDF_sha256=hashlib.sha256(painted.read_bytes()).hexdigest()))
        print(name, 'original Mac header/body/footer coordinates: PASS', flush=True)
    result = dict(host=sys.platform, require_macos=require_macos,
                  native_executable_sha256=hashlib.sha256(executable.read_bytes()).hexdigest(),
                  cases=results, scope='Two immutable Mac Word16.114 header originals,113 glyphs across all three stories. Windows Word16.0.19127.20936 PDFs supply font programs only. No Word launch or generated expected positions; original version/digests retained.')
    (output/'result.json').write_text(json.dumps(result, indent=2)+'\n', encoding='utf-8')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path, default=ROOT/'target/mac-header-originals')
    parser.add_argument('--require-macos', action='store_true')
    args = parser.parse_args()
    if args.native_executable:
        check_native(args.native_executable.resolve(), args.output_directory.resolve(),
                     require_macos=args.require_macos)
    else:
        print('Verified original Mac header references:', len(verify()))
