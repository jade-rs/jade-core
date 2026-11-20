"""Check original Word mixed-ascii controls without rewriting expectations."""
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
from io import BytesIO
from lxml import etree as E

INDEX = Path('tests/word-oracle/windows/mixed-ascii-observations.json')
INDEX_SHA256 = '58fb1c7842d033b46f84878ebfbac6523d74eac7f8eebf3d18a9256e5fced77c'
PRINT_NAMES_SHA256 = '7cf3afe2b421e2f5cb2d0d5c8c9df5c6d56ac2bef73508555c571f9d5275c807'


def locked_promotions(index):
    rows = index['fixtures']
    require(index['schema'] == 1 and len(rows) == len({r['name'] for r in rows}) == 16,
            'Mixed-ASCII source coverage changed')
    required = sorted(r['name'] for r in rows if r['coordinate_gate_promotion'] == 'mixed-ascii-print')
    require(len(required) == 15 and hashlib.sha256(json.dumps(
        required, separators=(',', ':')).encode()).hexdigest() == PRINT_NAMES_SHA256,
        'Required mixed-ascii PDF assertions changed')
    require(all(r['screen_gate_promotion'] == 'mixed-ascii-screen'
                and r['xps_gate_promotion'] == 'mixed-ascii-xps'
                and r['coordinate_gate_promotion'] in ('mixed-ascii-print', 'pending') for r in rows),
            'Required mixed-ascii screen/XPS assertion removed')
    require(sum(r['glyph_count'] for r in rows) == 3072, 'Mixed-ASCII glyph coverage changed')
    return frozenset(r['name'] for r in rows if r['coordinate_gate_promotion'] == 'pending')


