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
INDEX_SHA256 = '504ac78bbd25d12676d4af2c17d94650be36b2c27f3ee87e695db86c92735f75'


def has_visible_vector_ink(pdf):
    """Ignore uniform white paper fills, but retain strokes and filled guides."""
    import pymupdf
    with pymupdf.open(pdf) as document:
        for page in document:
            leading_white_fills = 0
            for drawing in page.get_drawings():
                fill = drawing.get('fill')
                stroke = drawing.get('color')
                paints_fill = fill is not None and drawing.get('fill_opacity', 0) > 0
                paints_stroke = stroke is not None and drawing.get('stroke_opacity', 0) > 0
                if not paints_fill and not paints_stroke:
                    continue
                white_paper = (drawing['type'] == 'f' and fill == (1., 1., 1.)
                               and drawing.get('fill_opacity') == 1
                               and drawing['seqno'] == leading_white_fills)
                # Leading opaque white fills cannot mark white paper, even
                # when the authored fractional-point size is smaller than the
                # raster-aligned PDF canvas. seqno counts text/images too:
                # a later white fill may erase ink and must still be rejected.
                if white_paper:
                    leading_white_fills += 1
                if not white_paper:
                    return True
    return False


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Windows font coverage/expectations changed')
    index = read_json(root / INDEX)
    require(index['schema'] == 1, 'Unknown font registry schema')
    require(len(index['fixtures']) == 1033, 'Windows font coverage removed')
    cases, names, drawings = [], set(), {}
    for case in index['fixtures']:
        require('no_visible_vector_ink' not in case
                or case['no_visible_vector_ink'] is True, 'Invalid paint expectation')
        require('solid_paragraph_background' not in case
                or case['solid_paragraph_background'] is True, 'Invalid shading expectation')
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
    require(sum(c[0]['glyph_count'] for c in cases) == 159779, 'Total font coverage changed')
    return cases


def require_original_font_programs_registered(directory, rendered):
    supplied = {hashlib.sha256(path.read_bytes()).hexdigest()
                for path in directory.iterdir()
                if path.is_file() and path.suffix.lower() in ('.ttf', '.otf', '.ttc')}
    registered = {hashlib.sha256(Path(face['path']).read_bytes()).hexdigest()
                  for face in rendered['fonts']}
    require(bool(supplied) and supplied <= registered,
            'Renderer omitted supplied original Word font programs')


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
            require_original_font_programs_registered(fonts, actual)
            compare(drawing, actual, case['name'] + '/EMF')
            compare(pdf_origins(original_pdf, True), actual, case['name'] + '/PDF-unclipped')
            visible_path = target / 'visible.json'
            subprocess.run([str(executable), str(source), str(visible_path), 'windows',
                            *options], check=True)
            visible = pdf_origins(original_pdf)
            rendered_visible = read_json(visible_path)
            require_original_font_programs_registered(fonts, rendered_visible)
            compare(visible, rendered_visible, case['name'] + '/PDF-visible')
            require(painted_path.read_bytes() != original_pdf.read_bytes(), 'Word self-comparison')
            compare(visible, pdf_origins(painted_path), case['name'] + '/painted-PDF')
            if case.get('no_visible_vector_ink'):
                require(not has_visible_vector_ink(original_pdf),
                        case['name'] + ': original Word has unexpected vector paint')
                require(not has_visible_vector_ink(painted_path),
                        case['name'] + ': native PDF contains extra border/guide paint')
            if case.get('solid_paragraph_background'):
                from paragraph_backgrounds import compare_paragraph_backgrounds
                compare_paragraph_backgrounds(source, original_pdf, painted_path, case['name'])
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
        print(f'Verified {len(verify())} Windows font cases / 159779 glyph origins')
