"""Check original Word paragraph-order controls without rewriting expectations."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
from zipfile import ZipFile

import fitz

from check_platform_coordinates import ROOT, checked_file, pdf_origins, read_json, require
from check_windows_aptos_sizes import assert_outputs, assert_screen_outputs
from check_windows_font_coordinates import has_visible_vector_ink, require_original_font_programs_registered
from check_windows_word_boundaries import assert_pending_pdf_text
from check_windows_xps import assert_model
from extract_windows_emf import extract
from windows_xps_fonts import extract_xps_fonts
import generate_windows_y_pdf_context_probes as producer

INDEX = Path('tests/word-oracle/windows/y-pdf-context-observations.json')
INDEX_SHA256 = 'ac87867814c055875293273e4bda27b28447492c2c2343ba62f725304eec5b08'
PRINT_NAMES_SHA256 = '6e12da6614b7e6cc8cc90e90673a377314d1c46ae2078341e79a69a6cd6ad378'


def locked_promotions(index):
    rows = index['fixtures']
    require(index['schema'] == 1 and len(rows) == len({r['name'] for r in rows}) == 24,
            'Paragraph-order source coverage changed')
    required = sorted(r['name'] for r in rows if r['coordinate_gate_promotion'] == 'y-context-print')
    require(len(required) == 14 and hashlib.sha256(json.dumps(
        required, separators=(',', ':')).encode()).hexdigest() == PRINT_NAMES_SHA256,
        'Required paragraph-order PDF assertions changed')
    require(all(r['screen_gate_promotion'] == 'y-context-screen'
                and r['xps_gate_promotion'] == 'y-context-xps'
                and r['coordinate_gate_promotion'] in ('y-context-print', 'pending') for r in rows),
            'Required paragraph-order screen/XPS assertion removed')
    require(sum(r['glyph_count'] for r in rows) == 2112, 'Paragraph-order glyph coverage changed')
    return frozenset(r['name'] for r in rows if r['coordinate_gate_promotion'] == 'pending')


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Original paragraph-order registry changed')
    index = read_json(root / INDEX)
    pending = locked_promotions(index)
    parent = read_json(checked_file(root, index['parent_index']))
    mac_path = checked_file(root, index['mac_parent_manifest'])
    mac = read_json(mac_path)
    require(parent['mac_parent_reference'] == index['mac_parent_reference'] == mac['reference']
            and mac['reference']['platform'] == 'mac'
            and hashlib.sha256(mac_path.with_name('source.docx').read_bytes()).hexdigest()
            == mac['sourceSha256'], 'Original Mac parent changed')
    generated = {name: (data, spec) for name, data, spec in producer.generate(root)}
    first = index['fixtures'][0]
    manifest = checked_file(root, first['capture'])
    capture = read_json(manifest)
    require(capture['schema'] == 2 and capture['status'] == 'complete'
            and capture['platform'] == 'windows' and capture['word'] == index['word']
            and capture['capture_id'] == index['capture_id']
            and capture['word']['product_version'] == '16.0.19127.20936'
            and capture['word']['executable_sha256'] ==
            '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312'
            and manifest.parent.parent.name == 'word-' + capture['word']['product_version'],
            'Original paragraph-order Word identity changed')
    documents = {r['name']: r for r in capture['documents']}
    drawings = {r['name']: r for r in extract(manifest)['fixtures']}
    facts_path = checked_file(root, index['font_facts'])
    fonts = {r['name']: r for r in read_json(facts_path)}
    require(len(documents) == len(capture['documents']) == 24
            and set(generated) == set(documents) == set(fonts) == set(drawings),
            'Paragraph-order capture coverage changed')
    cases = []
    with ZipFile(checked_file(root, first['archive'])) as bundle, \
            ZipFile(checked_file(root, index['original_capture'])) as original:
        require(len(bundle.namelist()) == len(set(bundle.namelist())) == 97
                and len(original.namelist()) == len(set(original.namelist())) == 104,
                'Original paragraph-order archive coverage changed')
        for member in bundle.namelist():
            require(bundle.read(member) == original.read(member), 'Original paragraph-order bytes substituted')
        require(bundle.read('capture.json') == manifest.read_bytes()
                and original.read('word-output-font-facts.json') == facts_path.read_bytes(),
                'Original paragraph-order metadata changed')
        for row in index['fixtures']:
            name = row['name']; document = documents[name]
            data, spec = generated[name]; source = checked_file(root, row['source'])
            require(row['capture'] == first['capture'] and row['archive'] == first['archive']
                    and row['word'] == capture['word'] and row['capture_id'] == capture['capture_id'],
                    'Paragraph-order reference substituted')
            require(source.read_bytes() == data == bundle.read('inputs/' + name + '.docx')
                    and row['input_specification'] == spec
                    and row['source']['sha256'] == document['sha256']
                    and document['compatibility_mode'] == spec['compatibility_mode']
                    and spec['compatibility_mode'] in (14, 15)
                    and document['open_mode'] == 'normal-read-only',
                    'Original paragraph-order source/mode changed')
            for field in ('pdf', 'xps'):
                path = manifest.parent / document[field]
                require(path.read_bytes() == bundle.read(document[field])
                        and hashlib.sha256(path.read_bytes()).hexdigest() == document[field + '_sha256'],
                        'Original paragraph-order PDF/XPS changed')
            require([p['page'] for p in document['metafiles']] == [1], 'Original page coverage changed')
            for page in document['metafiles']:
                raw = (manifest.parent / page['file']).read_bytes()
                require(raw == bundle.read(page['file'])
                        and hashlib.sha256(raw).hexdigest() == page['sha256'], 'Original EMF changed')
            pdf = manifest.parent / document['pdf']; xps = manifest.parent / document['xps']
            f = fonts[name]
            require(f['word'] == capture['word'] and f['source_sha256'] == document['sha256'],
                    'Original paragraph-order font identity changed')
            with fitz.open(pdf) as book:
                for font in f['pdf_fonts']:
                    require(hashlib.sha256(book.extract_font(font['xref'])[3]).hexdigest()
                            == font['program_sha256'], 'Original PDF font program changed')
                aptos = [font for font in f['pdf_fonts'] if font['family'] == 'Aptos Display']
                require(bool(aptos) and all(font['version'] == 'Version 2.01;O365' for font in aptos),
                        'Original Aptos version changed')
            with ZipFile(xps) as package:
                for font in f['xps_font_members']:
                    require(hashlib.sha256(package.read(font['file'])).hexdigest() == font['sha256'],
                            'Original XPS font program changed')
            drawing = drawings[name]; xm = pdf_origins(xps, True)
            require(document['pages'] == drawing['page_count'] == xm['page_count'] == 1
                    and len(drawing['glyphs']) == len(xm['glyphs']) == row['glyph_count']
                    == spec['expected_non_whitespace_glyphs'], 'Original paragraph-order geometry coverage changed')
            cases.append((row, source, pdf, drawing, pdf_origins(pdf), pdf_origins(pdf, True), xm, xps))
    return cases, pending


def assert_models(case, drawing, visible, full, xps, native, screen, painted, painted_full):
    name = case['name']
    require(native['input_sha256'] == screen['input_sha256'] == case['source']['sha256'],
            'Wrong paragraph-order rendered input')
    assert_screen_outputs(drawing, native, screen, name)
    require(native['glyphs'] == screen['glyphs'] and native['page_count'] == screen['page_count'],
            'PDF export changed paragraph-order live origins')
    for label, model in (('native', native), ('screen', screen)):
        assert_model(xps, model, name + '/XPS/' + label, True)
    assert_pending_pdf_text(visible, painted, name + '/PDF-visible')
    assert_pending_pdf_text(full, painted_full, name + '/PDF-unclipped')
    if case['coordinate_gate_promotion'] == 'y-context-print':
        assert_outputs(drawing, visible, full, native, screen, painted, painted_full, name)


def check(executable, output, root=ROOT):
    require(sys.platform == 'win32', 'Paragraph-order drawing checks require Windows')
    cases, pending = verify(root)
    output.mkdir(parents=True, exist_ok=False); results = []
    renderer_hash = hashlib.sha256(executable.read_bytes()).hexdigest()
    for case, source, original, drawing, visible, full, xps_model, xps in cases:
        target = output / case['name']; target.mkdir(); fonts = target / 'fonts'
        font_facts = extract_xps_fonts(xps, fonts, output / '.font-cache')
        native = target / 'native.json'; screen = target / 'screen.json'; painted = target / 'native.pdf'
        for model, extra in ((native, [str(painted), '--unclipped-tables']), (screen, [])):
            with model.with_suffix('.log').open('x', encoding='utf-8') as log:
                subprocess.run([str(executable), str(source), str(model), 'windows', *extra,
                                '--fonts-directory', str(fonts)], check=True, stdout=log, stderr=subprocess.STDOUT)
        n, s = read_json(native), read_json(screen)
        for model in (n, s): require_original_font_programs_registered(fonts, model)
        require(native.with_suffix('.png').read_bytes() == screen.with_suffix('.png').read_bytes(),
                'PDF export changed paragraph-order live pixels')
        require(painted.read_bytes() != original.read_bytes(), 'Word PDF self-comparison')
        require(not has_visible_vector_ink(painted), 'Unexpected paragraph-order vector paint')
        physical = pdf_origins(painted); physical_full = pdf_origins(painted, True)
        assert_models(case, drawing, visible, full, xps_model, n, s, physical, physical_full)
        results.append(dict(name=case['name'], source_sha256=case['source']['sha256'], required_screen=True,
                            required_xps=True, required_pdf=case['name'] not in pending,
                            strict_pdf_text_pages_order_finite=True, original_fonts=font_facts,
                            physical_pdf_max_x=max(abs(a[2] - b[2]) for a, b in zip(visible['glyphs'], physical['glyphs']))))
        print(case['name'], 'EMF/XPS PASS;', 'PDF pending' if case['name'] in pending else 'PDF PASS', flush=True)
    (output / 'results.json').write_text(json.dumps(dict(renderer_sha256=renderer_hash,
        index_sha256=INDEX_SHA256, cases=results), indent=2) + '\n', encoding='utf-8')
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'target/windows-y-pdf-context')
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable.resolve(), args.output_directory.resolve())
    else:
        print('Verified', len(verify()[0]), 'versioned original Word paragraph-order controls')
