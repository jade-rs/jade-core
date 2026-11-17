"""Assert versioned Word table coordinates, output contexts and painted rules."""
import argparse
import hashlib
from pathlib import Path
import subprocess
import sys
from zipfile import ZipFile

from check_platform_coordinates import (
    ROOT, checked_file, compare, extract_fonts, pdf_origins, read_json, require,
)
from check_windows_font_coordinates import require_original_font_programs_registered
from compare_windows_borders import compare as compare_borders
from extract_windows_emf import extract

INDEX = Path('tests/word-oracle/windows/column-table-observations.json')
INDEX_SHA256 = 'fb434b043a55317d73c436f1dd63a11aba2049ec1f8b205a9029cc36a700db45'


def independent_outputs(drawing, word_visible, word_full, native, screen, painted_visible, painted_full, label):
    compare(drawing, native, label + '/screen-EMF-with-export')
    compare(drawing, screen, label + '/screen-EMF-without-export')
    compare(word_visible, painted_visible, label + '/painted-PDF-visible')
    compare(word_full, painted_full, label + '/painted-PDF-unclipped')


def require_nonempty_matching_rules(pages, label):
    require(bool(pages), label + ': missing border pages')
    for page in pages:
        require(page['word_rectangles'] > 0 and page['native_rectangles'] > 0,
                label + ': missing nonempty table rules')
        require(page['word_uncovered_count'] == page['native_uncovered_count'] == 0,
                label + ': displaced or missing table rules on page ' + str(page['page']))


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Column-table reference coverage changed')
    index = read_json(root / INDEX)
    require(index['schema'] == 1 and len(index['fixtures']) == 54, 'Column-table coverage removed')
    checked_file(root, index['mac_parent_manifest'])
    cases, names, contrasts = [], set(), 0
    for case in index['fixtures']:
        name = case['name']
        require(name not in names, 'Duplicate column-table case')
        names.add(name)
        require(case['coordinate_gate_promotion'] == 'column-tables', 'Column-table gate removed')
        source = checked_file(root, case['source'])
        public = checked_file(root, case['public_source'])
        require(source.read_bytes() == public.read_bytes(), 'Public column-table input changed')
        manifest = checked_file(root, case['capture'])
        capture = read_json(manifest)
        require(capture['status'] == 'complete' and capture['platform'] == 'windows', 'Incomplete Word capture')
        require(capture['word'] == case['word'] and capture['capture_id'] == case['capture_id'],
                'Original Word version or capture identity changed')
        require(case['word']['product_version'] == '16.0.19127.20936', 'Recorded Word version removed')
        document, = capture['documents']
        require(document['sha256'] == case['source']['sha256'] and document['name'] == name,
                'Wrong original table input')
        require(document['open_mode'] == 'normal-read-only', 'Word table input was repaired or converted')
        facts = case['input_specification']
        require(document['compatibility_mode'] == facts['compatibility_mode'], 'Word mode changed')
        archive = checked_file(root, case['archive'])
        with ZipFile(archive) as bundle:
            require(bundle.read('capture.json') == manifest.read_bytes(), 'Archived Word metadata changed')
            require(bundle.read('inputs/' + name + '.docx') == source.read_bytes(), 'Archived Word input changed')
        original = manifest.parent / document['pdf']
        require(hashlib.sha256(original.read_bytes()).hexdigest() == document['pdf_sha256'], 'Original Word PDF changed')
        for page in document['metafiles']:
            require(hashlib.sha256((manifest.parent / page['file']).read_bytes()).hexdigest() == page['sha256'],
                    'Original Word EMF changed')
        drawing, = extract(manifest)['fixtures']
        visible, full = pdf_origins(original), pdf_origins(original, True)
        distinct = facts['frame'] == 'unequal' and facts['alignment'] in ('center', 'right') and facts['compatibility_mode'] == 14
        require(case['word_output_contexts_differ'] is distinct, 'Output-context expectation changed')
        if distinct:
            try:
                compare(full, drawing, name + '/original-output-contrast')
            except ValueError:
                contrasts += 1
            else:
                raise ValueError(name + ': original Word output contrast was removed')
        else:
            compare(full, drawing, name + '/original-output-agreement')
        cases.append((case, source, manifest, original, drawing, visible, full))
    require(contrasts == 6, 'Independent original Word output coverage removed')
    return cases


def check(executable, output, root=ROOT):
    require(sys.platform == 'win32', 'Word table checks require Windows')
    cases = verify(root)
    output.mkdir(parents=True, exist_ok=False)
    for case, source, _, original, drawing, visible, full in cases:
        target = output / case['name']
        target.mkdir()
        fonts = target / 'fonts'
        extract_fonts(original, fonts)
        options = ['--fonts-directory', str(fonts)]
        snapshot, screen, painted = target / 'native.json', target / 'screen.json', target / 'native.pdf'
        subprocess.run([str(executable), str(source), str(snapshot), 'windows', str(painted),
                        '--unclipped-tables', *options], check=True)
        subprocess.run([str(executable), str(source), str(screen), 'windows', *options], check=True)
        native, without_export = read_json(snapshot), read_json(screen)
        for model in (native, without_export):
            require(model['input_sha256'] == case['source']['sha256'], 'Wrong table rendered input')
            require_original_font_programs_registered(fonts, model)
        independent_outputs(drawing, visible, full, native, without_export,
                            pdf_origins(painted), pdf_origins(painted, True), case['name'])
        if not case['word_output_contexts_differ']:
            compare(visible, without_export, case['name'] + '/logical-PDF-visible')
            compare(full, native, case['name'] + '/logical-PDF-unclipped')
        require(snapshot.with_suffix('.png').read_bytes() == screen.with_suffix('.png').read_bytes(),
                case['name'] + ': PDF export changed live screen paint')
        require(painted.read_bytes() != original.read_bytes(), 'Word self-comparison')
        require_nonempty_matching_rules(compare_borders(original, painted), case['name'])
        print(case['name'], case['word']['product_version'],
              'independent screen/PDF origins, nonempty vector rules, preserved screen: PASS', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'target/windows-column-tables')
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable.resolve(), args.output_directory.resolve())
    else:
        print(f'Verified {len(verify())} original column-table screen/PDF/rule references')
