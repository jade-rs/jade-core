"""Assert Word print/PDF coordinates where Word's screen EMF contradicts its PDF.

These cases do not qualify as four-plane font matches. The unchanged original
EMFs remain archived, and verification requires their recorded disagreement to
still exist. The renderer receives DOCX and original PDF font programs only.
"""
import argparse
import hashlib
from pathlib import Path
import subprocess
import sys

from check_platform_coordinates import (
    ROOT, checked_file, compare, extract_fonts, pdf_origins, read_json, require,
)
from check_windows_note_rules import compare_rules, pdf_rules
from extract_windows_emf import extract

INDEX = Path('tests/word-oracle/windows/print-coordinates.json')
INDEX_SHA256 = 'f39bd6d4c4a1d97ebf278c965e40185476318e4a54c78907e2eb288908156a1e'


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Windows print coverage/expectations changed')
    index = read_json(root / INDEX)
    require(index['schema'] == 1 and len(index['fixtures']) == 2,
            'Windows print coverage removed')
    cases, names = [], set()
    for case in index['fixtures']:
        require(case['name'] not in names, 'Duplicate print fixture')
        names.add(case['name'])
        source = checked_file(root, case['source'])
        manifest = checked_file(root, case['capture'])
        capture = read_json(manifest)
        require(capture['word']['product_version'] == case['word_product_version'],
                'Wrong print Word version')
        document, = [d for d in capture['documents'] if d['name'] == case['name']]
        require(document['sha256'] == case['source']['sha256'], 'Wrong print Word input')
        require((manifest.parent / 'inputs' / source.name).read_bytes() == source.read_bytes(),
                'Archived print input changed')
        pdf = checked_file(root, dict(
            file=(manifest.parent / document['pdf']).relative_to(root).as_posix(),
            sha256=document['pdf_sha256']))
        drawing, = [d for d in extract(manifest)['fixtures'] if d['name'] == case['name']]
        try:
            compare(pdf_origins(pdf, True), drawing, case['name'] + '/Word-PDF-vs-EMF')
        except ValueError as error:
            require(case['emf_disagreement'] in str(error),
                    'Original Word PDF/EMF disagreement changed')
        else:
            raise ValueError('Word outputs now agree; require all four coordinate planes')
        require(len(pdf_rules(pdf)) == case['rule_count'], 'Original print rule count changed')
        cases.append((case, source, pdf))
    return cases


def check(executable, output, root=ROOT):
    require(sys.platform == 'win32', 'Windows print checks require Windows')
    for case, source, original in verify(root):
        target = output / case['name']
        target.mkdir(parents=True, exist_ok=False)
        fonts = target / 'fonts'
        extract_fonts(original, fonts)
        actual, painted = target / 'native.json', target / 'native.pdf'
        options = ['--fonts-directory', str(fonts)]
        subprocess.run([str(executable), str(source), str(actual), 'windows',
                        str(painted), '--unclipped-tables', *options], check=True)
        snapshot = read_json(actual)
        require(snapshot['input_sha256'] == case['source']['sha256'], 'Wrong print rendered input')
        compare(pdf_origins(original, True), snapshot, case['name'] + '/PDF-unclipped')
        visible = target / 'visible.json'
        subprocess.run([str(executable), str(source), str(visible), 'windows', *options], check=True)
        require(read_json(visible)['input_sha256'] == case['source']['sha256'],
                'Wrong visible print input')
        expected = pdf_origins(original)
        compare(expected, read_json(visible), case['name'] + '/PDF-visible')
        require(painted.read_bytes() != original.read_bytes(), 'Word print self-comparison')
        compare(expected, pdf_origins(painted), case['name'] + '/painted-PDF')
        compare_rules(pdf_rules(original), pdf_rules(painted), case['name'] + '/PDF-rules',
                      allow_empty=case['rule_count'] == 0)
        print(case['name'], case['word_product_version'],
              'print/PDF coordinates and rules: PASS; original EMF disagreement retained', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path,
                        default=ROOT / 'target/windows-print-coordinates')
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable.resolve(), args.output_directory.resolve())
    else:
        print(f'Verified {len(verify())} original Windows Word print cases')
