"""Assert shared DOCX identity and independent Mac/Windows Word expectations.

Mac rendering is checked by check_mac_word_coordinates.mjs on macOS. This
registry also runs the Windows native renderer against every Windows capture,
including raw EMF, PDF text and independently painted native PDF origins.
"""
import argparse
import gzip
import hashlib
import json
import math
from pathlib import Path
import subprocess
import sys

from extract_windows_emf import extract

ROOT = Path(__file__).resolve().parents[2]
INDEX = Path('tests/word-oracle/platform-coordinates.json')
REQUIRED_CAPTURES = {
    'word-mac-probe-floating-horizontal-15': '20261114T015622405Z-203d1791',
    'probe-hyphenation-split-word-sweep-off': '20261114T020157033Z-88c4d52e',
    'probe-contextual-paragraph-spacing-mode14': '20261114T023153447Z-6c24de77',
    'probe-contextual-paragraph-spacing-mode15': '20261114T023156058Z-d477ae1f',
    'probe-paragraph-border-wrap': '20261114T023201516Z-623785ec',
    'endnote-mac': '20261114T004143028Z-b4cb9887',
    'footnote-long-continuation-mac': '20261114T004143028Z-b4cb9887',
    'footnote-mac': '20261114T004143028Z-b4cb9887',
    'footnote-table-each-page-mac': '20261114T004143028Z-b4cb9887',
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read_json(path):
    return json.loads(path.read_text(encoding='utf-8-sig'))


def repo_file(root, relative):
    require(isinstance(relative, str) and '\\' not in relative and ':' not in relative
            and not Path(relative).is_absolute(),
            'Expected repository-relative POSIX path')
    path = (root / relative).resolve()
    require(path.is_relative_to(root.resolve()), 'Path escapes repository')
    require(path.is_file(), f'Missing evidence: {relative}')
    return path


def checked_file(root, record):
    path = repo_file(root, record['file'])
    require(hashlib.sha256(path.read_bytes()).hexdigest() == record['sha256'],
            f'Evidence changed: {record["file"]}')
    return path


def ordered(glyphs):
    result = sorted((g for g in glyphs if not g[1].isspace()), key=lambda g: (g[0], g[3]))
    start = 0
    while start < len(result):
        end = start + 1
        while (end < len(result) and result[end][0] == result[start][0]
               and abs(result[end][3] - result[start][3]) <= .25):
            end += 1
        result[start:end] = sorted(result[start:end], key=lambda g: g[2])
        start = end
    return result


def compare(expected, actual, label):
    require(actual['page_count'] == expected['page_count'], f'{label}: page count')
    expected_glyphs, actual_glyphs = ordered(expected['glyphs']), ordered(actual['glyphs'])
    require(bool(expected_glyphs), f'{label}: empty reference')
    require(len(expected_glyphs) == len(actual_glyphs), f'{label}: missing/extra glyphs')
    for i, (word, jade) in enumerate(zip(expected_glyphs, actual_glyphs)):
        require(word[:2] == jade[:2], f'{label}/{i}: text, order or page changed')
        for axis in (2, 3):
            require(math.isfinite(jade[axis]) and abs(word[axis] - jade[axis]) <= .375,
                    f'{label}/{i}: axis {axis}: Word {word}, Jade {jade}')
    return len(expected_glyphs)


def pdf_origins(path, unclipped=False):
    import pymupdf
    flags = pymupdf.TEXTFLAGS_RAWDICT
    if unclipped:
        flags &= ~pymupdf.TEXT_CLIP
    with pymupdf.open(path) as document:
        glyphs = []
        for number, page in enumerate(document, 1):
            for block in page.get_text('rawdict', flags=flags)['blocks']:
                for line in block.get('lines', []):
                    for span in line['spans']:
                        glyphs.extend([number, char['c'], *char['origin']]
                                      for char in span['chars'] if not char['c'].isspace())
        return dict(page_count=len(document), glyphs=glyphs)


def extract_fonts(pdf, directory):
    """Expose original font programs, never Word's expected text positions."""
    import pymupdf
    directory.mkdir(parents=True, exist_ok=False)
    with pymupdf.open(pdf) as document:
        xrefs = {font[0] for page in document for font in page.get_fonts(full=True)}
        count = 0
        for xref in sorted(xrefs):
            _, extension, _, data = document.extract_font(xref)
            if data and extension in ('ttf', 'otf', 'ttc'):
                (directory / f'{xref}.{extension}').write_bytes(data)
                count += 1
    require(count > 0, 'Original Word PDF contains no usable embedded fonts')


def verify(root=ROOT):
    index = read_json(root / INDEX)
    require(index['schema'] == 1 and bool(index['fixtures']), 'Empty platform registry')
    names = set()
    cases = []
    for case in index['fixtures']:
        name = case['mac_case']
        require(name not in names, 'Duplicate shared fixture')
        names.add(name)
        directory = f'tests/word-oracle/mac-elsa-synthetic/{name}'
        source = checked_file(root, case['source'])
        require(case['source']['file'] == f'{directory}/source.docx', 'Wrong Mac source')
        mac_file = checked_file(root, case['mac_manifest'])
        require(case['mac_manifest']['file'] == f'{directory}/manifest.json', 'Wrong Mac manifest')
        mac = read_json(mac_file)
        require(mac['reference']['platform'] == 'mac', 'Mac reference platform changed')
        # Some imported Mac captures predate version recording. Preserve that
        # provenance gap instead of inventing a build or changing their goldens.
        require(mac['reference']['application'] == 'Microsoft Word', 'Wrong Mac application')
        require(mac['sourceSha256'] == case['source']['sha256'], 'Mac source identity changed')
        coordinates = repo_file(root, f'{directory}/pages.json.gz').read_bytes()
        require(hashlib.sha256(coordinates).hexdigest() == mac['coordinatesSha256'],
                'Mac coordinates changed')
        require(len(json.loads(gzip.decompress(coordinates))) == mac['pages'], 'Mac page count')
        require(bool(case['windows']), 'Missing Windows version expectations')
        captures = set()
        for windows in case['windows']:
            manifest = checked_file(root, windows['capture'])
            capture = read_json(manifest)
            key = (capture['word']['product_version'], capture['capture_id'])
            require(key not in captures, 'Duplicate Word version/capture')
            captures.add(key)
            require(capture['platform'] == 'windows' and capture['status'] == 'complete',
                    'Incomplete or mislabelled Windows capture')
            require(key[0] == windows['word_product_version'], 'Windows Word version changed')
            documents = [d for d in capture['documents'] if d['name'] == windows['fixture']]
            require(len(documents) == 1, 'Missing or duplicate Windows input')
            document = documents[0]
            require(document['sha256'] == case['source']['sha256'], 'Windows source identity changed')
            archived = manifest.parent / 'inputs' / (document['name'] + '.docx')
            require(archived.read_bytes() == source.read_bytes(), 'Windows input differs from Mac original')
            require(hashlib.sha256((manifest.parent / document['pdf']).read_bytes()).hexdigest()
                    == document['pdf_sha256'], 'Original Windows Word PDF changed')
            reference_file = checked_file(root, windows['drawing_reference'])
            require(reference_file.parent == manifest.parent, 'Wrong drawing capture')
            reference = read_json(reference_file)
            require(reference == extract(manifest), 'Drawing origins differ from original Word EMFs')
            require(reference['platform'] == 'windows' and reference['tolerance_pt'] == .375,
                    'Windows platform/tolerance changed')
            fixture = next(f for f in reference['fixtures'] if f['name'] == document['name'])
            require(fixture['docx_sha256'] == case['source']['sha256'], 'Wrong drawing input')
            font_source = windows.get('font_source', 'installed')
            require(font_source in ('installed', 'original-word-pdf'), 'Unknown font source')
            cases.append((name, source, manifest, document, fixture, font_source))
        if name in REQUIRED_CAPTURES:
            require(('16.0.19127.20936', REQUIRED_CAPTURES[name]) in captures,
                    'Previously asserted Word version/capture was removed')
    require(REQUIRED_CAPTURES.keys() <= names, 'Shared fixture coverage removed')
    return cases


def check_windows(executable, output, root=ROOT):
    require(sys.platform == 'win32', 'Windows font measurements require Windows')
    cases = verify(root)
    output.mkdir(parents=True, exist_ok=True)
    checked = 0
    for name, source, manifest, document, drawing, font_source in cases:
        target = output / name / manifest.parent.name
        target.mkdir(parents=True, exist_ok=True)
        actual_path, painted_path = target / 'native.json', target / 'native.pdf'
        original_pdf = manifest.parent / document['pdf']
        options = []
        if font_source == 'original-word-pdf':
            extract_fonts(original_pdf, target / 'fonts')
            options = ['--fonts-directory', str(target / 'fonts')]
        subprocess.run([str(executable), str(source), str(actual_path), 'windows',
                        str(painted_path), '--unclipped-tables', *options], check=True)
        actual = read_json(actual_path)
        checked += compare(drawing, actual, name + '/EMF')
        compare(pdf_origins(original_pdf, True), actual, name + '/PDF-unclipped')
        visible_path = target / 'visible.json'
        subprocess.run([str(executable), str(source), str(visible_path), 'windows', *options], check=True)
        visible = pdf_origins(original_pdf)
        compare(visible, read_json(visible_path), name + '/PDF-visible')
        require(painted_path.read_bytes() != original_pdf.read_bytes(), 'Word self-comparison')
        compare(visible, pdf_origins(painted_path), name + '/painted-PDF')
        print(f'{name}: {document["pages"]} pages, {len(drawing["glyphs"])} glyphs, '
              f'Windows Word {read_json(manifest)["word"]["product_version"]}: PASS')
    return checked


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'target/platform-word-coordinates')
    args = parser.parse_args()
    if args.native_executable:
        print(f'Checked {check_windows(args.native_executable.resolve(), args.output_directory.resolve())} drawing origins')
    else:
        print(f'Verified {len(verify())} shared DOCX/platform-version reference pairs')
