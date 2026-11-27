"""Require all original nominal screen origins and the validated PDF subset."""
import argparse
import hashlib
from pathlib import Path
import subprocess
import sys
from zipfile import ZipFile
from check_platform_coordinates import ROOT, checked_file, compare, extract_fonts, pdf_origins, read_json, require, assert_pending_pdf_text
from check_windows_font_coordinates import require_original_font_programs_registered, has_visible_vector_ink
from check_windows_aptos_sizes import assert_screen_outputs, assert_outputs
from extract_windows_emf import extract
import generate_windows_aptos_nominal_probes as producer
import generate_windows_printer_toggle_probes as printer_producer

INDEX = Path('tests/word-oracle/windows/aptos-nominal-observations.json')
INDEX_SHA256 = 'fb9a1cf4a78f77f6d9d667242db4585c9eb4b3b447c36a448a29da11124874be'
# Original nominal cases retain eight failing print references; the settings
# controls retain twelve corresponding failures. All40 screen cases and the
# other20 logical and actual vector PDF comparisons remain required.
PENDING_PDF = frozenset(
    f'word-windows-aptos-nominal-{style}-s{size}-{char}48-m{mode}'
    for style, size, chars in [('bolditalic', 15, ['X']), ('regular', 16, ['X', 'one', 'hyphen'])]
    for char in chars for mode in (14, 15))

PRINTER_INDEX = Path('tests/word-oracle/windows/printer-metric-observations.json')
PRINTER_INDEX_SHA256 = '0505b9065941ffb3d76c85d1bc4043cbc803246da4eb5418fc1637d6b5a11fb9'
PENDING_PDF |= frozenset(
    f'word-windows-printer-metrics-{style}-s{size}-{char}-m{mode}-p{enabled}'
    for style, size, chars in [('bolditalic', 15, ['X']), ('regular', 16, ['one', 'hyphen'])]
    for char in chars for mode in (14, 15) for enabled in (0, 1))


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Nominal reference coverage changed')
    index = read_json(root / INDEX)
    require(index['schema'] == 1 and len(index['fixtures']) == 24, 'Nominal coverage removed')
    manifest = read_json(checked_file(root, index['mac_parent_manifest']))
    require(manifest['reference'] == index['mac_parent_reference']
            and manifest['reference']['platform'] == 'mac'
            and manifest['sourceSha256'] == producer.parentgen.PARENT_SHA256, 'Original Mac reference changed')
    require(hashlib.sha256((root / PRINTER_INDEX).read_bytes()).hexdigest() == PRINTER_INDEX_SHA256,
            'Printer-metric reference coverage changed')
    printer = read_json(root / PRINTER_INDEX)
    require(printer['schema'] == 1 and len(printer['fixtures']) == 16
            and printer['mac_parent_manifest'] == index['mac_parent_manifest']
            and printer['mac_parent_reference'] == index['mac_parent_reference'], 'Printer control coverage changed')
    index = dict(index, fixtures=index['fixtures'] + printer['fixtures'])
    generated = {name: (data, facts) for name, data, facts in producer.generate(root)}
    generated.update({name: (data, facts) for name, data, facts in printer_producer.generate(root)})
    require(len(generated) == 40 and set(generated) == {r['name'] for r in index['fixtures']},
            'Nominal source coverage changed')
    cases = []
    for case in index['fixtures']:
        name = case['name']; source = checked_file(root, case['source'])
        require(source.read_bytes() == generated[name][0]
                and case['input_specification'] == generated[name][1], 'Nominal source/specification changed')
        require(case['screen_gate_promotion'] == 'aptos-nominal-screen', 'Required nominal screen assertion removed')
        require(case['coordinate_gate_promotion'] == ('pending' if name in PENDING_PDF else 'aptos-nominal'),
                'Required/pending nominal print assertions changed')
        path = checked_file(root, case['capture']); capture = read_json(path)
        require(capture['status'] == 'complete' and capture['platform'] == 'windows'
                and capture['word'] == case['word'], 'Original nominal Word provenance changed')
        document, = [d for d in capture['documents'] if d['name'] == name]
        require(document['sha256'] == case['source']['sha256'] and document['open_mode'] == 'normal-read-only'
                and document['compatibility_mode'] == case['input_specification']['compatibility_mode'],
                'Original nominal input/mode changed')
        with ZipFile(checked_file(root, case['archive'])) as archive:
            require(archive.read('capture.json') == path.read_bytes()
                    and archive.read('inputs/' + name + '.docx') == source.read_bytes(), 'Original nominal archive changed')
            for filename, digest in [(document['pdf'], document['pdf_sha256'])] + [
                    (p['file'], p['sha256']) for p in document['metafiles']]:
                data = (path.parent / filename).read_bytes()
                require(archive.read(filename) == data and hashlib.sha256(data).hexdigest() == digest,
                        'Original nominal Word output changed')
        drawing, = [r for r in extract(path)['fixtures'] if r['name'] == name]
        require(drawing['page_count'] == document['pages'] == 1 and len(drawing['glyphs']) == 192,
                'Nominal page/glyph coverage removed')
        original = path.parent / document['pdf']
        cases.append((case, source, original, drawing, pdf_origins(original), pdf_origins(original, True)))
    require(len(cases) == 40 and len({c[0]['name'] for c in cases}) == 40
            and len([c for c in cases if c[0]['name'] not in PENDING_PDF]) == 20, 'Nominal coverage removed')
    return cases


