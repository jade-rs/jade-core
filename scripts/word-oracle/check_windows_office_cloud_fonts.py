"""Assert native Office font discovery with captured fonts in an isolated cache."""
import argparse
import hashlib
import os
from pathlib import Path
import subprocess
import sys

import pymupdf

from check_platform_coordinates import checked_file, compare, extract_fonts, pdf_origins, read_json, require
from extract_windows_emf import extract

ROOT = Path(__file__).resolve().parents[2]
INDEX = Path('tests/word-oracle/windows/office-cloud-font-observations.json')


def require_cached_body_font(case, rendered, cache):
    expected = {p['sha256'] for p in case['original_word_font_programs']
                if case['font_family'].casefold() in {f.casefold() for f in p['legacy_family']}
                and p['weight'] == 400 and 'Regular' in p['subfamily']}
    require(bool(expected), 'Missing original regular body font')
    selected = [f for f in rendered['fonts']
                if f['family'].casefold() == case['font_family'].casefold()
                and f['weight'] == 400 and not f['italic']]
    require(bool(selected), 'Missing cached body font registration')
    for face in selected:
        path = Path(face['path']).resolve()
        require(path.is_relative_to(cache.resolve()), 'Body font came from outside the isolated cache')
        require(hashlib.sha256(path.read_bytes()).hexdigest() in expected,
                'Body font differs from the original Word font program')


def mac_signature(path):
    model = read_json(path)
    with pymupdf.open(path.with_suffix('.pdf')) as pdf:
        pages = [(hashlib.sha256(page.get_pixmap().samples).hexdigest(), repr(page.get_drawings()))
                 for page in pdf]
    return model['glyphs'], model['page_count'], path.with_suffix('.png').read_bytes(), pages


def check(executable, output, root=ROOT):
    require(sys.platform == 'win32', 'Windows Office font discovery requires Windows')
    cases = read_json(root / INDEX)['fixtures']
    require(len(cases) == 8, 'Office font discovery coverage changed')
    for case in cases:
        source = checked_file(root, case['source'])
        manifest = checked_file(root, case['capture'])
        capture = read_json(manifest)
        require(capture['word'] == case['word'], 'Original Word build changed')
        document, = capture['documents']
        drawing, = extract(manifest)['fixtures']
        reference = read_json(checked_file(root, case['drawing_reference']))
        require(reference['fixtures'] == [drawing], 'Original Word drawing changed')
        target = output / case['name']
        target.mkdir(parents=True, exist_ok=False)
        local = target / 'Office font cache'
        # Deliberately not generation 4, so the gate rejects a hard-coded path.
        cache = local / 'Microsoft/FontCache/fixture-generation/CloudFonts'
        original = manifest.parent / document['pdf']
        extract_fonts(original, cache)
        environment = dict(os.environ)
        environment.pop('JADE_OFFICE_FONT_DIR', None)
        environment['LOCALAPPDATA'] = str(local.resolve())
        actual, visible, painted = target / 'native.json', target / 'visible.json', target / 'native.pdf'
        subprocess.run([str(executable), str(source), str(actual), 'windows', str(painted),
                        '--unclipped-tables'], env=environment, check=True)
        subprocess.run([str(executable), str(source), str(visible), 'windows'], env=environment, check=True)
        native, native_visible = read_json(actual), read_json(visible)
        for model in (native, native_visible):
            require(model['input_sha256'] == case['source']['sha256'], 'Wrong discovery source')
            require_cached_body_font(case, model, cache)
        compare(drawing, native_visible, case['name'] + '/cache/EMF')
        compare(pdf_origins(original, True), native, case['name'] + '/cache/PDF-unclipped')
        compare(pdf_origins(original), native_visible, case['name'] + '/cache/PDF-visible')
        require(painted.read_bytes() != original.read_bytes(), 'Word self-comparison')
        compare(pdf_origins(original), pdf_origins(painted), case['name'] + '/cache/painted-PDF')
        # An available Windows Office cache must not alter Mac-mode font
        # selection on this same host. This is an environment contrast, not a
        # renderer-generated replacement for the actual Mac Word references.
        mac_before, mac_after = target / 'mac-empty.json', target / 'mac-cache.json'
        empty = dict(environment, LOCALAPPDATA=str((target / 'empty profile').resolve()))
        for path, context in ((mac_before, empty), (mac_after, environment)):
            subprocess.run([str(executable), str(source), str(path), 'mac',
                            str(path.with_suffix('.pdf'))], env=context, check=True)
        require(mac_signature(mac_before) == mac_signature(mac_after),
                case['name'] + ': Windows cloud cache changed Mac glyph/page/screen/PDF paint')
        print(case['name'], case['word']['product_version'],
              'automatic Office cache, four coordinate planes and unchanged Mac inventory: PASS', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path, required=True)
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'target/windows-office-cloud-fonts')
    args = parser.parse_args()
    check(args.native_executable.resolve(), args.output_directory.resolve())
