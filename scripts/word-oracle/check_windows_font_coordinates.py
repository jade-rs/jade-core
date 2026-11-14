"""Assert Windows Word font coordinates using fonts embedded in original Word PDFs.

The renderer receives DOCX bytes and font programs only, never reference positions.
This makes the regression independent of Office font installation on hosted runners.
"""
import argparse
import hashlib
from pathlib import Path
import subprocess
import sys

from check_platform_coordinates import (
    ROOT, checked_file, compare, extract_fonts, pdf_origins, read_json, require,
)
from extract_windows_emf import extract

INDEX = Path('tests/word-oracle/windows/font-coordinates.json')
# Changing the established coverage requires an explicit change to this assertion.
INDEX_SHA256 = '0587bbf832a6dc058426d27284b6380a6733eeb0aa8f1048ef44be61f202b7ea'


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Windows font coverage/expectations changed')
    index = read_json(root / INDEX)
    require(index['schema'] == 1, 'Unknown font registry schema')
    require(len(index['fixtures']) == 218, 'Windows font coverage removed')
    cases, names, drawings = [], set(), {}
    for case in index['fixtures']:
        name = case['name']
        require(name not in names, 'Duplicate font fixture')
        names.add(name)
        source = checked_file(root, case['source'])
        manifest = checked_file(root, case['capture'])
        reference = checked_file(root, case['drawing_reference'])
        require(reference.parent == manifest.parent, 'Wrong drawing capture')
        capture = read_json(manifest)
        require(capture['word']['product_version'] == case['word_product_version'],
                'Wrong Word version')
        if manifest not in drawings:
            drawings[manifest] = extract(manifest)
            require(read_json(reference) == drawings[manifest],
                    'Drawing origins differ from original Word EMFs')
        document, = [d for d in capture['documents'] if d['name'] == name]
        drawing, = [f for f in drawings[manifest]['fixtures'] if f['name'] == name]
        require(document['sha256'] == case['source']['sha256'], 'Wrong Word input')
        require((manifest.parent / 'inputs' / (name + '.docx')).read_bytes()
                == source.read_bytes(), 'Archived input changed')
        require(len(drawing['glyphs']) == case['glyph_count'], 'Font glyph coverage changed')
        encrypted = source.read_bytes().startswith(bytes.fromhex('D0CF11E0A1B11AE1'))
        require(encrypted == bool(case['public_test_password']), 'Wrong password policy')
        cases.append((case, source, manifest, document, drawing))
    require(sum(c[0]['glyph_count'] for c in cases) == 98822, 'Total font coverage changed')
    return cases


def check_windows(executable, output, root=ROOT):
    require(sys.platform == 'win32', 'Windows font checks require Windows')
    for case, source, manifest, document, drawing in verify(root):
        target = output / manifest.parent.name / case['name']
        target.mkdir(parents=True, exist_ok=False)
        original_pdf = manifest.parent / document['pdf']
        fonts = target / 'fonts'
        extract_fonts(original_pdf, fonts)
        options = ['--fonts-directory', str(fonts)]
        password = target / 'public-test-password.txt'
        if case['public_test_password']:
            password.write_text(case['public_test_password'], encoding='utf-8')
            options += ['--password-file', str(password)]
        try:
            actual_path, painted_path = target / 'native.json', target / 'native.pdf'
            subprocess.run([str(executable), str(source), str(actual_path), 'windows',
                            str(painted_path), '--unclipped-tables', *options], check=True)
            actual = read_json(actual_path)
            require(actual['input_sha256'] == case['source']['sha256'], 'Wrong rendered input')
            compare(drawing, actual, case['name'] + '/EMF')
            compare(pdf_origins(original_pdf, True), actual, case['name'] + '/PDF-unclipped')
            visible_path = target / 'visible.json'
            subprocess.run([str(executable), str(source), str(visible_path), 'windows',
                            *options], check=True)
            visible = pdf_origins(original_pdf)
            compare(visible, read_json(visible_path), case['name'] + '/PDF-visible')
            require(painted_path.read_bytes() != original_pdf.read_bytes(), 'Word self-comparison')
            compare(visible, pdf_origins(painted_path), case['name'] + '/painted-PDF')
            print(case['name'], case['word_product_version'], 'four coordinate planes: PASS', flush=True)
        finally:
            password.unlink(missing_ok=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path,
                        default=ROOT / 'target/windows-font-coordinates')
    args = parser.parse_args()
    if args.native_executable:
        check_windows(args.native_executable.resolve(), args.output_directory.resolve())
    else:
        print(f'Verified {len(verify())} Windows font cases / 98822 glyph origins')
