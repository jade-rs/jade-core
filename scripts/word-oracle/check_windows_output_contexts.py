"""Assert Windows screen EMFs and vector PDFs against their own Word output.

The existing four-plane gate remains unchanged. These independently captured
tracking controls also retain Word's genuine screen/print differences.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
from zipfile import ZipFile

from check_platform_coordinates import (
    ROOT, checked_file, compare, extract_fonts, pdf_origins, read_json, require,
)
from check_windows_font_coordinates import require_original_font_programs_registered
from extract_windows_emf import extract

INDEX = Path('tests/word-oracle/windows/output-contexts.json')
INDEX_SHA256 = '17eed54ad40e2e08937faefcc1e221969ecc5d4ad5c1375162f6ac465db4d8af'


def original(case, directory, root=ROOT):
    archive = checked_file(root, case['archive'])
    with ZipFile(archive) as bundle:
        capture = json.loads(bundle.read('capture.json').decode('utf-8-sig'))
        require(capture['platform'] == 'windows' and capture['status'] == 'complete',
                'Wrong or incomplete Word output platform')
        require(capture['capture_id'] == case['capture_id'], 'Wrong output capture')
        require(capture['word']['product_version'] == case['word_product_version'],
                'Word output version changed')
        require(capture['word']['executable_sha256'] == case['word_executable_sha256'],
                'Word executable changed')
        document, = capture['documents']
        require(document['name'] == case['name'] and document['open_mode'] == 'normal-read-only',
                'Wrong or repaired output input')
        require(document['sha256'] == case['source']['sha256'], 'Wrong output source digest')
        members = {'capture.json', 'inputs/' + case['name'] + '.docx', document['pdf']}
        members.update(page['file'] for page in document['metafiles'])
        require(len(bundle.namelist()) == len(members) and set(bundle.namelist()) == members,
                'Unexpected output archive members')
        require(all(not Path(name).is_absolute() and ':' not in name and '\\' not in name
                    and '..' not in Path(name).parts for name in members),
                'Output archive path escapes target')
        source = checked_file(root, case['source'])
        require(bundle.read('inputs/' + case['name'] + '.docx') == source.read_bytes(),
                'Archived output input differs')
        expected = [(document['pdf'], document['pdf_sha256'])]
        expected += [(page['file'], page['sha256']) for page in document['metafiles']]
        require(all(hashlib.sha256(bundle.read(name)).hexdigest() == digest
                    for name, digest in expected), 'Original Word output changed')
        bundle.extractall(directory)
    return source, directory / document['pdf']


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Output-context coverage/expectations changed')
    index = read_json(root / INDEX)
    require(index['schema'] == 1 and len(index['fixtures']) == 18,
            'Output-context coverage removed')
    names = set()
    for case in index['fixtures']:
        require(case['name'] not in names, 'Duplicate output-context fixture')
        names.add(case['name'])
        require(case['screen_reference'] == 'EMF' and case['vector_pdf_reference'] == 'PDF',
                'Output contexts were interchanged')
        require(case['word_product_version'] == '16.0.19127.20936',
                'Established output Word version was removed')
        checked_file(root, case['source'])
        checked_file(root, case['archive'])
    return index['fixtures']


def check(executable, output, root=ROOT):
    require(sys.platform == 'win32', 'Windows output measurements require Windows')
    cases = verify(root)
    output.mkdir(parents=True, exist_ok=False)
    for case in cases:
        target = output / case['name']
        directory = target / 'originals'
        directory.mkdir(parents=True)
        source, pdf = original(case, directory, root)
        drawing, = extract(directory / 'capture.json')['fixtures']
        fonts = target / 'fonts'
        extract_fonts(pdf, fonts)
        options = ['--fonts-directory', str(fonts)]
        screen, snapshot, painted = target / 'screen.json', target / 'export.json', target / 'native.pdf'
        subprocess.run([str(executable), str(source), str(screen), 'windows', *options], check=True)
        subprocess.run([str(executable), str(source), str(snapshot), 'windows', str(painted),
                        '--unclipped-tables', *options], check=True)
        before, after = read_json(screen), read_json(snapshot)
        for model in (before, after):
            require(model['input_sha256'] == case['source']['sha256'], 'Wrong output rendered input')
            require_original_font_programs_registered(fonts, model)
            compare(drawing, model, case['name'] + '/screen-EMF')
        require(before['glyphs'] == after['glyphs'] and before['page_count'] == after['page_count'],
                'PDF export changed live logical coordinates')
        require(screen.with_suffix('.png').read_bytes() == snapshot.with_suffix('.png').read_bytes(),
                'PDF export changed live screen pixels')
        require(painted.read_bytes() != pdf.read_bytes(), 'Word PDF self-comparison')
        compare(pdf_origins(pdf), pdf_origins(painted), case['name'] + '/vector-PDF-visible')
        compare(pdf_origins(pdf, True), pdf_origins(painted, True),
                case['name'] + '/vector-PDF-unclipped')
        print(case['name'], case['word_product_version'],
              'independent screen/print origins and preserved live pixels: PASS', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path,
                        default=ROOT / 'target/windows-output-contexts')
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable.resolve(), args.output_directory.resolve())
    else:
        print(f'Verified {len(verify())} independent Windows screen/print cases')