def render_cases(executable, output, cases, pending_pdf):
    require(sys.platform == 'win32', 'Word coordinate checks require Windows')
    output.mkdir(parents=True, exist_ok=False)
    for case, source, original, drawing, visible, full in cases:
        target = output / case['name']; target.mkdir(); fonts = target / 'fonts'; extract_fonts(original, fonts)
        actual, screen, painted = target / 'native.json', target / 'screen.json', target / 'native.pdf'
        options = ['--fonts-directory', str(fonts)]
        subprocess.run([str(executable), str(source), str(actual), 'windows', str(painted), '--unclipped-tables', *options], check=True)
        subprocess.run([str(executable), str(source), str(screen), 'windows', *options], check=True)
        native, live = read_json(actual), read_json(screen)
        for model in (native, live):
            require(model['input_sha256'] == case['source']['sha256'], 'Wrong nominal rendered input')
            require_original_font_programs_registered(fonts, model)
        assert_screen_outputs(drawing, native, live, case['name'])
        require(native['glyphs'] == live['glyphs'] and native['page_count'] == live['page_count']
                and actual.with_suffix('.png').read_bytes() == screen.with_suffix('.png').read_bytes(),
                'PDF export changed nominal live origins/pixels')
        require(painted.read_bytes() != original.read_bytes(), 'Word PDF self-comparison')
        require(not has_visible_vector_ink(painted), 'Unexpected nominal vector paint')
        painted_visible, painted_full = pdf_origins(painted), pdf_origins(painted, True)
        assert_pending_pdf_text(visible, painted_visible, case['name'] + '/PDF-visible')
        assert_pending_pdf_text(full, painted_full, case['name'] + '/PDF-unclipped')
        if case['name'] not in pending_pdf:
            assert_outputs(drawing, visible, full, native, live,
                           painted_visible, painted_full, case['name'])
        print(case['name'], 'screen: PASS;' if case['name'] in pending_pdf else 'screen + PDF: PASS;',
              'PDF pending' if case['name'] in pending_pdf else '', flush=True)
    return len(cases)


def check(executable, output, root=ROOT):
    return render_cases(executable, output, verify(root), PENDING_PDF)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'target/windows-aptos-nominal')
    args = parser.parse_args()
    if args.native_executable:
        print('Checked', check(args.native_executable.resolve(), args.output_directory.resolve()),
              'nominal/printer screen cases /20 required PDF cases;20 PDF differences retained')
    else:
        print('Verified', len(verify()), 'original nominal controls')