def validated_source(root, row):
    spec = row['input_specification']
    data = checked_file(root, row['source']).read_bytes()
    require(spec['font_family'] == 'Aptos Display' and spec['font_style'] in
            ('regular', 'bold', 'italic', 'bolditalic') and spec['font_size_pt'] in (14, 15.5)
            and spec['paragraphs'] == 4 and spec['length'] == 48
            and spec['expected_non_whitespace_glyphs'] == 192
            and spec['changed_parts'] == ['word/document.xml']
            and spec['pattern'] in ('AVATARWAVE', '%m@WAT')
            and spec['text'] == (spec['pattern'] * 48)[:48], 'Mixed ASCII input specification changed')
    with ZipFile(checked_file(root, spec['derived_parent'])) as parent, ZipFile(BytesIO(data)) as changed:
        require(parent.namelist() == changed.namelist(), 'Mixed ASCII package members changed')
        for member in parent.namelist():
            if member != 'word/document.xml':
                require(parent.read(member) == changed.read(member), 'Unrelated DOCX part changed')
        before, after = [E.fromstring(z.read('word/document.xml')) for z in (parent, changed)]
        tag = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}t'
        old, new = list(before.iter(tag)), list(after.iter(tag))
        require(len(old) == len(new) == 4, 'Mixed ASCII paragraph coverage changed')
        for original, actual in zip(old, new):
            require(original.text == 'Z' * 48 and actual.text == spec['text'], 'Mixed ASCII text changed')
            actual.text = original.text
        require(E.tostring(before) == E.tostring(after), 'Mixed ASCII source formatting changed')
    return data, spec


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Original mixed-ascii registry changed')
    index = read_json(root / INDEX)
    pending = locked_promotions(index)
    parent = read_json(checked_file(root, index['parent_index']))
    mac_path = checked_file(root, index['mac_parent_manifest'])
    mac = read_json(mac_path)
    require(parent['mac_parent_reference'] == index['mac_parent_reference'] == mac['reference']
            and mac['reference']['platform'] == 'mac'
            and hashlib.sha256(mac_path.with_name('source.docx').read_bytes()).hexdigest()
            == mac['sourceSha256'], 'Original Mac parent changed')
    parents = {r['name']: r for r in parent['fixtures']}
    for row in index['fixtures']:
        spec = row['input_specification']
        original = parents.get(spec['parent_name'])
        require(original is not None and original['source'] == spec['derived_parent'],
                'Original mixed ASCII parent source changed')
        prior = original['input_specification']
        require(all(prior[key] == spec[key] for key in ('font_family', 'font_size_pt', 'compatibility_mode'))
                and prior['style'] == spec['font_style'] and prior['character'] == 'Z'
                and prior['characters_per_paragraph'] == 48 and prior['paragraphs'] == 4
                and prior['retained_kerning_thresholds_halfpoints'] == [200, 200, 2, 2],
                'Mixed ASCII parent font, size or shaping mode changed')
    generated = {r['name']: validated_source(root, r) for r in index['fixtures']}
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
            'Original mixed-ascii Word identity changed')
    documents = {r['name']: r for r in capture['documents']}
    drawings = {r['name']: r for r in extract(manifest)['fixtures']}
    facts_path = checked_file(root, index['font_facts'])
    fonts = {r['name']: r for r in read_json(facts_path)}
    require(len(documents) == len(capture['documents']) == 16
            and set(generated) == set(documents) == set(fonts) == set(drawings),
            'Mixed-ASCII capture coverage changed')
    cases = []
    with ZipFile(checked_file(root, first['archive'])) as bundle, \
            ZipFile(checked_file(root, index['original_capture'])) as original:
        require(len(bundle.namelist()) == len(set(bundle.namelist())) == 65
                and len(original.namelist()) == len(set(original.namelist())) == 72,
                'Original mixed-ascii archive coverage changed')
        for member in bundle.namelist():
            require(bundle.read(member) == original.read(member), 'Original mixed-ascii bytes substituted')
        require(bundle.read('capture.json') == manifest.read_bytes()
                and original.read('word-output-font-facts.json') == facts_path.read_bytes(),
                'Original mixed-ascii metadata changed')
        for row in index['fixtures']:
            name = row['name']; document = documents[name]
            data, spec = generated[name]; source = checked_file(root, row['source'])
            require(row['capture'] == first['capture'] and row['archive'] == first['archive']
                    and row['word'] == capture['word'] and row['capture_id'] == capture['capture_id'],
                    'Mixed-ASCII reference substituted')
            require(source.read_bytes() == data == bundle.read('inputs/' + name + '.docx')
                    and row['input_specification'] == spec
                    and row['source']['sha256'] == document['sha256']
                    and document['compatibility_mode'] == spec['compatibility_mode']
                    and spec['compatibility_mode'] == 14
                    and document['open_mode'] == 'normal-read-only',
                    'Original mixed-ascii source/mode changed')
            for field in ('pdf', 'xps'):
                path = manifest.parent / document[field]
                require(path.read_bytes() == bundle.read(document[field])
                        and hashlib.sha256(path.read_bytes()).hexdigest() == document[field + '_sha256'],
                        'Original mixed-ascii PDF/XPS changed')
            require([p['page'] for p in document['metafiles']] == [1], 'Original page coverage changed')
            for page in document['metafiles']:
                raw = (manifest.parent / page['file']).read_bytes()
                require(raw == bundle.read(page['file'])
                        and hashlib.sha256(raw).hexdigest() == page['sha256'], 'Original EMF changed')
            pdf = manifest.parent / document['pdf']; xps = manifest.parent / document['xps']
            f = fonts[name]
            require(f['word'] == capture['word'] and f['source_sha256'] == document['sha256'],
                    'Original mixed-ascii font identity changed')
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
                    == spec['expected_non_whitespace_glyphs'], 'Original mixed-ascii geometry coverage changed')
            cases.append((row, source, pdf, drawing, pdf_origins(pdf), pdf_origins(pdf, True), xm, xps))
    return cases, pending


def assert_models(case, drawing, visible, full, xps, native, screen, painted, painted_full):
    name = case['name']
    require(native['input_sha256'] == screen['input_sha256'] == case['source']['sha256'],
            'Wrong mixed-ascii rendered input')
    assert_screen_outputs(drawing, native, screen, name)
    require(native['glyphs'] == screen['glyphs'] and native['page_count'] == screen['page_count'],
            'PDF export changed mixed-ascii live origins')
    for label, model in (('native', native), ('screen', screen)):
        assert_model(xps, model, name + '/XPS/' + label, True)
    assert_pending_pdf_text(visible, painted, name + '/PDF-visible')
    assert_pending_pdf_text(full, painted_full, name + '/PDF-unclipped')
    if case['coordinate_gate_promotion'] == 'mixed-ascii-print':
        assert_outputs(drawing, visible, full, native, screen, painted, painted_full, name)


def check(executable, output, root=ROOT):
    require(sys.platform == 'win32', 'Mixed-ASCII drawing checks require Windows')
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
                'PDF export changed mixed-ascii live pixels')
        require(painted.read_bytes() != original.read_bytes(), 'Word PDF self-comparison')
        require(not has_visible_vector_ink(painted), 'Unexpected mixed-ascii vector paint')
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
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'target/windows-mixed-ascii')
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable.resolve(), args.output_directory.resolve())
    else:
        print('Verified', len(verify()[0]), 'versioned original Word mixed-ascii controls')
